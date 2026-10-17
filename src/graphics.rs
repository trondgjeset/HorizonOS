use core::ptr::{copy_nonoverlapping, read_unaligned};

use crate::font8x8::FONT8X8_BASIC;

/// Safe decoder and framebuffer blitter for uncompressed Windows bitmaps.
///
/// NOTE: Contribution Area: expand supported formats only with strict
/// dimension, compression, and allocation bounds.
#[path = "graphics/bmp.rs"]
pub mod bmp;

const MULTIBOOT_INFO_FRAMEBUFFER: u32 = 1 << 12;
const MULTIBOOT_FRAMEBUFFER_RGB: u8 = 1;
const MAX_BACKBUFFER_PIXELS: usize = 1024 * 1024;
/// Central palette for the polished charcoal-and-steel Horizon desktop.
///
/// Each value uses the renderer's `0xRRGGBBAA` convention. Keep the ice-blue
/// accent limited to selected/focused indicators; ordinary surfaces, controls,
/// and text use the neutral slate tokens.
///
/// NOTE: Brand Styling: customize palette values in [`THEME`] rather than
/// introducing local color literals in individual widgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Deep charcoal canvas shown behind the windows.
    ///
    /// NOTE: Brand Styling: this is the lowest-contrast foundation token.
    pub background_canvas: u32,
    /// Muted dark steel-gray title bar for the focused window.
    ///
    /// NOTE: Brand Styling: its partial alpha preserves subtle wallpaper depth.
    pub window_bar_active: u32,
    /// Off-black slate title bar for unfocused windows.
    ///
    /// NOTE: Brand Styling: keep inactive chrome quieter than the active bar.
    pub window_bar_inactive: u32,
    /// Ice Blue `#4A90E2` used only for active focus/selection indicators.
    ///
    /// NOTE: Brand Styling: reserve this token for deliberate interaction
    /// feedback rather than large surfaces or decorative text.
    pub accent_highlight: u32,
    /// Primary off-white text color for dark surfaces.
    ///
    /// NOTE: Brand Styling: maintain readable contrast against slate surfaces.
    pub text_primary: u32,
    /// Secondary neutral text color for supporting information.
    ///
    /// NOTE: Brand Styling: use a lower contrast than `text_primary`.
    pub text_secondary: u32,
    /// Thin steel-gray strokes for idle borders and wireframes.
    ///
    /// NOTE: Brand Styling: use the accent instead only while a component is active.
    pub border_subtle: u32,
    /// Opaque elevated slate surface for content and standard windows.
    ///
    /// NOTE: Brand Styling: separate surfaces with luminance, not extra accents.
    pub surface_window: u32,
    /// Translucent dark steel surface for the Start Menu panel.
    ///
    /// NOTE: Brand Styling: tune alpha to balance glass effect and legibility.
    pub surface_glass: u32,
    /// Quiet neutral surface for idle window controls and taskbar tabs.
    ///
    /// NOTE: Brand Styling: keep control chrome subordinate to window content.
    pub control_neutral: u32,
    /// Muted red reserved for the destructive close control and errors.
    ///
    /// NOTE: Brand Styling: keep destructive feedback distinct but restrained.
    pub destructive_muted: u32,
    /// Restrained success feedback color.
    ///
    /// NOTE: Brand Styling: avoid using success status as a general accent.
    pub status_success: u32,
    /// Restrained error feedback color.
    ///
    /// NOTE: Brand Styling: reserve this token for errors and validation state.
    pub status_error: u32,
}

/// Default dark corporate palette shared by the graphics and window layers.
///
/// NOTE: Brand Styling: edit this single initializer to retheme all components
/// that consume the global theme lookup.
pub const THEME: Theme = Theme {
    background_canvas: 0x0e141cff,
    window_bar_active: 0x46515fde,
    window_bar_inactive: 0x131a23ff,
    accent_highlight: 0x4a90e2ff,
    text_primary: 0xe7ebf0ff,
    text_secondary: 0xaeb8c4ff,
    border_subtle: 0x394553ff,
    surface_window: 0x161f2aff,
    surface_glass: 0x171f2bdd,
    control_neutral: 0x2a3440ff,
    destructive_muted: 0x9c5e62ff,
    status_success: 0xa8bacaff,
    status_error: 0xb9797fff,
};

const MAX_SHADOW_BLUR: usize = 32;
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

/// Draws a clipped, anti-aliased rounded rectangle into the back buffer.
///
/// `x` and `y` are the upper-left pixel coordinates; `width` and `height` are
/// extents; `radius` is clamped to half the shorter edge; and `color` is
/// `0xRRGGBBAA`. Edge coverage scales the source alpha and is composited over
/// the current back-buffer pixel. Returns an error if graphics is unavailable.
///
/// NOTE: Design Customization: change radius and the one-pixel coverage ramp
/// together to establish a consistent corner style across desktop controls.
pub fn draw_rounded_rectangle(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    radius: usize,
    color: u32,
) -> Result<(), &'static str> {
    let framebuffer = current_framebuffer()?;
    if width == 0 || height == 0 {
        return Ok(());
    }
    let right = x.saturating_add(width);
    let bottom = y.saturating_add(height);
    let left = x.saturating_sub(1).min(framebuffer.width);
    let top = y.saturating_sub(1).min(framebuffer.height);
    let right = right.saturating_add(1).min(framebuffer.width);
    let bottom = bottom.saturating_add(1).min(framebuffer.height);
    let radius = radius.min(width.min(height) / 2);

    for pixel_y in top..bottom {
        for pixel_x in left..right {
            let coverage =
                rounded_rectangle_coverage(pixel_x, pixel_y, x, y, width, height, radius);
            if coverage == 0 {
                continue;
            }
            let alpha = ((color & 0xff) * u32::from(coverage) + 1) / 2;
            let covered_color = (color & !0xff) | alpha;
            put_pixel_in(&framebuffer, pixel_x, pixel_y, covered_color)?;
        }
    }
    Ok(())
}

/// Draws a soft dark shadow behind a window or floating panel.
///
/// `offset` shifts the shadow down and right from the source bounds. The
/// `blur_strength` controls the number of progressively softer rounded layers
/// and is capped to bound per-frame work. Returns an error if graphics is not
/// initialized.
///
/// NOTE: Design Customization: tune the base opacity, blur cap, and offset to
/// match the visual depth of each window class.
pub fn draw_drop_shadow(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    offset: usize,
    blur_strength: usize,
) -> Result<(), &'static str> {
    let framebuffer = current_framebuffer()?;
    let blur = blur_strength.min(MAX_SHADOW_BLUR);
    if width == 0 || height == 0 || blur == 0 {
        return Ok(());
    }
    let shadow_x = x.saturating_add(offset);
    let shadow_y = y.saturating_add(offset);
    let original_right = shadow_x.saturating_add(width);
    let original_bottom = shadow_y.saturating_add(height);
    for spread in (1..=blur).rev() {
        let alpha = (64 * (blur - spread + 1) / (blur + 1)) as u32;
        let shape_x = shadow_x.saturating_sub(spread);
        let shape_y = shadow_y.saturating_sub(spread);
        let shape_width = width.saturating_add(spread.saturating_mul(2));
        let shape_height = height.saturating_add(spread.saturating_mul(2));
        let left = shape_x.saturating_sub(1).min(framebuffer.width);
        let top = shape_y.saturating_sub(1).min(framebuffer.height);
        let right = shape_x
            .saturating_add(shape_width)
            .saturating_add(1)
            .min(framebuffer.width);
        let bottom = shape_y
            .saturating_add(shape_height)
            .saturating_add(1)
            .min(framebuffer.height);
        // The window surface is painted after its shadow, so avoid blending
        // shadow pixels inside the source rectangle and visit only its border.
        for pixel_y in top..bottom {
            let inside_vertical_span = pixel_y >= shadow_y && pixel_y < original_bottom;
            for pixel_x in left..right {
                if inside_vertical_span && pixel_x >= shadow_x && pixel_x < original_right {
                    continue;
                }
                let coverage = rounded_rectangle_coverage(
                    pixel_x,
                    pixel_y,
                    shape_x,
                    shape_y,
                    shape_width,
                    shape_height,
                    spread.saturating_add(8),
                );
                if coverage == 0 {
                    continue;
                }
                let covered_alpha = (alpha * u32::from(coverage) + 1) / 2;
                put_pixel_in(&framebuffer, pixel_x, pixel_y, 0x03070b00 | covered_alpha)?;
            }
        }
    }
    Ok(())
}

/// Returns the source-over blend of two `0xRRGGBBAA` pixels.
///
/// The source alpha selects the contribution of the source RGB; destination
/// alpha is preserved as fully opaque because the framebuffer is an opaque
/// display surface. Alpha zero returns the destination RGB and alpha 255
/// returns the source RGB.
///
/// NOTE: Design Customization: this is the shared color-mixing equation for
/// translucent panels, title bars, rounded edges, and drop shadows.
pub const fn blend_rgba(source: u32, destination: u32) -> u32 {
    let alpha = source & 0xff;
    let inverse_alpha = 255 - alpha;
    let red = (((source >> 24 & 0xff) * alpha + (destination >> 24 & 0xff) * inverse_alpha + 127)
        / 255)
        << 24;
    let green =
        (((source >> 16 & 0xff) * alpha + (destination >> 16 & 0xff) * inverse_alpha + 127) / 255)
            << 16;
    let blue = (((source >> 8 & 0xff) * alpha + (destination >> 8 & 0xff) * inverse_alpha + 127)
        / 255)
        << 8;
    red | green | blue | 0xff
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
    draw_string_signed(
        x.min(i32::MAX as usize) as i32,
        y.min(i32::MAX as usize) as i32,
        text,
        color,
    )
}

/// Draws ASCII text at signed framebuffer coordinates, clipping negative origins.
///
/// Windows GDI uses signed client coordinates, while the native framebuffer
/// API uses unsigned pixel offsets. This bridge retains the glyph's original
/// position and clips individual pixels outside the top or left framebuffer
/// edges instead of wrapping negative coordinates into large `usize` values.
pub(crate) fn draw_string_signed(
    x: i32,
    y: i32,
    text: &str,
    color: u32,
) -> Result<(), &'static str> {
    let framebuffer = current_framebuffer()?;
    let origin_x = i64::from(x);
    let mut cursor_x = origin_x;
    let mut cursor_y = i64::from(y);

    for byte in text.bytes() {
        match byte {
            b'\n' => {
                cursor_x = origin_x;
                cursor_y = cursor_y.saturating_add(8);
                if cursor_y >= framebuffer.height as i64 {
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

        if cursor_x >= framebuffer.width as i64 || cursor_y >= framebuffer.height as i64 {
            break;
        }
        let glyph_index = if byte.is_ascii() {
            byte as usize
        } else {
            b'?' as usize
        };
        for (row, bits) in FONT8X8_BASIC[glyph_index].iter().copied().enumerate() {
            let pixel_y = cursor_y.saturating_add(row as i64);
            if pixel_y >= framebuffer.height as i64 {
                break;
            }
            for column in 0..8 {
                if bits & (1 << column) == 0 {
                    continue;
                }
                let pixel_x = cursor_x.saturating_add(column as i64);
                if pixel_x >= 0
                    && pixel_y >= 0
                    && pixel_x < framebuffer.width as i64
                    && pixel_y < framebuffer.height as i64
                {
                    put_pixel_in(&framebuffer, pixel_x as usize, pixel_y as usize, color)?;
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
            THEME.accent_highlight
        } else {
            THEME.text_primary
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
        let destination = (old_red << 24) | (old_green << 16) | (old_blue << 8) | 0xff;
        pack_color(blend_rgba(color, destination), framebuffer)
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

/// Calculates rounded-box edge coverage in half-pixel increments.
///
/// Coordinates are pixel indices and geometry uses the rectangle's exclusive
/// right/bottom bounds. The signed-distance calculation keeps corner behavior
/// consistent for every radius while integer square root avoids floating-point
/// runtime dependencies in the freestanding kernel.
///
/// NOTE: Design Customization: adjust the final one-pixel antialias ramp in
/// `draw_rounded_rectangle` if a sharper or softer edge is desired.
fn rounded_rectangle_coverage(
    pixel_x: usize,
    pixel_y: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    radius: usize,
) -> u8 {
    let radius = radius.min(width.min(height) / 2);
    if radius == 0 {
        return u8::from(
            pixel_x >= x
                && pixel_y >= y
                && pixel_x < x.saturating_add(width)
                && pixel_y < y.saturating_add(height),
        ) * 2;
    }
    let point_x = (pixel_x as i64 - x as i64) * 2 + 1;
    let point_y = (pixel_y as i64 - y as i64) * 2 + 1;
    let width = width as i64 * 2;
    let height = height as i64 * 2;
    let radius = radius as i64 * 2;
    let qx = (point_x - width / 2).abs() - (width / 2 - radius);
    let qy = (point_y - height / 2).abs() - (height / 2 - radius);
    let outside_x = qx.max(0) as u64;
    let outside_y = qy.max(0) as u64;
    let outside_distance = integer_sqrt(
        outside_x
            .saturating_mul(outside_x)
            .saturating_add(outside_y.saturating_mul(outside_y)),
    ) as i64;
    let signed_distance = outside_distance + qx.max(qy).min(0) - radius;
    (1 - signed_distance).clamp(0, 2) as u8
}

/// Computes a floor integer square root without relying on libm.
///
/// NOTE: Design Customization: replace this with a measured architecture-
/// specific primitive only if rounded edges become a significant render cost.
const fn integer_sqrt(value: u64) -> u64 {
    let mut result = 0u64;
    let mut bit = 1u64 << 62;
    while bit > value {
        bit >>= 2;
    }
    let mut remainder = value;
    while bit != 0 {
        if remainder >= result + bit {
            remainder -= result + bit;
            result = (result >> 1) + bit;
        } else {
            result >>= 1;
        }
        bit >>= 2;
    }
    result
}

#[cfg(test)]
mod tests {
    /// Verifies that the default palette keeps the requested restrained brand tokens.
    ///
    /// NOTE: Brand Styling: preserve these assertions when tuning visual
    /// values so Ice Blue stays exact and large surfaces remain neutral.
    #[test]
    fn default_theme_uses_corporate_neutral_palette() {
        assert_eq!(super::THEME.accent_highlight, 0x4a90e2ff);
        assert_eq!(super::THEME.background_canvas, 0x0e141cff);
        assert_eq!(super::THEME.window_bar_active & 0xff, 0xde);
        assert_eq!(super::THEME.window_bar_inactive & 0xff, 0xff);
        assert_ne!(
            super::THEME.window_bar_active,
            super::THEME.accent_highlight
        );
        assert_ne!(
            super::THEME.window_bar_inactive,
            super::THEME.accent_highlight
        );
    }

    /// Verifies the color mixer preserves both endpoint colors exactly.
    ///
    /// NOTE: Design Customization: retain these endpoint checks when changing
    /// the theme mixer or framebuffer alpha conventions.
    #[test]
    fn alpha_blending_obeys_transparent_and_opaque_endpoints() {
        assert_eq!(super::blend_rgba(0xff000000, 0x12345678), 0x123456ff);
        assert_eq!(super::blend_rgba(0xff000080, 0x000000ff), 0x800000ff);
        assert_eq!(super::blend_rgba(0xff0000ff, 0x12345678), 0xff0000ff);
    }

    /// Verifies corner pixels fade smoothly and the rounded-box center is solid.
    ///
    /// NOTE: Design Customization: update expectations only when intentionally
    /// changing the geometric coverage ramp.
    #[test]
    fn rounded_rectangle_coverage_fades_at_corners() {
        let corner = super::rounded_rectangle_coverage(0, 0, 0, 0, 12, 12, 4);
        let edge = super::rounded_rectangle_coverage(5, 0, 0, 0, 12, 12, 4);
        let center = super::rounded_rectangle_coverage(6, 6, 0, 0, 12, 12, 4);
        assert!(corner < edge);
        assert_eq!(center, 2);
    }

    /// Verifies the freestanding square-root helper around exact squares.
    ///
    /// NOTE: Design Customization: preserve floor semantics expected by the
    /// signed-distance antialias calculation.
    #[test]
    fn integer_square_root_rounds_down() {
        assert_eq!(super::integer_sqrt(0), 0);
        assert_eq!(super::integer_sqrt(15), 3);
        assert_eq!(super::integer_sqrt(16), 4);
    }
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
