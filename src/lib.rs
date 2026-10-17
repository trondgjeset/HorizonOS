#![no_std]

//! Horizon's freestanding 32-bit kernel entry point and hardware integration.
//!
//! The boot stub establishes CPU state, then this crate installs descriptor
//! tables, paging, the heap, devices, and the desktop before starting the
//! interrupt-driven scheduler.
extern crate alloc;

mod clock;
mod filesystem;
mod font8x8;
mod graphics;
mod heap;
mod keyboard;
mod memory;
mod mouse;
mod pe;
mod scheduler;
mod ui;
pub mod win32;
mod window_manager;

use core::mem::MaybeUninit;
use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_unaligned, read_volatile, write_volatile};

const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;
const VGA_CELLS: usize = VGA_WIDTH * VGA_HEIGHT;
const VGA_BUFFER: *mut u16 = 0xb8000 as *mut u16;
const COLOR: u16 = 0x0f00;
const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xa0;
const PIC2_DATA: u16 = 0xa1;
const KEYBOARD_DATA: u16 = 0x60;
const IDT_ENTRIES: usize = 256;
const IRQ0_VECTOR: usize = 32;
const IRQ1_VECTOR: usize = 33;
const IRQ12_VECTOR: usize = 44;
const MULTIBOOT_INFO_MODULES: u32 = 1 << 3;
const IDENTITY_MAPPED_LIMIT: u32 = 16 * 1024 * 1024;
const TSS_SELECTOR: u32 = 0x28;
const TSS_SIZE: usize = 104;

static mut GDT: [u64; 6] = [
    0,
    0x00cf_9a00_0000_ffff,
    0x00cf_9200_0000_ffff,
    0x00cf_fa00_0000_ffff,
    0x00cf_f200_0000_ffff,
    0,
];
#[repr(align(16))]
/// Storage for the 32-bit hardware Task State Segment.
struct TaskState {
    _storage: [u8; TSS_SIZE],
}
static mut TSS: TaskState = TaskState {
    _storage: [0; TSS_SIZE],
};
static mut WINDOW_MANAGER: MaybeUninit<window_manager::WindowManager> = MaybeUninit::uninit();
static mut BOOT_INFO_ADDRESS: u32 = 0;

#[derive(Clone, Copy)]
#[repr(C, packed)]
/// x86 pseudo-descriptor consumed by LGDT and LIDT.
struct TablePointer {
    limit: u16,
    base: u32,
}

#[derive(Clone, Copy)]
#[repr(C, packed)]
/// Eight-byte x86 interrupt-gate descriptor.
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    zero: u8,
    flags: u8,
    offset_high: u16,
}

#[repr(C)]
/// Initial Multiboot fields needed to locate the boot modules.
struct MultibootInfoPrefix {
    flags: u32,
    _memory_lower: u32,
    _memory_upper: u32,
    _boot_device: u32,
    _command_line: u32,
    module_count: u32,
    module_address: u32,
}

#[repr(C)]
/// Multiboot module start/end addresses and command-line metadata.
struct MultibootModule {
    start: u32,
    end: u32,
    command_line: u32,
    _reserved: u32,
}

impl IdtEntry {
    const EMPTY: Self = Self {
        offset_low: 0,
        selector: 0,
        zero: 0,
        flags: 0,
        offset_high: 0,
    };

    /// Builds a present, ring-0 interrupt gate for a 32-bit handler address.
    fn interrupt_gate(handler: u32) -> Self {
        Self {
            offset_low: handler as u16,
            selector: 0x08,
            zero: 0,
            flags: 0x8e,
            offset_high: (handler >> 16) as u16,
        }
    }
}

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::EMPTY; IDT_ENTRIES];
static mut CURSOR: usize = 0;
static mut SHIFT_PRESSED: bool = false;
static mut CAPS_LOCK: bool = false;

extern "C" {
    fn load_gdt(pointer: *const TablePointer);
    fn load_idt(pointer: *const TablePointer);
    fn load_tss(selector: u32);
    fn irq0_stub();
    fn irq1_stub();
    fn irq12_stub();
    fn unexpected_interrupt_stub();
    static user_calculation_entry: u8;
    static user_calculation_end: u8;
}

#[no_mangle]
/// Rust kernel entry called by `src/boot.s` with the Multiboot handoff values.
///
/// `boot_magic` identifies Multiboot v1 and `multiboot_info_address` points to
/// its boot-information structure. This routine initializes the machine and
/// transfers control to the scheduler; it never returns.
///
/// # Safety
/// Must be called only by the matching boot stub with a valid Multiboot handoff,
/// an established stack, and the CPU in 32-bit protected mode.
pub extern "C" fn rust_kernel_main(boot_magic: u32, multiboot_info_address: u32) -> ! {
    clear_screen();
    write_str("Horizon OS - paging and interrupt test\n");

    initialize_gdt();

    initialize_idt();
    initialize_pic();
    clock::initialize();

    write_str("GDT and IDT loaded.\n");
    match memory::initialize_and_test(boot_magic, multiboot_info_address) {
        Ok(()) => {
            debug_write_str("memory ready\n");
            write_str("Paging active; page-frame allocation passed.\n");
            write_str("Supervisor virtual page read/write passed.\n");
        }
        Err(message) => {
            write_str("Memory initialization failed: ");
            write_str(message);
            write_str("\n");
            halt_forever();
        }
    }
    match memory::initialize_kernel_heap() {
        Ok(()) => debug_write_str("kernel heap initialized: 16 MiB\n"),
        Err(message) => {
            debug_write_str("Kernel heap initialization failed: ");
            debug_write_str(message);
            debug_write_str("\n");
            write_str("Kernel heap initialization failed: ");
            write_str(message);
            write_str("\n");
            halt_forever();
        }
    }
    match filesystem::initialize_from_multiboot(boot_magic, multiboot_info_address) {
        Ok(()) => debug_write_str("C: RAM drive indexed from initrd\n"),
        Err(message) => {
            debug_write_str("C: RAM drive initialization failed: ");
            debug_write_str(message);
            debug_write_str("\n");
            write_str("RAM drive initialization failed: ");
            write_str(message);
            write_str("\n");
            halt_forever();
        }
    }

    match graphics::initialize(multiboot_info_address) {
        Ok(()) => debug_write_str("framebuffer ready\n"),
        Err(message) => {
            debug_write_str("Graphics initialization failed: ");
            debug_write_str(message);
            debug_write_str("\n");
            write_str("Graphics initialization failed: ");
            write_str(message);
            write_str("\n");
            halt_forever();
        }
    }

    let (width, height) = match graphics::dimensions() {
        Ok(dimensions) => dimensions,
        Err(message) => {
            write_str("Framebuffer dimensions unavailable: ");
            write_str(message);
            write_str("\n");
            halt_forever();
        }
    };

    match mouse::initialize(width, height) {
        Ok(()) => debug_write_str("mouse ready\n"),
        Err(message) => {
            debug_write_str("Mouse initialization failed: ");
            debug_write_str(message);
            debug_write_str("\n");
            write_str("Mouse initialization failed: ");
            write_str(message);
            write_str("\n");
            halt_forever();
        }
    }

    let mut window_manager = match window_manager::WindowManager::new(width, height) {
        Ok(manager) => manager,
        Err(message) => {
            debug_write_str("Window manager initialization failed: ");
            debug_write_str(message);
            debug_write_str("\n");
            write_str("Window manager initialization failed: ");
            write_str(message);
            write_str("\n");
            halt_forever();
        }
    };
    let wallpaper_result = (|| {
        // The boot archive is immutable; retain an owned, size-bounded byte
        // copy in the window manager so each redraw can safely decode it.
        // NOTE: Contribution Area: route wallpaper selection through a
        // settings-controlled VFS path after user preferences are implemented.
        let entry = filesystem::resolve_windows_path(r"C:\Windows\System\wallpaper.bmp")?;
        if entry.is_directory {
            return Err("desktop wallpaper path resolves to a directory");
        }
        if entry.size > graphics::bmp::MAX_WALLPAPER_BYTES {
            return Err("desktop wallpaper exceeds the decoder size limit");
        }
        let mut bytes = alloc::vec::Vec::new();
        bytes
            .try_reserve_exact(entry.size)
            .map_err(|_| "not enough heap memory to load the desktop wallpaper")?;
        bytes.resize(entry.size, 0);
        let bytes_read = filesystem::read_file(r"C:\Windows\System\wallpaper.bmp", &mut bytes)?;
        if bytes_read != entry.size {
            return Err("desktop wallpaper size changed while reading");
        }
        window_manager.set_wallpaper(bytes)
    })();
    match wallpaper_result {
        Ok(()) => {
            debug_write_str("desktop wallpaper loaded from C:\\Windows\\System\\wallpaper.bmp\n")
        }
        Err(message) => {
            debug_write_str("Wallpaper load failed; using the slate background: ");
            debug_write_str(message);
            debug_write_str("\n");
        }
    }
    let initial_clock = clock::read_datetime();
    if let Err(message) = initial_clock {
        debug_write_str("RTC unavailable: ");
        debug_write_str(message);
        debug_write_str("\n");
    }
    window_manager.update_clock(initial_clock);

    let initial_mouse = mouse::snapshot();
    if let Err(message) = window_manager
        .render()
        .and_then(|()| {
            graphics::draw_cursor(initial_mouse.x, initial_mouse.y, initial_mouse.buttons)
        })
        .and_then(|()| graphics::flush())
    {
        debug_write_str("Initial display failed: ");
        debug_write_str(message);
        debug_write_str("\n");
        write_str("Initial display failed: ");
        write_str(message);
        write_str("\n");
        halt_forever();
    }
    debug_write_str("initial frame flushed\n");

    unsafe {
        addr_of_mut!(WINDOW_MANAGER)
            .cast::<window_manager::WindowManager>()
            .write(window_manager);
        write_volatile(addr_of_mut!(BOOT_INFO_ADDRESS), multiboot_info_address);
    }
    let user_space = match memory::create_user_address_space(
        addr_of!(user_calculation_entry) as u32,
        addr_of!(user_calculation_end) as u32,
    ) {
        Ok(space) => space,
        Err(message) => {
            debug_write_str("User address-space initialization failed: ");
            debug_write_str(message);
            debug_write_str("\n");
            halt_forever();
        }
    };
    if let Err(message) = scheduler::initialize(
        memory::kernel_page_directory(),
        user_space,
        desktop_task as *const () as usize as u32,
    ) {
        debug_write_str("Scheduler initialization failed: ");
        debug_write_str(message);
        debug_write_str("\n");
        halt_forever();
    }
    debug_write_str(
        "PIT scheduler ready at 100 Hz; launching desktop with Ring 3 calculator blocked\n",
    );
    scheduler::start()
}

/// Wakes the pre-created Ring 3 calculator after a utility-launch action succeeds.
///
/// The scheduler prepared the isolated address space and context during boot,
/// but keeps the task blocked until this action-routing point.
///
/// NOTE: Action Routing: invoke only after the utility window is visible; report
/// scheduler failures rather than disguising them as a successful launch.
fn wake_utility_calculator() {
    match scheduler::wake_calculator() {
        Ok(()) => debug_write_str("Ring 3 calculator task queued by System Utility\n"),
        Err(message) => {
            debug_write_str("Ring 3 calculator task could not be queued: ");
            debug_write_str(message);
            debug_write_str("\n");
        }
    }
}

/// Runs desktop events and routes typed UI actions to kernel-owned operations.
///
/// NOTE: Action Routing: preserve the UI/scheduler boundary here; Start Menu
/// hit testing returns an enum and never manipulates scheduler state directly.
extern "C" fn desktop_task() -> ! {
    let manager =
        unsafe { &mut *addr_of_mut!(WINDOW_MANAGER).cast::<window_manager::WindowManager>() };
    let multiboot_info_address = unsafe { read_volatile(addr_of!(BOOT_INFO_ADDRESS)) };
    let mut displayed_second = clock::seconds();
    let mut displayed_calculation_update = clock::ticks() / 10;
    let mut displayed_caret_phase = clock::ticks() / window_manager::CARET_BLINK_TICKS;
    loop {
        let mut redraw = false;
        if mouse::take_dirty() {
            let mut state = mouse::snapshot();
            state.left_pressed_at = mouse::take_left_press();
            match manager.handle_mouse(state) {
                Ok(Some(window_manager::UiAction::RunSamplePe)) => {
                    debug_write_str("Run Sample PE selected\n");
                    match manager.show_pe_result(parse_pe_module(multiboot_info_address)) {
                        Ok(()) => wake_utility_calculator(),
                        Err(message) => {
                            debug_write_str("System Utility could not open: ");
                            debug_write_str(message);
                            debug_write_str("\n");
                        }
                    }
                }
                Ok(Some(window_manager::UiAction::RunWin32Demo)) => {
                    debug_write_str("Run Win32 Demo selected\n");
                    // NOTE: This is a native kernel smoke test, not a guest
                    // process request; no untrusted user pointer is forwarded.
                    if let Err(message) = manager.show_win32_demo() {
                        debug_write_str("Win32 GDI demo could not open: ");
                        debug_write_str(message);
                        debug_write_str("\n");
                    }
                }
                Ok(Some(window_manager::UiAction::LaunchExternalApp)) => {
                    debug_write_str("External PE staging requested: C:\\Apps\\TEST.EXE\n");
                    // NOTE: This stages an untrusted file in kernel-owned,
                    // non-executable memory only; the fixed scheduler and VMM
                    // cannot create a separate dynamic process address space.
                    match manager.show_pe_result(
                        win32::loader::load_from_vfs(r"C:\Apps\TEST.EXE")
                            .map(|image| image.summary()),
                    ) {
                        Ok(()) => wake_utility_calculator(),
                        Err(message) => {
                            debug_write_str("System Utility could not open: ");
                            debug_write_str(message);
                            debug_write_str("\n");
                        }
                    }
                }
                // NOTE: Action Routing: window allocation succeeds before the
                // scheduler releases the isolated calculator context.
                Ok(Some(window_manager::UiAction::ShowSystemUtility)) => {
                    debug_write_str("System Utility selected\n");
                    match manager.show_scratchpad() {
                        Ok(()) => wake_utility_calculator(),
                        Err(message) => {
                            debug_write_str("System Utility could not open: ");
                            debug_write_str(message);
                            debug_write_str("\n");
                        }
                    }
                }
                Ok(Some(window_manager::UiAction::SaveEditorFile)) => {
                    debug_write_str("Native Save File intent granted\n");
                    if let Err(message) = manager.save_editor_file() {
                        debug_write_str("Save File failed: ");
                        debug_write_str(message);
                        debug_write_str("\n");
                    }
                }
                Ok(Some(window_manager::UiAction::OpenTextFile(entry))) => {
                    if let Err(message) = manager.show_text_file(entry) {
                        debug_write_str("Text Viewer could not open file: ");
                        debug_write_str(message);
                        debug_write_str("\n");
                    }
                }
                Ok(Some(window_manager::UiAction::OpenThisPC)) => {
                    debug_write_str("This PC desktop shortcut opened\n");
                    if let Err(message) = manager.show_this_pc() {
                        debug_write_str("This PC window could not open: ");
                        debug_write_str(message);
                        debug_write_str("\n");
                    }
                }
                Ok(Some(window_manager::UiAction::OpenUserFolder)) => {
                    debug_write_str("User Folder desktop shortcut opened\n");
                    if let Err(message) = manager.show_user_folder() {
                        debug_write_str("User Documents folder could not open: ");
                        debug_write_str(message);
                        debug_write_str("\n");
                    }
                }
                Ok(Some(window_manager::UiAction::OpenRecycleBin)) => {
                    debug_write_str("Recycle Bin desktop shortcut opened\n");
                    if let Err(message) = manager.show_recycle_bin() {
                        debug_write_str("Recycle Bin folder could not open: ");
                        debug_write_str(message);
                        debug_write_str("\n");
                    }
                }
                Ok(None) => {}
                Err(message) => {
                    debug_write_str("Desktop input handling failed: ");
                    debug_write_str(message);
                    debug_write_str("\n");
                }
            }
            redraw = true;
        }

        while let Some(character) = keyboard::pop_character() {
            if let Err(message) = manager.handle_character(character) {
                debug_write_str("Keyboard text allocation failed: ");
                debug_write_str(message);
                debug_write_str("\n");
            }
            redraw = true;
        }
        let dropped_characters = keyboard::take_dropped_count();
        if dropped_characters > 0 {
            debug_write_str("Keyboard input queue overflow; dropped characters: ");
            write_decimal_usize_debug(dropped_characters);
            debug_write_str("\n");
        }

        let tick = clock::ticks();
        // IRQ1 only queues ASCII. This task applies String mutations and
        // repaints the caret on alternating tick windows while the field is focused.
        let caret_phase = tick / window_manager::CARET_BLINK_TICKS;
        if caret_phase != displayed_caret_phase {
            displayed_caret_phase = caret_phase;
            redraw |= manager.input_caret_focused();
        }
        let calculation_update = tick / 10;
        if calculation_update != displayed_calculation_update {
            displayed_calculation_update = calculation_update;
            redraw |= manager.update_background_counter(memory::user_calculation_count());
        }

        let current_second = clock::seconds();
        if current_second != displayed_second {
            displayed_second = current_second;
            let current_clock = clock::read_datetime();
            if let Err(message) = current_clock {
                debug_write_str("RTC read failed: ");
                debug_write_str(message);
                debug_write_str("\n");
            }
            manager.update_clock(current_clock);
            redraw = true;
        }

        if redraw {
            let state = mouse::snapshot();
            if let Err(message) = manager
                .render()
                .and_then(|()| graphics::draw_cursor(state.x, state.y, state.buttons))
                .and_then(|()| graphics::flush())
            {
                debug_write_str("Display refresh failed: ");
                debug_write_str(message);
                debug_write_str("\n");
                halt_forever();
            }
        }

        unsafe {
            core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
        }
        if mouse::has_pending_event()
            || keyboard::has_pending_input()
            || (manager.input_caret_focused()
                && clock::ticks() / window_manager::CARET_BLINK_TICKS != displayed_caret_phase)
            || clock::seconds() != displayed_second
        {
            unsafe {
                core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
            }
            continue;
        }
        unsafe {
            core::arch::asm!("sti", "hlt", options(nomem, nostack));
        }
    }
}

fn parse_pe_module(multiboot_info_address: u32) -> Result<pe::PeSummary, &'static str> {
    // `multiboot_info_address` is supplied by GRUB and must remain within the
    // early identity map; all subsequent module and command-line ranges are
    // checked before converting their physical addresses to Rust slices.
    let info = unsafe { read_unaligned(multiboot_info_address as *const MultibootInfoPrefix) };
    if info.flags & MULTIBOOT_INFO_MODULES == 0 || info.module_count == 0 {
        return Err("GRUB did not provide a fixture module");
    }

    let module_table_size = (info.module_count as usize)
        .checked_mul(core::mem::size_of::<MultibootModule>())
        .ok_or("Multiboot module table size overflow")?;
    let module_table_end = (info.module_address as usize)
        .checked_add(module_table_size)
        .ok_or("Multiboot module table address overflow")?;
    if module_table_end > IDENTITY_MAPPED_LIMIT as usize {
        return Err("Multiboot module table is outside the current identity map");
    }

    // Multiboot module records contain physical/identity addresses, not Rust
    // references. Check the complete table and selected module before creating
    // the borrowed slice passed to the bounds-checking PE parser.
    let mut selected = None;
    for index in 0..info.module_count as usize {
        let address =
            info.module_address as usize + index * core::mem::size_of::<MultibootModule>();
        let module = unsafe { read_unaligned(address as *const MultibootModule) };
        if multiboot_command_line_equals(module.command_line, b"hello-fixture") {
            selected = Some(module);
            break;
        }
    }
    let module = selected.ok_or("GRUB did not provide the hello-fixture module")?;
    if module.end <= module.start || module.end > IDENTITY_MAPPED_LIMIT {
        return Err("fixture is outside the current identity map");
    }

    let fixture = unsafe {
        core::slice::from_raw_parts(
            module.start as *const u8,
            (module.end - module.start) as usize,
        )
    };
    let image = pe::parse_pe32(fixture)?;
    Ok(image.summary())
}

fn multiboot_command_line_equals(address: u32, expected: &[u8]) -> bool {
    let Some(command_line_end) = (address as usize).checked_add(expected.len() + 1) else {
        return false;
    };
    if address == 0 || command_line_end > IDENTITY_MAPPED_LIMIT as usize {
        return false;
    }
    for (index, expected_byte) in expected.iter().enumerate() {
        if unsafe { read_volatile((address as *const u8).add(index)) } != *expected_byte {
            return false;
        }
    }
    unsafe { read_volatile((address as *const u8).add(expected.len())) == 0 }
}

fn halt_forever() -> ! {
    loop {
        unsafe {
            core::arch::asm!("cli", "hlt", options(nomem, nostack));
        }
    }
}

/// Writes a message to QEMU's conventional debug-console port `0xE9`.
///
/// `message` is emitted byte-for-byte and has no return value. This is a
/// diagnostic channel; it does not require the graphical framebuffer.
pub(crate) fn debug_write_str(message: &str) {
    for byte in message.bytes() {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") 0xe9u16,
                in("al") byte,
                options(nomem, nostack, preserves_flags)
            );
        }
    }
}

fn initialize_gdt() {
    unsafe {
        let tss = addr_of_mut!(TSS).cast::<u8>();
        write_volatile(tss.add(8).cast::<u16>(), 0x10);
        write_volatile(tss.add(102).cast::<u16>(), TSS_SIZE as u16);
        let base = tss as u32;
        let limit = (TSS_SIZE - 1) as u32;
        // Encode a 32-bit available TSS descriptor. The GDT's predefined code
        // and data entries include ring-0 and ring-3 selectors for IRET.
        let descriptor = u64::from(limit & 0xffff)
            | (u64::from(base & 0xffff) << 16)
            | (u64::from((base >> 16) & 0xff) << 32)
            | (0x89u64 << 40)
            | (u64::from((limit >> 16) & 0x0f) << 48)
            | (u64::from((base >> 24) & 0xff) << 56);
        write_volatile(addr_of_mut!(GDT).cast::<u64>().add(5), descriptor);
        let gdt_pointer = TablePointer {
            limit: (core::mem::size_of::<[u64; 6]>() - 1) as u16,
            base: addr_of!(GDT) as u32,
        };
        load_gdt(&gdt_pointer);
        load_tss(TSS_SELECTOR);
    }
}

/// Updates the TSS ring-0 stack used on privilege transitions.
///
/// `stack_top` is the top of the kernel stack for the thread about to run.
///
/// # Safety
/// The TSS must already be loaded, and `stack_top` must point one byte past a
/// mapped writable kernel stack.
pub(crate) fn set_tss_kernel_stack_top(stack_top: u32) {
    unsafe {
        write_volatile(
            addr_of_mut!(TSS).cast::<u8>().add(4).cast::<u32>(),
            stack_top,
        );
    }
}

fn write_decimal_usize_debug(mut value: usize) {
    let mut buffer = [0u8; 20];
    let mut offset = buffer.len();
    loop {
        offset -= 1;
        buffer[offset] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for byte in &buffer[offset..] {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") 0xe9u16,
                in("al") *byte,
                options(nomem, nostack, preserves_flags)
            );
        }
    }
}

fn initialize_idt() {
    let unexpected = unexpected_interrupt_stub as *const () as usize as u32;
    let irq0 = irq0_stub as *const () as usize as u32;
    let irq1 = irq1_stub as *const () as usize as u32;
    let irq12 = irq12_stub as *const () as usize as u32;
    let idt = addr_of_mut!(IDT).cast::<IdtEntry>();

    // Install a valid fallback at every vector before enabling IRQs, then
    // replace only the timer, keyboard, and auxiliary-device vectors.
    for vector in 0..IDT_ENTRIES {
        let handler = match vector {
            IRQ0_VECTOR => irq0,
            IRQ1_VECTOR => irq1,
            IRQ12_VECTOR => irq12,
            _ => unexpected,
        };
        unsafe {
            write_volatile(idt.add(vector), IdtEntry::interrupt_gate(handler));
        }
    }

    let idt_pointer = TablePointer {
        limit: (core::mem::size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
        base: idt as u32,
    };
    unsafe {
        load_idt(&idt_pointer);
    }
}

fn initialize_pic() {
    unsafe {
        write_port(PIC1_COMMAND, 0x11);
        io_wait();
        write_port(PIC2_COMMAND, 0x11);
        io_wait();
        write_port(PIC1_DATA, 0x20);
        io_wait();
        write_port(PIC2_DATA, 0x28);
        io_wait();
        write_port(PIC1_DATA, 0x04);
        io_wait();
        write_port(PIC2_DATA, 0x02);
        io_wait();
        write_port(PIC1_DATA, 0x01);
        io_wait();
        write_port(PIC2_DATA, 0x01);
        io_wait();

        // Unmask timer IRQ0, keyboard IRQ1, the IRQ2 cascade, and slave IRQ12.
        write_port(PIC1_DATA, 0xf8);
        write_port(PIC2_DATA, 0xef);
    }
}

#[no_mangle]
/// Handles PIT IRQ0, accounts for the tick, and returns the selected stack.
///
/// `saved_stack_pointer` is the interrupt frame created by `irq0_stub`; the
/// returned pointer is restored by that assembly stub.
///
/// # Safety
/// Called only through the configured IRQ0 gate with its expected stack frame.
pub extern "C" fn rust_irq0_handler(saved_stack_pointer: u32) -> u32 {
    clock::tick();
    scheduler::on_timer_interrupt(saved_stack_pointer)
}

#[no_mangle]
/// Reads and translates one keyboard scan code, then acknowledges PIC IRQ1.
///
/// # Safety
/// Must be invoked through the IRQ1 gate after the PIC and keyboard controller
/// have been initialized.
pub extern "C" fn rust_irq1_handler() {
    let scan_code = unsafe { read_port(KEYBOARD_DATA) };
    handle_scan_code(scan_code);
    unsafe {
        write_port(PIC1_COMMAND, 0x20);
    }
}

#[no_mangle]
/// Reports an unhandled CPU exception/interrupt and halts the processor.
///
/// # Safety
/// Called only through an installed IDT gate while running in ring 0.
pub extern "C" fn rust_unexpected_interrupt_handler() -> ! {
    write_str("\nUnexpected interrupt or CPU exception; halted.\n");
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Updates modifier state or queues one translated printable/control key.
///
/// `scan_code` is one byte read from the PS/2 controller in the IRQ1 handler.
/// This routine never allocates or mutates a window string; the desktop task
/// consumes the bounded queue after interrupt return.
///
/// NOTE: Contribution Area: add extended scan-code state and keyboard-layout
/// tables here without moving dynamic allocation into interrupt context.
fn handle_scan_code(scan_code: u8) {
    let released = scan_code & 0x80 != 0;
    let key = scan_code & 0x7f;

    match key {
        0x2a | 0x36 => {
            unsafe {
                SHIFT_PRESSED = !released;
            }
            return;
        }
        0x3a if !released => {
            unsafe {
                CAPS_LOCK = !CAPS_LOCK;
            }
            return;
        }
        _ if released => return,
        _ => {}
    }

    let shifted = unsafe { SHIFT_PRESSED };
    let caps_lock = unsafe { CAPS_LOCK };
    if let Some(mut character) = scan_code_to_ascii(key, shifted) {
        if character.is_ascii_lowercase() {
            if shifted ^ caps_lock {
                character = character.to_ascii_uppercase();
            }
        } else if caps_lock && character.is_ascii_uppercase() && !shifted {
            character = character.to_ascii_lowercase();
        }
        let _ = keyboard::push_character(character);
    }
}

fn scan_code_to_ascii(scan_code: u8, shifted: bool) -> Option<u8> {
    let character = match scan_code {
        0x02 => {
            if shifted {
                b'!'
            } else {
                b'1'
            }
        }
        0x03 => {
            if shifted {
                b'@'
            } else {
                b'2'
            }
        }
        0x04 => {
            if shifted {
                b'#'
            } else {
                b'3'
            }
        }
        0x05 => {
            if shifted {
                b'$'
            } else {
                b'4'
            }
        }
        0x06 => {
            if shifted {
                b'%'
            } else {
                b'5'
            }
        }
        0x07 => {
            if shifted {
                b'^'
            } else {
                b'6'
            }
        }
        0x08 => {
            if shifted {
                b'&'
            } else {
                b'7'
            }
        }
        0x09 => {
            if shifted {
                b'*'
            } else {
                b'8'
            }
        }
        0x0a => {
            if shifted {
                b'('
            } else {
                b'9'
            }
        }
        0x0b => {
            if shifted {
                b')'
            } else {
                b'0'
            }
        }
        0x0c => {
            if shifted {
                b'_'
            } else {
                b'-'
            }
        }
        0x0d => {
            if shifted {
                b'+'
            } else {
                b'='
            }
        }
        0x0e => 0x08,
        0x10 => b'q',
        0x11 => b'w',
        0x12 => b'e',
        0x13 => b'r',
        0x14 => b't',
        0x15 => b'y',
        0x16 => b'u',
        0x17 => b'i',
        0x18 => b'o',
        0x19 => b'p',
        0x1a => {
            if shifted {
                b'{'
            } else {
                b'['
            }
        }
        0x1b => {
            if shifted {
                b'}'
            } else {
                b']'
            }
        }
        0x1c => b'\n',
        0x1e => b'a',
        0x1f => b's',
        0x20 => b'd',
        0x21 => b'f',
        0x22 => b'g',
        0x23 => b'h',
        0x24 => b'j',
        0x25 => b'k',
        0x26 => b'l',
        0x27 => {
            if shifted {
                b':'
            } else {
                b';'
            }
        }
        0x28 => {
            if shifted {
                b'"'
            } else {
                b'\''
            }
        }
        0x29 => {
            if shifted {
                b'~'
            } else {
                b'`'
            }
        }
        0x2b => {
            if shifted {
                b'|'
            } else {
                b'\\'
            }
        }
        0x2c => b'z',
        0x2d => b'x',
        0x2e => b'c',
        0x2f => b'v',
        0x30 => b'b',
        0x31 => b'n',
        0x32 => b'm',
        0x33 => {
            if shifted {
                b'<'
            } else {
                b','
            }
        }
        0x34 => {
            if shifted {
                b'>'
            } else {
                b'.'
            }
        }
        0x35 => {
            if shifted {
                b'?'
            } else {
                b'/'
            }
        }
        0x39 => b' ',
        _ => return None,
    };

    Some(character)
}

fn clear_screen() {
    for cell in 0..VGA_CELLS {
        unsafe {
            write_volatile(VGA_BUFFER.add(cell), COLOR | b' ' as u16);
        }
    }
    unsafe {
        CURSOR = 0;
    }
}

fn write_str(message: &str) {
    for byte in message.bytes() {
        write_byte(byte);
    }
}

fn write_byte(byte: u8) {
    scroll_if_needed();

    let cursor = unsafe { CURSOR };
    match byte {
        b'\n' => {
            unsafe {
                CURSOR = (cursor / VGA_WIDTH + 1) * VGA_WIDTH;
            }
            scroll_if_needed();
        }
        0x08 => {
            if cursor > 0 {
                let previous = cursor - 1;
                unsafe {
                    CURSOR = previous;
                    write_volatile(VGA_BUFFER.add(previous), COLOR | b' ' as u16);
                }
            }
        }
        _ => unsafe {
            write_volatile(VGA_BUFFER.add(cursor), COLOR | byte as u16);
            CURSOR = cursor + 1;
        },
    }
}

fn scroll_if_needed() {
    while unsafe { CURSOR } >= VGA_CELLS {
        for cell in 0..(VGA_CELLS - VGA_WIDTH) {
            unsafe {
                let value = read_volatile(VGA_BUFFER.add(cell + VGA_WIDTH));
                write_volatile(VGA_BUFFER.add(cell), value);
            }
        }
        for cell in (VGA_CELLS - VGA_WIDTH)..VGA_CELLS {
            unsafe {
                write_volatile(VGA_BUFFER.add(cell), COLOR | b' ' as u16);
            }
        }
        unsafe {
            CURSOR -= VGA_WIDTH;
        }
    }
}

unsafe fn read_port(port: u16) -> u8 {
    // SAFETY: callers select a documented device/PIC port and execute at CPL 0.
    let value: u8;
    core::arch::asm!(
        "in al, dx",
        in("dx") port,
        out("al") value,
        options(nomem, nostack, preserves_flags)
    );
    value
}

unsafe fn write_port(port: u16, value: u8) {
    // SAFETY: callers select a documented device/PIC port and execute at CPL 0.
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

unsafe fn io_wait() {
    // SAFETY: port 0x80 is used as the legacy chipset I/O delay port.
    write_port(0x80, 0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
