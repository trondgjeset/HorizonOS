use core::ptr::{addr_of, addr_of_mut, read_unaligned, read_volatile, write_volatile};

const PAGE_SIZE: usize = 4096;
const PAGE_PRESENT: u32 = 1 << 0;
const PAGE_WRITABLE: u32 = 1 << 1;
const PAGE_WRITE_THROUGH: u32 = 1 << 3;
const PAGE_CACHE_DISABLE: u32 = 1 << 4;
const PAGE_FRAMEBUFFER_FLAGS: u32 =
    PAGE_PRESENT | PAGE_WRITABLE | PAGE_WRITE_THROUGH | PAGE_CACHE_DISABLE;
const FRAME_COUNT: usize = 1 << 20;
const BITMAP_BYTES: usize = FRAME_COUNT / 8;
/// Upper bound of the low-memory identity map retained during early boot.
pub const IDENTITY_MAPPED_LIMIT: usize = 16 * 1024 * 1024;
const IDENTITY_MAPPED_BYTES: usize = IDENTITY_MAPPED_LIMIT;
const TEST_VIRTUAL_ADDRESS: usize = 0xd000_0000;
const HEAP_VIRTUAL_BASE: usize = 0xc000_0000;
const HEAP_SIZE: usize = 16 * 1024 * 1024;
const HEAP_TABLE_COUNT: usize = HEAP_SIZE / (PAGE_SIZE * 1024);
const FRAMEBUFFER_VIRTUAL_BASE: usize = 0xe000_0000;
const FRAMEBUFFER_TABLE_COUNT: usize = 8;
/// Virtual address at which the isolated user calculation image is mapped.
pub const USER_CODE_VIRTUAL_BASE: usize = 0x0040_0000;
const USER_SHARED_VIRTUAL_BASE: usize = 0x0050_0000;
const USER_STACK_VIRTUAL_BASE: usize = 0x007f_f000;
const USER_DIRECTORY_INDEX: usize = USER_CODE_VIRTUAL_BASE >> 22;
const MULTIBOOT_BOOTLOADER_MAGIC: u32 = 0x2bad_b002;
const MULTIBOOT_INFO_MEMORY_MAP: u32 = 1 << 6;
const MULTIBOOT_INFO_MODULES: u32 = 1 << 3;

static mut FRAME_BITMAP: [u8; BITMAP_BYTES] = [0xff; BITMAP_BYTES];
static mut FRAME_SEARCH_START: usize = 256;

/// Prefix of the Multiboot information structure used by this memory manager.
#[repr(C)]
struct MultibootInfo {
    flags: u32,
    _memory_lower: u32,
    _memory_upper: u32,
    _boot_device: u32,
    _command_line: u32,
    module_count: u32,
    modules_address: u32,
    _symbols: [u32; 4],
    memory_map_length: u32,
    memory_map_address: u32,
}

/// Multiboot boot-module range used to reserve initrd and fixture memory.
#[repr(C)]
struct MultibootModule {
    start: u32,
    end: u32,
    _command_line: u32,
    _reserved: u32,
}

/// A page-aligned x86 page table or page directory.
#[repr(C, align(4096))]
struct PageTable([u32; 1024]);

static mut PAGE_DIRECTORY: PageTable = PageTable([0; 1024]);
static mut IDENTITY_TABLES: [PageTable; 4] = [
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
];
static mut TEST_TABLE: PageTable = PageTable([0; 1024]);
static mut HEAP_TABLES: [PageTable; HEAP_TABLE_COUNT] = [
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
];
static mut FRAMEBUFFER_TABLES: [PageTable; FRAMEBUFFER_TABLE_COUNT] = [
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
    PageTable([0; 1024]),
];
static mut USER_PAGE_DIRECTORY: PageTable = PageTable([0; 1024]);
static mut USER_PAGE_TABLE: PageTable = PageTable([0; 1024]);

/// Shared counter page mapped into the calculator's user address space.
#[repr(C, align(4096))]
struct UserSharedPage {
    calculation_count: u32,
    _reserved: [u8; PAGE_SIZE - core::mem::size_of::<u32>()],
}

static mut USER_SHARED_PAGE: UserSharedPage = UserSharedPage {
    calculation_count: 0,
    _reserved: [0; PAGE_SIZE - core::mem::size_of::<u32>()],
};

/// Kernel-created user address space and its initial stack pointer.
pub struct UserAddressSpace {
    pub page_directory: u32,
    pub initial_stack_pointer: u32,
}

extern "C" {
    static kernel_end: u8;
}

/// Returns the physical address of the kernel page directory for CR3.
pub fn kernel_page_directory() -> u32 {
    unsafe { addr_of!(PAGE_DIRECTORY.0) as u32 }
}

/// Builds an isolated address space for the supplied, page-aligned user code.
///
/// `user_code_start..user_code_end` identifies the user program's bytes. The
/// program must fit in one page; the result contains the directory address and
/// top of its initial user stack. Errors report invalid mappings or exhausted
/// page frames. The code page must be reserved and accessible through the
/// kernel's current identity mapping.
pub fn create_user_address_space(
    user_code_start: u32,
    user_code_end: u32,
) -> Result<UserAddressSpace, &'static str> {
    if user_code_start as usize % PAGE_SIZE != 0
        || user_code_end <= user_code_start
        || user_code_end as usize > user_code_start as usize + PAGE_SIZE
    {
        return Err("user code must fit within one aligned page");
    }

    let kernel_directory = unsafe { addr_of!(PAGE_DIRECTORY.0).cast::<u32>() };
    let user_directory = unsafe { addr_of_mut!(USER_PAGE_DIRECTORY.0).cast::<u32>() };
    let kernel_table_entry = unsafe { read_volatile(kernel_directory.add(USER_DIRECTORY_INDEX)) };
    if kernel_table_entry & PAGE_PRESENT == 0 {
        return Err("kernel identity page table for user mappings is missing");
    }

    // Copy the kernel mappings, then replace the user-code PDE with a private
    // table. PTE user bits are enabled only for the code, shared counter, and stack.
    let kernel_table = (kernel_table_entry & 0xffff_f000) as *const u32;
    let user_table = unsafe { addr_of_mut!(USER_PAGE_TABLE.0).cast::<u32>() };
    let shared_physical = addr_of!(USER_SHARED_PAGE) as u32;
    let stack_physical = allocate_frame().ok_or("no free frame for user stack")?;

    unsafe {
        for index in 0..1024 {
            write_volatile(
                user_directory.add(index),
                read_volatile(kernel_directory.add(index)),
            );
            write_volatile(
                user_table.add(index),
                read_volatile(kernel_table.add(index)),
            );
        }
        write_volatile(
            user_directory.add(USER_DIRECTORY_INDEX),
            user_table as u32 | 0x007,
        );
        map_user_page(
            user_table,
            USER_CODE_VIRTUAL_BASE,
            user_code_start,
            PAGE_PRESENT | 0x004,
        );
        map_user_page(
            user_table,
            USER_SHARED_VIRTUAL_BASE,
            shared_physical,
            PAGE_PRESENT | PAGE_WRITABLE | 0x004,
        );
        map_user_page(
            user_table,
            USER_STACK_VIRTUAL_BASE,
            stack_physical,
            PAGE_PRESENT | PAGE_WRITABLE | 0x004,
        );
        write_volatile(addr_of_mut!(USER_SHARED_PAGE.calculation_count), 0);

        let stack = USER_STACK_VIRTUAL_BASE as *mut u32;
        for index in 0..(PAGE_SIZE / core::mem::size_of::<u32>()) {
            write_volatile(stack.add(index), 0);
        }
    }

    Ok(UserAddressSpace {
        page_directory: user_directory as u32,
        initial_stack_pointer: (USER_STACK_VIRTUAL_BASE + PAGE_SIZE - 16) as u32,
    })
}

/// Reads the calculator's counter from the kernel-owned shared page.
pub fn user_calculation_count() -> u32 {
    unsafe { read_volatile(addr_of!(USER_SHARED_PAGE.calculation_count)) }
}

/// Installs one user-accessible PTE in `table`.
///
/// # Safety
/// `table` must point to a writable, mapped 1024-entry page table. The physical
/// frame must be reserved, and the supplied flags must reflect the intended
/// privilege and access policy.
unsafe fn map_user_page(table: *mut u32, virtual_address: usize, physical: u32, flags: u32) {
    let index = (virtual_address >> 12) & 0x3ff;
    write_volatile(table.add(index), (physical & 0xffff_f000) | flags);
}

/// Initializes physical-frame tracking and paging, then checks a supervisor mapping.
///
/// `boot_magic` and `multiboot_info_address` are the values passed by the
/// Multiboot loader. Returns an error when the boot metadata is invalid, required
/// physical memory is unavailable, or the paging self-test fails.
///
/// # Safety
/// `multiboot_info_address` must point to a readable Multiboot v1 information
/// structure and memory map within the currently accessible physical range.
pub fn initialize_and_test(
    boot_magic: u32,
    multiboot_info_address: u32,
) -> Result<(), &'static str> {
    if boot_magic != MULTIBOOT_BOOTLOADER_MAGIC {
        return Err("invalid Multiboot boot magic");
    }

    let info = unsafe { read_unaligned(multiboot_info_address as *const MultibootInfo) };
    if info.flags & MULTIBOOT_INFO_MEMORY_MAP == 0 || info.memory_map_length == 0 {
        return Err("Multiboot memory map is unavailable");
    }

    // The initial directory identity-maps low memory so boot metadata, kernel
    // objects, page tables, and loaded modules remain accessible after CR0.PG.
    unsafe {
        initialize_frame_bitmap(&info, multiboot_info_address)?;
        initialize_paging()?;
        test_supervisor_mapping()?;
    }

    Ok(())
}

/// Maps the reserved 16 MiB kernel heap and initializes the global allocator.
///
/// Returns an error if any virtual slot is already occupied or there are not
/// enough physical frames to back the complete range.
pub fn initialize_kernel_heap() -> Result<(), &'static str> {
    let directory = unsafe { addr_of_mut!(PAGE_DIRECTORY.0).cast::<u32>() };
    let tables = addr_of_mut!(HEAP_TABLES).cast::<PageTable>();
    let first_directory_index = HEAP_VIRTUAL_BASE >> 22;

    unsafe {
        // The heap's PDEs and PTEs are supervisor-only; each frame is claimed
        // before it is published in the page table.
        for table_index in 0..HEAP_TABLE_COUNT {
            if read_volatile(directory.add(first_directory_index + table_index)) & PAGE_PRESENT != 0
            {
                return Err("kernel heap virtual range is already mapped");
            }
        }

        for table_index in 0..HEAP_TABLE_COUNT {
            let table = tables.add(table_index);
            let table_address = table as u32;
            write_volatile(
                directory.add(first_directory_index + table_index),
                table_address | PAGE_PRESENT | PAGE_WRITABLE,
            );
            for entry_index in 0..1024 {
                let physical =
                    allocate_frame().ok_or("not enough physical memory for kernel heap")?;
                write_volatile(
                    (*table).0.as_mut_ptr().add(entry_index),
                    physical | PAGE_PRESENT | PAGE_WRITABLE,
                );
            }
        }

        for page_offset in (0..HEAP_SIZE).step_by(PAGE_SIZE) {
            core::arch::asm!(
                "invlpg [{0:e}]",
                in(reg) HEAP_VIRTUAL_BASE + page_offset,
                options(nostack, preserves_flags)
            );
        }
    }

    // TODO: Optimization Opportunity: consider demand-mapping heap pages so
    // small configurations do not reserve all 16 MiB of physical memory at boot.
    unsafe { crate::heap::initialize(HEAP_VIRTUAL_BASE, HEAP_SIZE) }
}

/// Maps a physical framebuffer range as write-through, cache-inhibited memory.
///
/// `physical_address` and `length` describe the device memory. Returns a pointer
/// to the first requested byte in the kernel framebuffer window, or an error if
/// the range does not fit the reserved mapping.
///
/// # Safety
/// The supplied physical range must describe the framebuffer device and must
/// not overlap ordinary RAM that the frame allocator may use.
pub fn map_framebuffer(physical_address: u64, length: usize) -> Result<*mut u8, &'static str> {
    if length == 0 {
        return Err("framebuffer size is zero");
    }

    let physical_end = physical_address
        .checked_add(length as u64)
        .ok_or("framebuffer physical range overflow")?;
    if physical_end > (u32::MAX as u64) + 1 {
        return Err("framebuffer is outside 32-bit physical address space");
    }

    let physical_page = physical_address & !((PAGE_SIZE as u64) - 1);
    let physical_offset = (physical_address - physical_page) as usize;
    let mapped_size = physical_offset
        .checked_add(length)
        .ok_or("framebuffer mapping size overflow")?;
    let page_count = mapped_size
        .checked_add(PAGE_SIZE - 1)
        .ok_or("framebuffer page count overflow")?
        / PAGE_SIZE;
    let maximum_pages = FRAMEBUFFER_TABLE_COUNT * 1024;
    if page_count > maximum_pages {
        return Err("framebuffer exceeds reserved mapping window");
    }

    let (directory, tables) = unsafe {
        (
            addr_of_mut!(PAGE_DIRECTORY.0).cast::<u32>(),
            addr_of_mut!(FRAMEBUFFER_TABLES).cast::<PageTable>(),
        )
    };
    let mapped_tables = (page_count + 1023) / 1024;
    let first_directory_index = FRAMEBUFFER_VIRTUAL_BASE >> 22;
    if first_directory_index + mapped_tables > 1024 {
        return Err("framebuffer mapping exceeds page directory");
    }

    unsafe {
        for table_index in 0..mapped_tables {
            let directory_index = first_directory_index + table_index;
            if read_volatile(directory.add(directory_index)) & 1 != 0 {
                return Err("framebuffer virtual mapping is already in use");
            }
        }

        for table_index in 0..mapped_tables {
            let directory_index = first_directory_index + table_index;
            let table = tables.add(table_index);
            write_volatile(directory.add(directory_index), table as u32 | 0x003);

            for entry_index in 0..1024 {
                let page_index = table_index * 1024 + entry_index;
                if page_index >= page_count {
                    break;
                }
                let physical = physical_page
                    .checked_add((page_index * PAGE_SIZE) as u64)
                    .ok_or("framebuffer page address overflow")?;
                if physical > u32::MAX as u64 {
                    return Err("framebuffer page exceeds 32-bit address space");
                }
                // Framebuffer pages are mapped with PWT/PCD to avoid caching
                // device writes as ordinary write-back memory.
                write_volatile(
                    (*table).0.as_mut_ptr().add(entry_index),
                    physical as u32 | PAGE_FRAMEBUFFER_FLAGS,
                );
            }
        }

        for page_index in 0..page_count {
            let virtual_address = FRAMEBUFFER_VIRTUAL_BASE + page_index * PAGE_SIZE;
            core::arch::asm!(
                "invlpg [{0:e}]",
                in(reg) virtual_address,
                options(nostack, preserves_flags)
            );
        }
    }

    Ok((FRAMEBUFFER_VIRTUAL_BASE + physical_offset) as *mut u8)
}

unsafe fn initialize_frame_bitmap(
    info: &MultibootInfo,
    multiboot_info_address: u32,
) -> Result<(), &'static str> {
    // SAFETY: `info` and all embedded ranges originate in the validated
    // Multiboot handoff and are expected to remain identity-mapped at this stage.
    FRAME_SEARCH_START = 256;
    let bitmap = addr_of_mut!(FRAME_BITMAP).cast::<u8>();
    for index in 0..BITMAP_BYTES {
        write_volatile(bitmap.add(index), 0xff);
    }

    // Start pessimistically with every frame reserved, mark only Multiboot
    // "available" ranges free, then reserve kernel and bootloader-owned ranges.
    let map_start = info.memory_map_address as usize;
    let map_length = info.memory_map_length as usize;
    let map_end = map_start
        .checked_add(map_length)
        .ok_or("Multiboot memory map address overflow")?;
    let mut cursor = map_start;

    while cursor < map_end {
        if map_end - cursor < 4 {
            return Err("truncated Multiboot memory map entry");
        }

        let entry_size = read_unaligned(cursor as *const u32) as usize;
        let total_size = entry_size
            .checked_add(4)
            .ok_or("Multiboot memory map entry size overflow")?;
        if entry_size < 20 || total_size > map_end - cursor {
            return Err("invalid Multiboot memory map entry");
        }

        let base = read_unaligned((cursor + 4) as *const u64);
        let length = read_unaligned((cursor + 12) as *const u64);
        let entry_type = read_unaligned((cursor + 20) as *const u32);

        if entry_type == 1 {
            mark_usable_frames(base, length);
        }
        cursor += total_size;
    }

    let kernel_end_address = addr_of!(kernel_end) as u64;
    reserve_range(0, 1024 * 1024);
    reserve_range(0, kernel_end_address);
    reserve_range(
        multiboot_info_address as u64,
        multiboot_info_address as u64 + core::mem::size_of::<MultibootInfo>() as u64,
    );
    reserve_range(map_start as u64, map_end as u64);

    if info.flags & MULTIBOOT_INFO_MODULES != 0 && info.module_count != 0 {
        let module_count = info.module_count as usize;
        let module_table_bytes = module_count
            .checked_mul(core::mem::size_of::<MultibootModule>())
            .ok_or("Multiboot module table size overflow")?;
        let module_table_start = info.modules_address as usize;
        let module_table_end = module_table_start
            .checked_add(module_table_bytes)
            .ok_or("Multiboot module table address overflow")?;
        reserve_range(module_table_start as u64, module_table_end as u64);

        for index in 0..module_count {
            let module = read_unaligned(
                (module_table_start + index * core::mem::size_of::<MultibootModule>())
                    as *const MultibootModule,
            );
            if module.end < module.start {
                return Err("invalid Multiboot module range");
            }
            reserve_range(module.start as u64, module.end as u64);
        }
    }

    Ok(())
}

fn mark_usable_frames(base: u64, length: u64) {
    let Some(end) = base.checked_add(length) else {
        return;
    };
    let start_frame = base.saturating_add((PAGE_SIZE - 1) as u64) / PAGE_SIZE as u64;
    let end_frame = (end / PAGE_SIZE as u64).min(FRAME_COUNT as u64);
    let first_frame = start_frame.min(FRAME_COUNT as u64);

    for frame in first_frame..end_frame {
        set_frame_used(frame as usize, false);
    }
}

fn reserve_range(start: u64, end: u64) {
    let first_frame = (start / PAGE_SIZE as u64).min(FRAME_COUNT as u64);
    let end_frame = end
        .saturating_add((PAGE_SIZE - 1) as u64)
        .checked_div(PAGE_SIZE as u64)
        .unwrap_or(FRAME_COUNT as u64)
        .min(FRAME_COUNT as u64);

    for frame in first_frame..end_frame {
        set_frame_used(frame as usize, true);
    }
}

fn set_frame_used(frame: usize, used: bool) {
    if frame >= FRAME_COUNT {
        return;
    }
    let byte = frame / 8;
    let mask = 1 << (frame % 8);
    unsafe {
        let bitmap = addr_of_mut!(FRAME_BITMAP).cast::<u8>();
        let value = read_volatile(bitmap.add(byte));
        write_volatile(
            bitmap.add(byte),
            if used { value | mask } else { value & !mask },
        );
    }
}

fn allocate_frame() -> Option<u32> {
    let first_frame = unsafe { FRAME_SEARCH_START.max(256) };
    for frame in first_frame..FRAME_COUNT {
        if let Some(physical) = claim_frame(frame) {
            unsafe {
                FRAME_SEARCH_START = frame + 1;
            }
            return Some(physical);
        }
    }
    for frame in 256..first_frame {
        if let Some(physical) = claim_frame(frame) {
            unsafe {
                FRAME_SEARCH_START = frame + 1;
            }
            return Some(physical);
        }
    }
    None
}

fn claim_frame(frame: usize) -> Option<u32> {
    if frame >= FRAME_COUNT {
        return None;
    }
    let byte = frame / 8;
    let mask = 1 << (frame % 8);
    unsafe {
        let bitmap = addr_of_mut!(FRAME_BITMAP).cast::<u8>();
        let used = read_volatile(bitmap.add(byte));
        if used & mask == 0 {
            write_volatile(bitmap.add(byte), used | mask);
            Some((frame * PAGE_SIZE) as u32)
        } else {
            None
        }
    }
}

unsafe fn initialize_paging() -> Result<(), &'static str> {
    // SAFETY: the statically allocated page directory and tables are aligned,
    // writable, identity-mapped, and remain live for the kernel's lifetime.
    let kernel_end_address = addr_of!(kernel_end) as usize;
    if kernel_end_address > IDENTITY_MAPPED_BYTES {
        return Err("kernel exceeds the initial 16 MiB identity map");
    }

    let directory = addr_of_mut!(PAGE_DIRECTORY.0).cast::<u32>();
    let tables = addr_of_mut!(IDENTITY_TABLES).cast::<PageTable>();

    for table_index in 0..4 {
        let table = tables.add(table_index);
        let table_physical = table as u32;
        write_volatile(directory.add(table_index), table_physical | 0x003);

        for entry_index in 0..1024 {
            let physical = ((table_index * 1024 + entry_index) * PAGE_SIZE) as u32;
            let flags = if physical == 0 { 0 } else { 0x003 };
            write_volatile((*table).0.as_mut_ptr().add(entry_index), physical | flags);
        }
    }

    // Load CR3 before setting CR0.PG; CR0.WP also enforces supervisor write
    // permissions for read-only mappings.
    let directory_address = directory as u32;
    core::arch::asm!("mov cr3, {0:e}", in(reg) directory_address, options(nostack));

    let mut cr0: u32;
    core::arch::asm!("mov {0:e}, cr0", out(reg) cr0, options(nostack));
    cr0 |= (1 << 31) | (1 << 16);
    core::arch::asm!("mov cr0, {0:e}", in(reg) cr0, options(nostack));

    Ok(())
}

unsafe fn test_supervisor_mapping() -> Result<(), &'static str> {
    // SAFETY: the test virtual range is reserved for this page table, and the
    // frame is claimed before its present supervisor PTE is installed.
    let frame = allocate_frame().ok_or("no free page frame below 16 MiB")?;
    let directory_index = TEST_VIRTUAL_ADDRESS >> 22;
    let table_address = addr_of_mut!(TEST_TABLE.0) as u32;
    let directory = addr_of_mut!(PAGE_DIRECTORY.0).cast::<u32>();
    write_volatile(directory.add(directory_index), table_address | 0x003);

    let table = addr_of_mut!(TEST_TABLE.0).cast::<u32>();
    let table_index = (TEST_VIRTUAL_ADDRESS >> 12) & 0x3ff;
    let page_entry = frame | 0x003;
    write_volatile(table.add(table_index), page_entry);
    if read_volatile(table.add(table_index)) & 0x004 != 0 {
        return Err("test page is unexpectedly user-accessible");
    }
    core::arch::asm!("invlpg [{0:e}]", in(reg) TEST_VIRTUAL_ADDRESS, options(nostack, preserves_flags));

    let test_address = TEST_VIRTUAL_ADDRESS as *mut u32;
    write_volatile(test_address, 0x485a_4f4e);
    if read_volatile(test_address) != 0x485a_4f4e {
        return Err("virtual page read/write verification failed");
    }

    Ok(())
}
