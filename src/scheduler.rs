use crate::memory::UserAddressSpace;
use core::ptr::{addr_of_mut, write_volatile};

const THREAD_COUNT: usize = 2;
const KERNEL_STACK_SIZE: usize = 16 * 1024;
const KERNEL_CODE_SELECTOR: u32 = 0x08;
const KERNEL_DATA_SELECTOR: u32 = 0x10;
const USER_CODE_SELECTOR: u32 = 0x1b;
const USER_DATA_SELECTOR: u32 = 0x23;
const INTERRUPTS_ENABLED_EFLAGS: u32 = 0x202;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
/// Runnable state used by the scheduler's fixed set of kernel tasks.
pub enum ThreadState {
    Ready,
    Running,
    Blocked,
}

#[derive(Clone, Copy)]
#[repr(C)]
/// CPU stack pointer saved by the assembly interrupt stub.
pub struct ContextState {
    pub saved_stack_pointer: u32,
}

#[derive(Clone, Copy)]
#[repr(C)]
/// Scheduler metadata required to resume one kernel or user thread.
pub struct ThreadControlBlock {
    pub id: u32,
    pub state: ThreadState,
    pub page_directory: u32,
    pub context_state: *mut ContextState,
    pub kernel_stack_top: u32,
}

impl ThreadControlBlock {
    const EMPTY: Self = Self {
        id: 0,
        state: ThreadState::Blocked,
        page_directory: 0,
        context_state: core::ptr::null_mut(),
        kernel_stack_top: 0,
    };
}

#[repr(align(16))]
/// Dedicated kernel stack used while a thread runs in the kernel.
struct KernelStack {
    _storage: [u8; KERNEL_STACK_SIZE],
}

#[repr(align(16))]
/// 16-byte-aligned FXSAVE image for one thread's floating-point state.
struct FpuState {
    _storage: [u8; 512],
}

static mut THREADS: [ThreadControlBlock; THREAD_COUNT] = [ThreadControlBlock::EMPTY; THREAD_COUNT];
static mut CONTEXTS: [ContextState; THREAD_COUNT] = [ContextState {
    saved_stack_pointer: 0,
}; THREAD_COUNT];
static mut KERNEL_STACKS: [KernelStack; THREAD_COUNT] = [
    KernelStack {
        _storage: [0; KERNEL_STACK_SIZE],
    },
    KernelStack {
        _storage: [0; KERNEL_STACK_SIZE],
    },
];
static mut FPU_STATES: [FpuState; THREAD_COUNT] = [
    FpuState { _storage: [0; 512] },
    FpuState { _storage: [0; 512] },
];
static mut CURRENT_THREAD: usize = 0;
static mut INITIALIZED: bool = false;

#[no_mangle]
/// Pointer to the FPU image belonging to the thread currently being switched.
pub static mut CURRENT_FPU_STATE: u32 = 0;

extern "C" {
    fn scheduler_start(page_directory: u32, saved_stack_pointer: u32) -> !;
}

/// Creates the desktop and isolated calculator thread contexts.
///
/// `kernel_page_directory` is the supervisor address-space root, `user_space`
/// supplies the calculator's directory and initial user stack, and
/// `desktop_entry` is the desktop task's kernel entry point. Returns an error
/// when any required context address is invalid.
pub fn initialize(
    kernel_page_directory: u32,
    user_space: UserAddressSpace,
    desktop_entry: u32,
) -> Result<(), &'static str> {
    if kernel_page_directory == 0 || user_space.page_directory == 0 || desktop_entry == 0 {
        return Err("scheduler received an invalid task context");
    }

    unsafe {
        let threads = addr_of_mut!(THREADS).cast::<ThreadControlBlock>();
        let contexts = addr_of_mut!(CONTEXTS).cast::<ContextState>();
        let stacks = addr_of_mut!(KERNEL_STACKS).cast::<KernelStack>();
        let fpu_states = addr_of_mut!(FPU_STATES).cast::<FpuState>();
        core::arch::asm!("fninit", options(nomem, nostack, preserves_flags));
        for index in 0..THREAD_COUNT {
            let state = fpu_states.add(index).cast::<u8>();
            core::arch::asm!(
                "fxsave [{0:e}]",
                in(reg) state,
                options(nostack, preserves_flags)
            );
        }

        for index in 0..THREAD_COUNT {
            let stack_top = (stacks.add(index) as u32) + KERNEL_STACK_SIZE as u32;
            let context = contexts.add(index);
            let (entry, page_directory, user_stack, code_selector, state) = if index == 0 {
                (
                    desktop_entry,
                    kernel_page_directory,
                    None,
                    KERNEL_CODE_SELECTOR,
                    ThreadState::Running,
                )
            } else {
                (
                    crate::memory::USER_CODE_VIRTUAL_BASE as u32,
                    user_space.page_directory,
                    Some(user_space.initial_stack_pointer),
                    USER_CODE_SELECTOR,
                    ThreadState::Ready,
                )
            };
            let initial_stack = build_initial_context(stack_top, entry, code_selector, user_stack);
            write_volatile(addr_of_mut!((*context).saved_stack_pointer), initial_stack);
            write_volatile(
                threads.add(index),
                ThreadControlBlock {
                    id: index as u32 + 1,
                    state,
                    page_directory,
                    context_state: context,
                    kernel_stack_top: stack_top,
                },
            );
        }
        CURRENT_THREAD = 0;
        CURRENT_FPU_STATE = fpu_states as u32;
        INITIALIZED = true;
    }
    Ok(())
}

/// Enters the first scheduled task and does not return.
///
/// # Safety
/// The scheduler must have been initialized, the first context must be valid,
/// and the GDT/TSS and interrupt vectors must already be installed.
pub fn start() -> ! {
    unsafe {
        if !INITIALIZED {
            crate::halt_forever();
        }
        let first = addr_of_mut!(THREADS).cast::<ThreadControlBlock>();
        let thread = &*first;
        crate::set_tss_kernel_stack_top(thread.kernel_stack_top);
        CURRENT_FPU_STATE = addr_of_mut!(FPU_STATES).cast::<FpuState>() as u32;
        let saved_stack_pointer = (*thread.context_state).saved_stack_pointer;
        scheduler_start(thread.page_directory, saved_stack_pointer)
    }
}

/// Saves the interrupted thread and selects the next ready thread.
///
/// `saved_stack_pointer` is the stack image produced by the IRQ0 assembly stub.
/// Returns the saved stack pointer of the context to restore.
///
/// # Safety
/// Must be called only from the timer interrupt path with interrupts masked and
/// a valid stack frame matching `irq0_stub`'s save/restore layout.
pub fn on_timer_interrupt(saved_stack_pointer: u32) -> u32 {
    unsafe {
        if !INITIALIZED {
            return saved_stack_pointer;
        }
        let threads = addr_of_mut!(THREADS).cast::<ThreadControlBlock>();
        let current = CURRENT_THREAD;
        let current_thread = &mut *threads.add(current);
        write_volatile(
            addr_of_mut!((*current_thread.context_state).saved_stack_pointer),
            saved_stack_pointer,
        );
        if current_thread.state == ThreadState::Running {
            current_thread.state = ThreadState::Ready;
        }

        // Round-robin from the current slot; if no other task is ready, the
        // current thread is selected again and resumes from its saved frame.
        let mut next = current;
        for distance in 1..=THREAD_COUNT {
            let candidate = (current + distance) % THREAD_COUNT;
            if (*threads.add(candidate)).state == ThreadState::Ready {
                next = candidate;
                break;
            }
        }

        let next_thread = &mut *threads.add(next);
        next_thread.state = ThreadState::Running;
        CURRENT_THREAD = next;
        CURRENT_FPU_STATE = addr_of_mut!(FPU_STATES).cast::<FpuState>().add(next) as u32;
        crate::set_tss_kernel_stack_top(next_thread.kernel_stack_top);

        let mut active_page_directory: u32;
        core::arch::asm!(
            "mov {0:e}, cr3",
            out(reg) active_page_directory,
            options(nomem, nostack, preserves_flags)
        );
        // A CR3 change flushes non-global translations and switches the active
        // address space before the assembly stub restores the selected context.
        if active_page_directory != next_thread.page_directory {
            active_page_directory = next_thread.page_directory;
            core::arch::asm!(
                "mov cr3, {0:e}",
                in(reg) active_page_directory,
                options(nostack, preserves_flags)
            );
        }
        (*next_thread.context_state).saved_stack_pointer
    }
}

unsafe fn build_initial_context(
    stack_top: u32,
    entry: u32,
    code_selector: u32,
    user_stack: Option<u32>,
) -> u32 {
    // This stack layout deliberately mirrors the segment, general-register,
    // and IRET frames consumed by scheduler_start and irq0_stub.
    let mut stack = stack_top as *mut u32;
    if let Some(user_stack) = user_stack {
        push(&mut stack, USER_DATA_SELECTOR);
        push(&mut stack, user_stack);
    } else {
        push(&mut stack, task_return_trap as *const () as usize as u32);
    }
    push(&mut stack, INTERRUPTS_ENABLED_EFLAGS);
    push(&mut stack, code_selector);
    push(&mut stack, entry);

    let data_selector = if user_stack.is_some() {
        USER_DATA_SELECTOR
    } else {
        KERNEL_DATA_SELECTOR
    };
    push(&mut stack, data_selector);
    push(&mut stack, data_selector);
    push(&mut stack, data_selector);
    push(&mut stack, data_selector);
    for _ in 0..8 {
        push(&mut stack, 0);
    }
    stack as u32
}

unsafe fn push(stack: &mut *mut u32, value: u32) {
    // SAFETY: the caller supplies a writable stack pointer within the task's
    // allocated kernel stack and reserves enough room for the complete frame.
    *stack = (*stack).sub(1);
    write_volatile(*stack, value);
}

extern "C" fn task_return_trap() -> ! {
    crate::halt_forever()
}
