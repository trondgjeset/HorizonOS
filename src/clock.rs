use core::sync::atomic::{AtomicU32, Ordering};

const PIT_COMMAND: u16 = 0x43;
const PIT_CHANNEL_0: u16 = 0x40;
const PIC1_COMMAND: u16 = 0x20;
const CMOS_INDEX: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;
const CMOS_STATUS_A: u8 = 0x0a;
const CMOS_STATUS_B: u8 = 0x0b;
const RTC_UPDATE_IN_PROGRESS: u8 = 0x80;
const RTC_24_HOUR: u8 = 1 << 1;
const RTC_BINARY_MODE: u8 = 1 << 2;
const RTC_PM_FLAG: u8 = 1 << 7;
const RTC_SNAPSHOT_ATTEMPTS: usize = 8;
const PIT_INPUT_HZ: u32 = 1_193_182;
const PIT_TICK_HZ: u32 = 100;

static TICKS: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
/// Calendar date and wall-clock time read from the CMOS real-time clock.
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Raw, mutually consistent CMOS values captured before decoding.
struct RtcSnapshot {
    second: u8,
    minute: u8,
    hour: u8,
    day: u8,
    month: u8,
    year: u8,
    century: u8,
    mode: u8,
}

/// Programs PIT channel 0 for the kernel's 100 Hz scheduling clock.
pub fn initialize() {
    let divisor = ((PIT_INPUT_HZ + PIT_TICK_HZ / 2) / PIT_TICK_HZ) as u16;
    unsafe {
        write_port(PIT_COMMAND, 0x36);
        write_port(PIT_CHANNEL_0, divisor as u8);
        write_port(PIT_CHANNEL_0, (divisor >> 8) as u8);
    }
}

/// Accounts for one timer interrupt and acknowledges the master PIC.
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe {
        write_port(PIC1_COMMAND, 0x20);
    }
}

/// Returns elapsed whole seconds since the PIT was initialized.
pub fn seconds() -> u32 {
    TICKS.load(Ordering::Acquire) / 100
}

/// Returns the number of PIT ticks since initialization.
pub fn ticks() -> u32 {
    TICKS.load(Ordering::Acquire)
}

/// Reads a stable RTC snapshot and converts it to a validated calendar value.
///
/// Returns an error if the CMOS clock is updating continuously or reports
/// invalid date/time fields.
pub fn read_datetime() -> Result<DateTime, &'static str> {
    let mut previous = None;
    for _ in 0..RTC_SNAPSHOT_ATTEMPTS {
        let snapshot = read_snapshot()?;
        if previous == Some(snapshot) {
            return convert_snapshot(snapshot);
        }
        previous = Some(snapshot);
    }
    Err("RTC did not produce a stable date and time")
}

fn read_snapshot() -> Result<RtcSnapshot, &'static str> {
    // Read status A both before and after the fields: if an update began during
    // the read, discard the mixed snapshot and retry.
    for _ in 0..100_000 {
        if read_register(CMOS_STATUS_A) & RTC_UPDATE_IN_PROGRESS == 0 {
            let snapshot = RtcSnapshot {
                second: read_register(0x00),
                minute: read_register(0x02),
                hour: read_register(0x04),
                day: read_register(0x07),
                month: read_register(0x08),
                year: read_register(0x09),
                century: read_register(0x32),
                mode: read_register(CMOS_STATUS_B),
            };
            if read_register(CMOS_STATUS_A) & RTC_UPDATE_IN_PROGRESS == 0 {
                return Ok(snapshot);
            }
        }
    }
    Err("RTC update remained in progress")
}

fn convert_snapshot(snapshot: RtcSnapshot) -> Result<DateTime, &'static str> {
    // CMOS can expose either packed BCD or binary values and either 12-hour or
    // 24-hour time; normalize both encodings before validating field ranges.
    let binary_mode = snapshot.mode & RTC_BINARY_MODE != 0;
    let is_24_hour = snapshot.mode & RTC_24_HOUR != 0;
    let pm = snapshot.hour & RTC_PM_FLAG != 0;
    let hour_raw = snapshot.hour & !RTC_PM_FLAG;
    let second = decode(snapshot.second, binary_mode);
    let minute = decode(snapshot.minute, binary_mode);
    let mut hour = decode(hour_raw, binary_mode);
    let day = decode(snapshot.day, binary_mode);
    let month = decode(snapshot.month, binary_mode);
    let year = decode(snapshot.year, binary_mode) as u16;
    let century = decode(snapshot.century, binary_mode) as u16;

    if !is_24_hour {
        if !(1..=12).contains(&hour) {
            return Err("RTC hour is outside the 12-hour range");
        }
        hour %= 12;
        if pm {
            hour += 12;
        }
    }

    if second > 59
        || minute > 59
        || hour > 23
        || !(1..=31).contains(&day)
        || !(1..=12).contains(&month)
    {
        return Err("RTC returned an invalid date or time");
    }

    let full_year = if (19..=99).contains(&century) {
        century * 100 + year
    } else {
        2000 + year
    };
    Ok(DateTime {
        year: full_year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

fn decode(value: u8, binary_mode: bool) -> u8 {
    if binary_mode {
        value
    } else {
        (value & 0x0f) + ((value >> 4) * 10)
    }
}

fn read_register(index: u8) -> u8 {
    unsafe {
        write_port(CMOS_INDEX, index);
        read_port(CMOS_DATA)
    }
}

unsafe fn read_port(port: u16) -> u8 {
    // SAFETY: callers pass an x86 I/O port supported by this kernel's chipset
    // initialization path; executing IN requires ring 0.
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
    // SAFETY: callers pass a chipset/PIT/CMOS port and execute at ring 0.
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

#[cfg(test)]
mod tests {
    use super::{convert_snapshot, RtcSnapshot};

    #[test]
    fn converts_bcd_24_hour_rtc_values() {
        let datetime = convert_snapshot(RtcSnapshot {
            second: 0x47,
            minute: 0x23,
            hour: 0x21,
            day: 0x17,
            month: 0x10,
            year: 0x26,
            century: 0x20,
            mode: super::RTC_24_HOUR,
        })
        .unwrap();

        assert_eq!(datetime.year, 2026);
        assert_eq!(datetime.month, 10);
        assert_eq!(datetime.day, 17);
        assert_eq!(datetime.hour, 21);
        assert_eq!(datetime.minute, 23);
        assert_eq!(datetime.second, 47);
    }

    #[test]
    fn converts_12_hour_pm_values() {
        let datetime = convert_snapshot(RtcSnapshot {
            second: 0x00,
            minute: 0x05,
            hour: 0x89,
            day: 0x01,
            month: 0x01,
            year: 0x00,
            century: 0x00,
            mode: 0,
        })
        .unwrap();

        assert_eq!(datetime.hour, 21);
    }

    #[test]
    fn rejects_invalid_rtc_fields() {
        let invalid = RtcSnapshot {
            second: 0x00,
            minute: 0x00,
            hour: 0x24,
            day: 0x01,
            month: 0x01,
            year: 0x26,
            century: 0x20,
            mode: super::RTC_24_HOUR,
        };

        assert!(convert_snapshot(invalid).is_err());
    }
}
