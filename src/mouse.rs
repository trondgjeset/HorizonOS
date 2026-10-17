use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU8, Ordering};

const DATA_PORT: u16 = 0x60;
const STATUS_PORT: u16 = 0x64;
const MOUSE_PACKET_SIZE: usize = 3;
const STATUS_OUTPUT_FULL: u8 = 1;
const STATUS_INPUT_FULL: u8 = 2;
const STATUS_AUXILIARY: u8 = 1 << 5;
const MOUSE_SYNC_BIT: u8 = 1 << 3;
const MOUSE_OVERFLOW_X: u8 = 1 << 6;
const MOUSE_OVERFLOW_Y: u8 = 1 << 7;
const POLL_LIMIT: usize = 100_000;
/// Per-axis pointer tolerance for treating two presses as the same location.
///
/// NOTE: Open-source Contribution Area: tune only alongside icon hit testing.
const DOUBLE_CLICK_DISTANCE: usize = 8;

/// Maximum elapsed PIT ticks between a matching pair of pointer clicks.
///
/// NOTE: Open-source Contribution Area: tune this interval with desktop input
/// testing and keep it consistent with the shell's 100 Hz timer.
pub const DOUBLE_CLICK_TICKS: u32 = 45;

static MOUSE_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_Y: AtomicI32 = AtomicI32::new(0);
static MAX_X: AtomicI32 = AtomicI32::new(0);
static MAX_Y: AtomicI32 = AtomicI32::new(0);
static BUTTONS: AtomicU8 = AtomicU8::new(0);
static DIRTY: AtomicU8 = AtomicU8::new(0);
static LEFT_PRESS_PENDING: AtomicU8 = AtomicU8::new(0);
static LEFT_PRESS_TICK: AtomicU32 = AtomicU32::new(0);
static mut PACKET: [u8; MOUSE_PACKET_SIZE] = [0; MOUSE_PACKET_SIZE];
static mut PACKET_INDEX: usize = 0;

#[derive(Clone, Copy)]
/// Snapshot of the bounded pointer position, buttons, and pending click time.
pub struct MouseState {
    pub x: usize,
    pub y: usize,
    pub buttons: u8,
    pub left_pressed_at: Option<u32>,
}

/// Tracks two nearby pointer presses and safely handles PIT counter rollover.
///
/// NOTE: Open-source Contribution Area: add per-device timing and movement
/// policies when multiple pointer devices are supported.
#[derive(Default)]
pub struct DoubleClickTracker {
    previous: Option<(usize, usize, u32)>,
}

impl DoubleClickTracker {
    /// Records a press and reports whether it matches the preceding click.
    ///
    /// Coordinates may move by at most [`DOUBLE_CLICK_DISTANCE`] pixels on
    /// either axis; callers still verify that both hits identify the same UI
    /// element before dispatching an action.
    ///
    /// NOTE: Open-source Contribution Area: update the spatial tolerance only
    /// alongside matching hit-test semantics.
    pub fn register_press(&mut self, x: usize, y: usize, tick: u32) -> bool {
        let is_double = self
            .previous
            .is_some_and(|(previous_x, previous_y, previous_tick)| {
                x.abs_diff(previous_x) <= DOUBLE_CLICK_DISTANCE
                    && y.abs_diff(previous_y) <= DOUBLE_CLICK_DISTANCE
                    && tick.wrapping_sub(previous_tick) <= DOUBLE_CLICK_TICKS
            });
        if is_double {
            self.previous = None;
        } else {
            self.previous = Some((x, y, tick));
        }
        is_double
    }

    /// Discards pending click history when a press is outside a target.
    ///
    /// NOTE: Open-source Contribution Area: preserve explicit reset behavior
    /// if press tracking later becomes shared between multiple widgets.
    pub fn clear(&mut self) {
        self.previous = None;
    }
}

/// Enables the PS/2 auxiliary device and bounds pointer coordinates to a screen.
///
/// `width` and `height` are framebuffer dimensions. Returns an error when the
/// dimensions are invalid or the controller does not respond before timeout.
pub fn initialize(width: usize, height: usize) -> Result<(), &'static str> {
    if width == 0 || height == 0 || width > i32::MAX as usize || height > i32::MAX as usize {
        return Err("invalid mouse coordinate bounds");
    }

    wait_input_empty()?;
    unsafe {
        write_port(STATUS_PORT, 0xa8);
    }

    wait_input_empty()?;
    unsafe {
        write_port(STATUS_PORT, 0x20);
    }
    let mut command_byte = wait_output_full()?;
    command_byte |= 0x03;
    command_byte &= !0x20;

    wait_input_empty()?;
    unsafe {
        write_port(STATUS_PORT, 0x60);
    }
    wait_input_empty()?;
    unsafe {
        write_port(DATA_PORT, command_byte);
    }

    send_mouse_command(0xf6)?;
    send_mouse_command(0xf4)?;

    MOUSE_X.store((width / 2) as i32, Ordering::Relaxed);
    MOUSE_Y.store((height / 2) as i32, Ordering::Relaxed);
    MAX_X.store((width - 1) as i32, Ordering::Relaxed);
    MAX_Y.store((height - 1) as i32, Ordering::Relaxed);
    BUTTONS.store(0, Ordering::Relaxed);
    DIRTY.store(0, Ordering::Relaxed);
    LEFT_PRESS_PENDING.store(0, Ordering::Relaxed);
    LEFT_PRESS_TICK.store(0, Ordering::Relaxed);
    unsafe {
        PACKET_INDEX = 0;
    }
    Ok(())
}

/// Returns the latest pointer position and button state.
pub fn snapshot() -> MouseState {
    MouseState {
        x: MOUSE_X.load(Ordering::Acquire) as usize,
        y: MOUSE_Y.load(Ordering::Acquire) as usize,
        buttons: BUTTONS.load(Ordering::Acquire),
        left_pressed_at: None,
    }
}

/// Consumes the timestamp of the most recent left-button press, if any.
pub fn take_left_press() -> Option<u32> {
    if LEFT_PRESS_PENDING.swap(0, Ordering::AcqRel) == 0 {
        return None;
    }
    Some(LEFT_PRESS_TICK.load(Ordering::Acquire))
}

/// Clears and reports the pointer driver's redraw-dirty flag.
pub fn take_dirty() -> bool {
    DIRTY.swap(0, Ordering::AcqRel) != 0
}

/// Reports whether mouse state changed since the event loop last acknowledged it.
pub fn has_pending_event() -> bool {
    DIRTY.load(Ordering::Acquire) != 0
}

#[no_mangle]
/// Drains one PS/2 auxiliary byte, updates packet state, and acknowledges both PICs.
///
/// # Safety
/// The IDT must route IRQ12 here, and the PS/2 controller and PIC must have been
/// initialized so reading the status/data ports is valid.
pub extern "C" fn rust_irq12_handler() {
    let status = unsafe { read_port(STATUS_PORT) };
    if status & STATUS_OUTPUT_FULL != 0 && status & STATUS_AUXILIARY != 0 {
        let value = unsafe { read_port(DATA_PORT) };
        consume_mouse_byte(value);
    }

    unsafe {
        write_port(0xa0, 0x20);
        write_port(0x20, 0x20);
    }
}

fn consume_mouse_byte(value: u8) {
    // The synchronization bit lets us discard stray bytes until packet byte 0;
    // complete packets are processed only after all three bytes arrive.
    let packet = core::ptr::addr_of_mut!(PACKET).cast::<u8>();
    let index_pointer = core::ptr::addr_of_mut!(PACKET_INDEX);
    let index = unsafe { *index_pointer };

    if index == 0 && value & MOUSE_SYNC_BIT == 0 {
        return;
    }
    unsafe {
        packet.add(index).write(value);
        *index_pointer = index + 1;
    }

    if index + 1 == MOUSE_PACKET_SIZE {
        unsafe {
            *index_pointer = 0;
        }
        let flags = unsafe { packet.read() };
        let delta_x = unsafe { packet.add(1).read() };
        let delta_y = unsafe { packet.add(2).read() };
        process_packet(flags, delta_x, delta_y);
    }
}

fn process_packet(flags: u8, delta_x_byte: u8, delta_y_byte: u8) {
    let buttons = flags & 0x07;
    let previous_buttons = BUTTONS.swap(buttons, Ordering::AcqRel);
    if buttons & 1 != 0 && previous_buttons & 1 == 0 {
        record_left_press(current_tick());
    }

    if flags & (MOUSE_OVERFLOW_X | MOUSE_OVERFLOW_Y) != 0 {
        DIRTY.store(1, Ordering::Release);
        return;
    }

    // PS/2 deltas are signed 8-bit values; screen coordinates invert the
    // device's positive-up Y axis and are clamped atomically to valid bounds.
    let delta_x = delta_x_byte as i8 as i32;
    let delta_y = delta_y_byte as i8 as i32;
    update_coordinate(&MOUSE_X, delta_x, &MAX_X);
    update_coordinate(&MOUSE_Y, -delta_y, &MAX_Y);
    DIRTY.store(1, Ordering::Release);
}

#[cfg(not(test))]
fn current_tick() -> u32 {
    crate::clock::ticks()
}

#[cfg(test)]
fn current_tick() -> u32 {
    0
}

fn record_left_press(tick: u32) {
    LEFT_PRESS_TICK.store(tick, Ordering::Relaxed);
    LEFT_PRESS_PENDING.store(1, Ordering::Release);
}

fn update_coordinate(coordinate: &AtomicI32, delta: i32, maximum: &AtomicI32) {
    let maximum = maximum.load(Ordering::Relaxed);
    let mut current = coordinate.load(Ordering::Acquire);
    loop {
        let updated = current.saturating_add(delta).clamp(0, maximum);
        match coordinate.compare_exchange_weak(
            current,
            updated,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

fn send_mouse_command(command: u8) -> Result<(), &'static str> {
    wait_input_empty()?;
    unsafe {
        write_port(STATUS_PORT, 0xd4);
    }
    wait_input_empty()?;
    unsafe {
        write_port(DATA_PORT, command);
    }

    let response = wait_output_full()?;
    if response != 0xfa {
        return Err("PS/2 mouse did not acknowledge command");
    }
    Ok(())
}

fn wait_input_empty() -> Result<(), &'static str> {
    for _ in 0..POLL_LIMIT {
        if unsafe { read_port(STATUS_PORT) } & STATUS_INPUT_FULL == 0 {
            return Ok(());
        }
    }
    Err("PS/2 controller input buffer timed out")
}

fn wait_output_full() -> Result<u8, &'static str> {
    for _ in 0..POLL_LIMIT {
        if unsafe { read_port(STATUS_PORT) } & STATUS_OUTPUT_FULL != 0 {
            return Ok(unsafe { read_port(DATA_PORT) });
        }
    }
    Err("PS/2 controller response timed out")
}

unsafe fn read_port(port: u16) -> u8 {
    // SAFETY: the caller supplies a controller port and runs with I/O privilege.
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
    // SAFETY: the caller supplies a controller port and runs with I/O privilege.
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

#[cfg(test)]
mod tests {
    use super::{
        process_packet, record_left_press, take_left_press, DoubleClickTracker, BUTTONS,
        LEFT_PRESS_PENDING,
    };
    use core::sync::atomic::Ordering;

    #[test]
    fn short_left_press_is_latched_until_consumed() {
        BUTTONS.store(0, Ordering::Relaxed);
        LEFT_PRESS_PENDING.store(0, Ordering::Relaxed);

        record_left_press(17);

        assert_eq!(take_left_press(), Some(17));
        assert_eq!(take_left_press(), None);
    }

    #[test]
    fn repeated_press_after_release_is_latched_separately() {
        BUTTONS.store(0, Ordering::Relaxed);
        LEFT_PRESS_PENDING.store(0, Ordering::Relaxed);

        record_left_press(17);
        assert_eq!(take_left_press(), Some(17));
        record_left_press(22);

        assert_eq!(take_left_press(), Some(22));
    }

    #[test]
    fn button_release_does_not_create_a_left_press() {
        BUTTONS.store(1, Ordering::Relaxed);
        LEFT_PRESS_PENDING.store(0, Ordering::Relaxed);

        process_packet(0, 0, 0);

        assert_eq!(take_left_press(), None);
    }

    #[test]
    /// Verifies location, timing, and wrapping behavior for click pairs.
    fn double_click_tracker_checks_location_time_and_tick_wrap() {
        let mut tracker = DoubleClickTracker::default();
        assert!(!tracker.register_press(10, 20, u32::MAX - 10));
        assert!(tracker.register_press(14, 24, 10));
        assert!(!tracker.register_press(10, 20, 30));
        assert!(!tracker.register_press(40, 20, 31));
        assert!(!tracker.register_press(40, 20, 80));
    }
}
