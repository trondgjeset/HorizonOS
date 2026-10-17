use core::ptr::{copy_nonoverlapping, read_unaligned};

use crate::font8x8::FONT8X8_BASIC;

const MULTIBOOT_INFO_FRAMEBUFFER: u32 = 1 << 12;
const MULTIBOOT_FRAMEBUFFER_RGB: u8 = 1;
const MAX_BACKBUFFER_PIXELS: usize = 1024 * 1024;
const CURSOR_WIDTH: usize = 12;
const CURSOR_HEIGHT: usize = 18;
const CURSOR_PIXELS: usize = CURSOR_WIDTH * CURSOR_HEIGHT;
const CURSOR_SHAPE: [u16; CURSOR_HEIGHT] = [
    0b0000_0000_0001,
    0b0000_0000_0011,
    0b0000_0000_0111,
    0b0000_0000_1111,
    0b0000_0001_1111,
    0b0000_0011_1111,
    0b0000_0111_1111,
    0b0000_1111_1111,
    0b0001_1111_1111,
    0b0000_0011_1111,
    0b0000_0011_0111,
    0b0000_0110_0011,
    0b0000_0110_0011,
    0b0000_1100_0011,
    0b0000_1100_0001,
    0b0001_1000_0001,
    0b0001_1000_0000,
    0b0011_0000_0000,
];

#[derive(Clone, Copy)]
/// Bit position and width of one component in the bootloader's pixel format.
struct Channel {
    position: u8,
    size: u8,
}

#[derive(Clone, Copy)]
/// Validated framebuffer metadata plus the kernel back-buffer pointer.
struct Framebuffer {
    address: *mut u8,
    pitch: usize,
    width: usize,
    height: usize,
    red: Channel,
    green: Channel,
    blue: Channel,
    backbuffer: *mut u32,
}

static mut FRAMEBUFFER: Option<Framebuffer> = None;
static mut BACKBUFFER: [u32; MAX_BACKBUFFER_PIXELS] = [0; MAX_BACKBUFFER_PIXELS];
static mut CURSOR_BACKUP: [u32; CURSOR_PIXELS] = [0; CURSOR_PIXELS];
static mut PREVIOUS_CURSOR: Option<(usize, usize, usize, usize)> = None;

/// Validates Multiboot framebuffer metadata, maps device memory, and selects a back buffer.
///
/// `multiboot_info_address` is the physical/identity-mapped Multiboot structure
/// address. Returns an error for unsupported pixel formats, invalid dimensions,
/// or a framebuffer mapping that exceeds reserved resources.
///
/// # Safety
/// The bootloader information address must point to a valid Multiboot structure
/// in mapped memory, and its framebuffer fields must describe the actual device.
pub fn initialize(multiboot_info_address: u32) -> Result<(), &'static str> {
    let info = multiboot_info_address as *const u8;
    let flags = unsafe { read_unaligned(info.cast::<u32>()) };
    if flags & MULTIBOOT_INFO_FRAMEBUFFER == 0 {
        return Err("GRUB did not provide framebuffer information");
    }

    // Multiboot v1 places the framebuffer address, pitch, dimensions, and RGB
    // channel descriptors at fixed offsets in the variable-length info record.
    let physical_address = unsafe { read_unaligned(info.add(88).cast::<u64>()) };
    let pitch = unsafe { read_unaligned(info.add(96).cast::<u32>()) } as usize;
    let width = unsafe { read_unaligned(info.add(100).cast::<u32>()) } as usize;
    let height = unsafe { read_unaligned(info.add(104).cast::<u32>()) } as usize;
    let bits_per_pixel = unsafe { read_unaligned(info.add(108)) };
    let framebuffer_type = unsafe { read_unaligned(info.add(109)) };

    if bits_per_pixel != 32 || framebuffer_type != MULTIBOOT_FRAMEBUFFER_RGB {
        return Err("only 32-bit RGB framebuffers are supported");
    }
    if width == 0 || height == 0 {
        return Err("framebuffer dimensions are zero");
    }

    let red = read_channel(info, 112)?;
    let green = read_channel(info, 114)?;
    let blue = read_channel(info, 116)?;
    validate_channels(red, green, blue)?;

    let minimum_pitch = width.checked_mul(4).ok_or("framebuffer width overflow")?;
    if pitch < minimum_pitch {
        return Err("framebuffer pitch is smaller than one pixel row");
    }
    let buffer_size = pitch
        .checked_mul(height)
        .ok_or("framebuffer size overflow")?;
    let pixel_count = width
        .checked_mul(height)
        .ok_or("framebuffer pixel count overflow")?;
    if pixel_count > MAX_BACKBUFFER_PIXELS {
        return Err("framebuffer exceeds backbuffer capacity");
    }
    let address = crate::memory::map_framebuffer(physical_address, buffer_size)?;
    let backbuffer = core::ptr::addr_of_mut!(BACKBUFFER).cast::<u32>();

    unsafe {
        FRAMEBUFFER = Some(Framebuffer {
            address,
            pitch,
            width,
            height,
            red,
            green,
            blue,
            backbuffer,
        });
        PREVIOUS_CURSOR = None;
    }
    Ok(())
}

/// Returns the initialized framebuffer's width and height in pixels.
pub fn dimensions() -> Result<(usize, usize), &'static str> {
    let framebuffer = current_framebuffer()?;
    Ok((framebuffer.width, framebuffer.height))
}

/// Writes one RGBA-formatted pixel into the back buffer.
///
/// `x` and `y` are pixel coordinates; `color` is `0xRRGGBBAA`. Returns an error
/// when graphics is not initialized or the coordinates are out of bounds.
pub fn put_pixel(x: usize, y: usize, color: u32) -> Result<(), &'static str> {
    let framebuffer = current_framebuffer()?;
    if x >= framebuffer.width || y >= framebuffer.height {
        return Err("pixel coordinates are outside the framebuffer");
    }
    put_pixel_in(&framebuffer, x, y, color)?;
    Ok(())
}

/// Fills the visible back-buffer area with one RGBA color.
///
/// Returns an error if graphics has not been initialized.
pub fn clear_screen(color: u32) -> Result<(), &'static str> {
    let framebuffer = current_framebuffer()?;
    let packed = pack_color(color, &framebuffer);
    let count = framebuffer.width * framebuffer.height;
    for index in 0..count {
        unsafe {
            framebuffer.backbuffer.add(index).write(packed);
        }
    }
    unsafe {
        PREVIOUS_CURSOR = None;
    }
    Ok(())
}

/// Draws a clipped rectangle into the back buffer.
///
/// `x`, `y`, `width`, and `height` define its upper-left origin and extent;
/// `color` is `0xRRGGBBAA`. Returns an error if graphics is not initialized.
pub fn draw_rectangle(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: u32,
) -> Result<(), &'static str> {
    let framebuffer = current_framebuffer()?;
    let left = x.min(framebuffer.width);
    let top = y.min(framebuffer.height);
    let right = x.saturating_add(width).min(framebuffer.width);
    let bottom = y.saturating_add(height).min(framebuffer.height);

    for row in top..bottom {
        for column in left..right {
            put_pixel_in(&framebuffer, column, row, color)?;
        }
    }
    Ok(())
}

/// Copies a source pixel buffer into the clipped back-buffer region.
///
/// `buffer_width`/`buffer_height` describe `pixels`; `clip_width`/`clip_height`
/// cap the drawn extent. `x`/`y` select the destination origin. Returns an error
/// for inconsistent source dimensions or an unavailable framebuffer. Callers
/// must keep each clip dimension within its source-buffer dimension; a larger
/// visible extent would index beyond `pixels`.
pub fn draw_buffer(
    x: usize,
    y: usize,
    buffer_width: usize,
    buffer_height: usize,
    clip_width: usize,
    clip_height: usize,
    pixels: &[u32],
) -> Result<(), &'static str> {
    let required_pixels = buffer_width
        .checked_mul(buffer_height)
        .ok_or("buffer dimensions overflow")?;
    if pixels.len() < required_pixels {
        return Err("pixel buffer is smaller than its dimensions");
    }

    let framebuffer = current_framebuffer()?;
    let right = x.saturating_add(clip_width).min(framebuffer.width);
    let bottom = y.saturating_add(clip_height).min(framebuffer.height);
    if x >= right || y >= bottom {
        return Ok(());
    }

    for destination_y in y..bottom {
        let source_row = (destination_y - y) * buffer_width;
        for destination_x in x..right {
            let color = pixels[source_row + destination_x - x];
            put_pixel_in(&framebuffer, destination_x, destination_y, color)?;
        }
    }
    Ok(())
}

/// Renders ASCII text with the embedded 8x8 font into the back buffer.
///
/// `x` and `y` are the text origin and `color` is `0xRRGGBBAA`. Newlines,
/// carriage returns, and tabs are handled; drawing stops at the screen edge.
/// Returns an error if graphics is uninitialized.
pub fn draw_string(x: usize, y: usize, text: &str, color: u32) -> Result<(), &'static str> {
    let framebuffer = current_framebuffer()?;
    let origin_x = x;
    let mut cursor_x = x;
    let mut cursor_y = y;

    for byte in text.bytes() {
        match byte {
            b'\n' => {
                cursor_x = origin_x;
                cursor_y = cursor_y.saturating_add(8);
                if cursor_y >= framebuffer.height {
                    break;
                }
                continue;
            }
            b'\r' => continue,
            b'\t' => {
                cursor_x = cursor_x.saturating_add(32);
                continue;
            }
            _ => {}
        }

        if cursor_x >= framebuffer.width || cursor_y >= framebuffer.height {
            break;
        }
        let glyph_index = if byte.is_ascii() {
            byte as usize
        } else {
            b'?' as usize
        };
        for (row, bits) in FONT8X8_BASIC[glyph_index].iter().copied().enumerate() {
            let pixel_y = cursor_y.saturating_add(row);
            if pixel_y >= framebuffer.height {
                break;
            }
            for column in 0..8 {
                if bits & (1 << column) == 0 {
                    continue;
                }
                let pixel_x = cursor_x.saturating_add(column);
                if pixel_x < framebuffer.width {
                    put_pixel_in(&framebuffer, pixel_x, pixel_y, color)?;
                }
            }
        }
        cursor_x = cursor_x.saturating_add(8);
    }
    Ok(())
}

/// Draws the mouse cursor over the back buffer, preserving the covered pixels.
///
/// `x` and `y` are the cursor hotspot and `buttons` selects the pressed style.
/// Returns an error if graphics has not been initialized.
pub fn draw_cursor(x: usize, y: usize, buttons: u8) -> Result<(), &'static str> {
    let framebuffer = current_framebuffer()?;
    unsafe {
        if let Some((previous_x, previous_y, previous_width, previous_height)) = PREVIOUS_CURSOR {
            restore_cursor(
                &framebuffer,
                previous_x,
                previous_y,
                previous_width,
                previous_height,
            );
        }

        let left = x.min(framebuffer.width);
        let top = y.min(framebuffer.height);
        let width = CURSOR_WIDTH.min(framebuffer.width.saturating_sub(left));
        let height = CURSOR_HEIGHT.min(framebuffer.height.saturating_sub(top));
        save_cursor(&framebuffer, left, top, width, height);

        let foreground = if buttons & 1 != 0 {
            0xffd166ff
        } else {
            0xffffffff
        };
        let black = pack_color(0x000000ff, &framebuffer);
        let white = pack_color(foreground, &framebuffer);

        for row in 0..height {
            for column in 0..width {
                if CURSOR_SHAPE[row] & (1 << column) == 0 {
                    continue;
                }

                let edge = (column > 0 && CURSOR_SHAPE[row] & (1 << (column - 1)) == 0)
                    || (column + 1 < CURSOR_WIDTH && CURSOR_SHAPE[row] & (1 << (column + 1)) == 0)
                    || (row > 0 && CURSOR_SHAPE[row - 1] & (1 << column) == 0)
                    || (row + 1 < CURSOR_HEIGHT && CURSOR_SHAPE[row + 1] & (1 << column) == 0);
                framebuffer
                    .backbuffer
                    .add((top + row) * framebuffer.width + left + column)
                    .write(if edge { black } else { white });
            }
        }
        PREVIOUS_CURSOR = Some((left, top, width, height));
    }
    Ok(())
}

/// Copies the complete back buffer to the mapped linear framebuffer.
///
/// Returns an error if graphics has not been initialized.
pub fn flush() -> Result<(), &'static str> {
    let framebuffer = current_framebuffer()?;
    // TODO: Optimization Opportunity: benchmark row copies against SIMD or
    // dirty-rectangle flushing once the compositor tracks changed regions.
    for row in 0..framebuffer.height {
        let source = unsafe { framebuffer.backbuffer.add(row * framebuffer.width) };
        let destination = unsafe {
            framebuffer
                .address
                .add(row * framebuffer.pitch)
                .cast::<u32>()
        };
        unsafe {
            copy_nonoverlapping(source, destination, framebuffer.width);
        }
    }
    Ok(())
}

fn read_channel(info: *const u8, offset: usize) -> Result<Channel, &'static str> {
    let position = unsafe { read_unaligned(info.add(offset)) };
    let size = unsafe { read_unaligned(info.add(offset + 1)) };
    if size == 0 {
        return Err("framebuffer color channel has zero width");
    }
    if size > 8 || position as u16 + size as u16 > 32 {
        return Err("invalid framebuffer color channel layout");
    }
    Ok(Channel { position, size })
}

fn validate_channels(red: Channel, green: Channel, blue: Channel) -> Result<(), &'static str> {
    let red_mask = channel_mask(red);
    let green_mask = channel_mask(green);
    let blue_mask = channel_mask(blue);
    if red_mask & green_mask != 0 || red_mask & blue_mask != 0 || green_mask & blue_mask != 0 {
        return Err("overlapping framebuffer color channels");
    }
    Ok(())
}

fn channel_mask(channel: Channel) -> u32 {
    ((1u32 << channel.size) - 1) << channel.position
}

fn current_framebuffer() -> Result<Framebuffer, &'static str> {
    unsafe { FRAMEBUFFER.ok_or("framebuffer graphics is not initialized") }
}

fn put_pixel_in(
    framebuffer: &Framebuffer,
    x: usize,
    y: usize,
    color: u32,
) -> Result<(), &'static str> {
    if x >= framebuffer.width || y >= framebuffer.height {
        return Err("pixel coordinates are outside the framebuffer");
    }
    let pixel = unsafe { framebuffer.backbuffer.add(y * framebuffer.width + x) };
    // The back buffer stores channels in the hardware layout. For translucent
    // drawing, decode the old pixel to normalized RGB, blend, then repack.
    let alpha = color & 0xff;
    let old = unsafe { pixel.read() };
    let packed = if alpha == 0xff {
        pack_color(color, framebuffer)
    } else {
        let (old_red, old_green, old_blue) = unpack_color(old, framebuffer);
        pack_color(
            (blend_channel((color >> 24) & 0xff, old_red, alpha) << 24)
                | (blend_channel((color >> 16) & 0xff, old_green, alpha) << 16)
                | (blend_channel((color >> 8) & 0xff, old_blue, alpha) << 8)
                | 0xff,
            framebuffer,
        )
    };
    unsafe {
        pixel.write(packed);
    }
    Ok(())
}

fn pack_color(color: u32, framebuffer: &Framebuffer) -> u32 {
    pack_channel((color >> 24) & 0xff, framebuffer.red)
        | pack_channel((color >> 16) & 0xff, framebuffer.green)
        | pack_channel((color >> 8) & 0xff, framebuffer.blue)
}

fn unpack_color(pixel: u32, framebuffer: &Framebuffer) -> (u32, u32, u32) {
    (
        unpack_channel(pixel, framebuffer.red),
        unpack_channel(pixel, framebuffer.green),
        unpack_channel(pixel, framebuffer.blue),
    )
}

fn unpack_channel(pixel: u32, channel: Channel) -> u32 {
    let maximum = (1u32 << channel.size) - 1;
    let value = (pixel >> channel.position) & maximum;
    (value * 255 + maximum / 2) / maximum
}

fn pack_channel(value: u32, channel: Channel) -> u32 {
    let maximum = (1u32 << channel.size) - 1;
    ((value * maximum + 127) / 255) << channel.position
}

fn blend_channel(source: u32, destination: u32, alpha: u32) -> u32 {
    (source * alpha + destination * (255 - alpha) + 127) / 255
}

unsafe fn save_cursor(framebuffer: &Framebuffer, x: usize, y: usize, width: usize, height: usize) {
    // SAFETY: coordinates and extents are clipped by draw_cursor, and the
    // cursor backup has capacity for CURSOR_WIDTH * CURSOR_HEIGHT pixels.
    let backup = core::ptr::addr_of_mut!(CURSOR_BACKUP).cast::<u32>();
    for row in 0..height {
        for column in 0..width {
            let source = framebuffer
                .backbuffer
                .add((y + row) * framebuffer.width + x + column);
            backup.add(row * CURSOR_WIDTH + column).write(source.read());
        }
    }
}

unsafe fn restore_cursor(
    framebuffer: &Framebuffer,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) {
    // SAFETY: the saved rectangle was clipped to these framebuffer bounds and
    // fits the fixed cursor backup array.
    let backup = core::ptr::addr_of!(CURSOR_BACKUP).cast::<u32>();
    for row in 0..height {
        for column in 0..width {
            framebuffer
                .backbuffer
                .add((y + row) * framebuffer.width + x + column)
                .write(backup.add(row * CURSOR_WIDTH + column).read());
        }
    }
}
