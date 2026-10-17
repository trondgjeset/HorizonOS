#[cfg(not(test))]
use core::alloc::GlobalAlloc;
use core::alloc::Layout;
#[cfg(not(test))]
use core::cell::UnsafeCell;
use core::ptr::null_mut;
#[cfg(not(test))]
use core::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
/// A free region linked into the heap's address-ordered free list.
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

#[repr(C)]
/// Metadata stored immediately before each returned allocation.
struct AllocationHeader {
    block_start_offset: usize,
    size: usize,
}

/// Mutable allocator state protected by the global heap lock.
struct HeapState {
    head: *mut FreeBlock,
    initialized: bool,
}

impl HeapState {
    const fn empty() -> Self {
        Self {
            head: null_mut(),
            initialized: false,
        }
    }

    /// Initializes the free list over the supplied mapped heap region.
    ///
    /// # Safety
    /// `start..start + size` must be writable, exclusively owned heap memory
    /// that remains mapped for the allocator's lifetime.
    unsafe fn initialize(&mut self, start: usize, size: usize) -> Result<(), &'static str> {
        if self.initialized {
            return Err("kernel heap is already initialized");
        }
        let alignment = core::mem::align_of::<FreeBlock>();
        let aligned_start = align_up(start, alignment).ok_or("kernel heap address overflow")?;
        let skipped = aligned_start
            .checked_sub(start)
            .ok_or("kernel heap start alignment underflow")?;
        let aligned_size = size
            .checked_sub(skipped)
            .ok_or("kernel heap is smaller than its alignment padding")?
            & !(alignment - 1);
        if aligned_size < core::mem::size_of::<FreeBlock>() {
            return Err("kernel heap is too small for a free block");
        }

        let block = aligned_start as *mut FreeBlock;
        block.write(FreeBlock {
            size: aligned_size,
            next: null_mut(),
        });
        self.head = block;
        self.initialized = true;
        Ok(())
    }

    /// Finds a suitably aligned allocation using a first-fit free-list scan.
    ///
    /// Returns null when the heap is not initialized, arithmetic overflows, or
    /// no block can satisfy `layout`.
    ///
    /// # Safety
    /// The heap state must be exclusively borrowed and its region must remain
    /// writable and mapped.
    unsafe fn allocate(&mut self, layout: Layout) -> *mut u8 {
        if !self.initialized {
            return null_mut();
        }

        let alignment = layout
            .align()
            .max(core::mem::align_of::<AllocationHeader>());
        let size = layout.size().max(1);
        let mut previous: *mut FreeBlock = null_mut();
        let mut current = self.head;

        while !current.is_null() {
            let block_start = current as usize;
            let user_start = match block_start
                .checked_add(core::mem::size_of::<FreeBlock>())
                .and_then(|address| address.checked_add(core::mem::size_of::<AllocationHeader>()))
                .and_then(|address| align_up(address, alignment))
            {
                Some(address) => address,
                None => return null_mut(),
            };
            let block_end = match user_start
                .checked_add(size)
                .and_then(|address| align_up(address, core::mem::align_of::<FreeBlock>()))
            {
                Some(address) => address,
                None => return null_mut(),
            };
            let allocation_size = block_end.saturating_sub(block_start);
            let block_size = (*current).size;
            if allocation_size <= block_size {
                let next = (*current).next;
                let remainder = block_size - allocation_size;
                let allocated_size = if remainder >= core::mem::size_of::<FreeBlock>() {
                    let remainder_block = block_end as *mut FreeBlock;
                    remainder_block.write(FreeBlock {
                        size: remainder,
                        next,
                    });
                    if previous.is_null() {
                        self.head = remainder_block;
                    } else {
                        (*previous).next = remainder_block;
                    }
                    allocation_size
                } else {
                    if previous.is_null() {
                        self.head = next;
                    } else {
                        (*previous).next = next;
                    }
                    block_size
                };

                let result = user_start as *mut u8;
                let header = result
                    .sub(core::mem::size_of::<AllocationHeader>())
                    .cast::<AllocationHeader>();
                header.write(AllocationHeader {
                    block_start_offset: (header as usize).saturating_sub(block_start),
                    size: allocated_size,
                });
                return result;
            }

            previous = current;
            current = (*current).next;
        }

        null_mut()
    }

    /// Returns a pointer previously produced by `allocate` to the free list.
    ///
    /// # Safety
    /// `pointer` must be a live allocation from this heap and must not already
    /// have been deallocated.
    unsafe fn deallocate(&mut self, pointer: *mut u8) {
        if pointer.is_null() || !self.initialized {
            return;
        }
        let header = pointer
            .sub(core::mem::size_of::<AllocationHeader>())
            .cast::<AllocationHeader>();
        let block_start = (header as usize).saturating_sub((*header).block_start_offset);
        let block_size = (*header).size;
        if block_size < core::mem::size_of::<FreeBlock>() {
            return;
        }

        let mut previous: *mut FreeBlock = null_mut();
        let mut next = self.head;
        while !next.is_null() && (next as usize) < block_start {
            previous = next;
            next = (*next).next;
        }

        let block = block_start as *mut FreeBlock;
        block.write(FreeBlock {
            size: block_size,
            next,
        });
        if previous.is_null() {
            self.head = block;
        } else {
            (*previous).next = block;
        }

        // Merge adjacent regions so later large allocations can reuse them.
        // TODO: Optimization Opportunity: track a rover or use size classes to
        // avoid repeated full-list insertion scans as fragmentation increases.
        if !next.is_null() && block_start.saturating_add((*block).size) == next as usize {
            (*block).size = (*block).size.saturating_add((*next).size);
            (*block).next = (*next).next;
        }
        if !previous.is_null()
            && (previous as usize).saturating_add((*previous).size) == block as usize
        {
            (*previous).size = (*previous).size.saturating_add((*block).size);
            (*previous).next = (*block).next;
        }
    }
}

#[cfg(not(test))]
/// Synchronizes allocator access and owns the mutable heap state.
struct LockedHeap {
    locked: AtomicBool,
    state: UnsafeCell<HeapState>,
}

#[cfg(not(test))]
// SAFETY: every access to the non-Sync HeapState behind this cell is serialized
// by the atomic lock; the allocator's region is initialized once before use.
unsafe impl Sync for LockedHeap {}

#[cfg(not(test))]
impl LockedHeap {
    const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
            state: UnsafeCell::new(HeapState::empty()),
        }
    }

    fn lock(&self) -> HeapGuard<'_> {
        let interrupt_state = disable_interrupts();
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        HeapGuard {
            heap: self,
            interrupt_state,
        }
    }

    unsafe fn initialize(&self, start: usize, size: usize) -> Result<(), &'static str> {
        let _guard = self.lock();
        (*self.state.get()).initialize(start, size)
    }
}

#[cfg(not(test))]
/// Releases the heap lock and restores the caller's interrupt state on drop.
struct HeapGuard<'a> {
    heap: &'a LockedHeap,
    interrupt_state: usize,
}

#[cfg(not(test))]
impl Drop for HeapGuard<'_> {
    fn drop(&mut self) {
        self.heap.locked.store(false, Ordering::Release);
        restore_interrupts(self.interrupt_state);
    }
}

#[cfg(not(test))]
// SAFETY: the guard serializes allocator state and the caller is responsible for
// honoring GlobalAlloc's layout and allocation-lifetime requirements.
unsafe impl GlobalAlloc for LockedHeap {
    // SAFETY: the lock serializes access to the heap; callers uphold GlobalAlloc's
    // pointer and layout contract, and allocations remain mapped for their lifetime.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = self.lock();
        (*self.state.get()).allocate(layout)
    }

    // SAFETY: GlobalAlloc callers may only return pointers previously allocated
    // by this allocator, which HeapState uses to recover the original block.
    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        let _guard = self.lock();
        (*self.state.get()).deallocate(pointer);
    }
}

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: LockedHeap = LockedHeap::new();

/// Makes the supplied mapped memory available to Rust's global allocator.
///
/// `start` is the heap's virtual base and `size` is its mapped byte length.
/// Returns an error if the range cannot hold allocator metadata or the heap was
/// already initialized.
///
/// # Safety
/// The caller must provide an exclusively owned, writable, contiguous virtual
/// range of `size` bytes that remains mapped until kernel shutdown.
pub unsafe fn initialize(start: usize, size: usize) -> Result<(), &'static str> {
    #[cfg(not(test))]
    {
        return GLOBAL_ALLOCATOR.initialize(start, size);
    }
    #[cfg(test)]
    {
        let _ = (start, size);
        Err("global heap initialization is unavailable in unit tests")
    }
}

// TODO: Optimization Opportunity: replace the address-ordered first-fit scan
// with segregated lists or a buddy allocator if allocation latency becomes material.
fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

#[cfg(all(not(test), target_arch = "x86"))]
fn disable_interrupts() -> usize {
    let flags: usize;
    unsafe {
        core::arch::asm!(
            "pushfd",
            "pop {flags:e}",
            "cli",
            flags = out(reg) flags,
            options(nomem, nostack)
        );
    }
    flags
}

#[cfg(all(not(test), not(target_arch = "x86")))]
fn disable_interrupts() -> usize {
    0
}

#[cfg(all(not(test), target_arch = "x86"))]
fn restore_interrupts(flags: usize) {
    if flags & (1 << 9) != 0 {
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack));
        }
    }
}

#[cfg(all(not(test), not(target_arch = "x86")))]
fn restore_interrupts(_flags: usize) {}

#[cfg(test)]
mod tests {
    use super::{FreeBlock, HeapState};
    use core::alloc::Layout;

    #[repr(align(4096))]
    /// Page-aligned backing region used to test allocator alignment and reuse.
    struct TestMemory([u8; 16 * 1024]);

    #[test]
    fn allocation_honors_alignment_and_reuses_freed_blocks() {
        let mut memory = TestMemory([0; 16 * 1024]);
        let mut heap = HeapState::empty();
        unsafe {
            heap.initialize(memory.0.as_mut_ptr() as usize, memory.0.len())
                .unwrap();
            let layout = Layout::from_size_align(173, 256).unwrap();
            let first = heap.allocate(layout);
            let second = heap.allocate(Layout::from_size_align(91, 16).unwrap());
            assert!(!first.is_null());
            assert!(!second.is_null());
            assert_eq!(first as usize % 256, 0);
            assert_eq!(second as usize % 16, 0);

            heap.deallocate(first);
            let reused = heap.allocate(layout);
            assert_eq!(reused, first);
        }
    }

    #[test]
    fn adjacent_freed_allocations_coalesce() {
        let mut memory = TestMemory([0; 16 * 1024]);
        let mut heap = HeapState::empty();
        unsafe {
            heap.initialize(memory.0.as_mut_ptr() as usize, memory.0.len())
                .unwrap();
            let layout = Layout::from_size_align(1024, 16).unwrap();
            let first = heap.allocate(layout);
            let second = heap.allocate(layout);
            let large = heap.allocate(Layout::from_size_align(15 * 1024, 16).unwrap());
            assert!(!first.is_null());
            assert!(!second.is_null());
            assert!(large.is_null());

            heap.deallocate(first);
            heap.deallocate(second);
            let combined = heap.allocate(Layout::from_size_align(2048, 16).unwrap());
            assert!(!combined.is_null());
        }
    }

    #[test]
    fn allocation_before_initialization_returns_null() {
        let mut heap = HeapState::empty();
        let layout = Layout::from_size_align(8, 8).unwrap();
        assert!(unsafe { heap.allocate(layout) }.is_null());
    }

    #[test]
    fn heap_requires_space_for_a_free_block() {
        let mut memory = [0u8; core::mem::size_of::<FreeBlock>() - 1];
        let mut heap = HeapState::empty();
        assert!(unsafe { heap.initialize(memory.as_mut_ptr() as usize, memory.len()) }.is_err());
    }
}
