//! Small, explicitly limited GDI-shaped hooks backed by Horizon's framebuffer.
//!
//! The native framebuffer and the Windows drawing model both use pixel units
//! and a top-left origin with Y increasing downward. GDI `RECT` values are
//! signed and half-open (`left..right`, `top..bottom`); Horizon rectangles use
//! an unsigned origin plus width and height. The hooks clip signed coordinates
//! to the framebuffer before converting them to the native API.
//!
//! TODO: Route these calls through a validated guest ABI dispatcher and a
//! per-process drawing context before exposing them to Windows binaries.

use core::ffi::c_void;

/// Minimal framebuffer stand-in used only when this module is compiled directly
/// as the standalone coordinate-test crate by `make test-win32`.
#[cfg(test)]
mod graphics {
    /// Subset of palette tokens needed by the standalone GDI test crate.
    pub struct TestTheme {
        /// Primary readable text color.
        pub text_primary: u32,
        /// Neutral fill for a demo brush.
        pub control_neutral: u32,
        /// Restrained success color.
        pub status_success: u32,
        /// Restrained destructive-action color.
        pub destructive_muted: u32,
    }

    /// Deterministic palette matching the production theme's GDI-relevant tokens.
    pub const THEME: TestTheme = TestTheme {
        text_primary: 0xe7ebf0ff,
        control_neutral: 0x2a3440ff,
        status_success: 0xa8bacaff,
        destructive_muted: 0x9c5e62ff,
    };

    /// Supplies a deterministic framebuffer shape for the isolated test crate.
    ///
    /// TODO: Keep conversion tests independent from display hardware.
    pub fn dimensions() -> Result<(usize, usize), &'static str> {
        Ok((100, 100))
    }

    /// Accepts native rectangle calls in the isolated test crate.
    ///
    /// TODO: Add an instrumented pixel buffer if wrapper rendering is tested.
    pub fn draw_rectangle(
        _x: usize,
        _y: usize,
        _width: usize,
        _height: usize,
        _color: u32,
    ) -> Result<(), &'static str> {
        Ok(())
    }

    /// Accepts signed text calls in the isolated test crate.
    ///
    /// TODO: Add an instrumented pixel buffer if text rendering is tested.
    pub fn draw_string_signed(
        _x: i32,
        _y: i32,
        _text: &str,
        _color: u32,
    ) -> Result<(), &'static str> {
        Ok(())
    }
}

/// Opaque device-context argument from the classic GDI function signatures.
///
/// The current prototype ignores this handle and always targets Horizon's
/// global framebuffer. A real implementation must resolve a validated,
/// process-owned drawing context rather than trusting an incoming pointer.
pub type Hdc = *mut c_void;

/// Opaque brush handle from the classic `FillRect` function signature.
pub type Hbrush = *mut c_void;

/// C-layout, signed, half-open rectangle matching the Win32 `RECT` field layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Inclusive left X coordinate in pixels.
    pub left: i32,
    /// Inclusive top Y coordinate in pixels.
    pub top: i32,
    /// Exclusive right X coordinate in pixels.
    pub right: i32,
    /// Exclusive bottom Y coordinate in pixels.
    pub bottom: i32,
}

/// Demo-only blue brush token; this integer-shaped handle is never dereferenced.
///
/// NOTE: This is not a native GDI object handle and must not be accepted from an
/// untrusted process. It exists solely so the in-kernel demo can exercise
/// `FillRect` without implementing GDI object management.
pub const DEMO_BLUE_BRUSH: Hbrush = 1usize as Hbrush;

/// Demo-only green brush token; this integer-shaped handle is never dereferenced.
///
/// TODO: Replace demo tokens with validated, reference-counted brush objects
/// owned by the process that created them.
pub const DEMO_GREEN_BRUSH: Hbrush = 2usize as Hbrush;

/// Demo-only red brush token; this integer-shaped handle is never dereferenced.
///
/// TODO: Replace demo tokens with validated, reference-counted brush objects
/// owned by the process that created them.
pub const DEMO_RED_BRUSH: Hbrush = 3usize as Hbrush;

/// Draws a byte string using the native embedded font.
///
/// `hdc` is currently ignored. `x` and `y` are signed framebuffer pixel
/// coordinates and are clipped at each pixel, preserving partially visible
/// glyphs when an origin is negative. `text` points to `length` readable bytes;
/// bytes outside ASCII are rendered as `?`. At most 256 bytes are accepted.
/// The return value follows Win32 `BOOL` convention: nonzero on success.
///
/// # Safety
/// For nonzero `length`, `text` must reference `length` readable bytes in the
/// current kernel address space. This hook cannot validate Ring 3 addresses;
/// an eventual guest ABI gateway must copy through checked user memory first.
///
/// TODO: Implement the Windows ANSI code page and use a process-local HDC.
#[no_mangle]
pub unsafe extern "C" fn TextOutA(hdc: Hdc, x: i32, y: i32, text: *const u8, length: i32) -> i32 {
    // TODO: Route GDI operations through a validated process-local software
    // canvas before allowing a future Vulkan-backed accelerator to consume them.
    let _ = hdc;
    if length < 0 || length as usize > MAX_TEXT_BYTES {
        return 0;
    }

    let mut ascii = [0u8; MAX_TEXT_BYTES];
    if length > 0 {
        if text.is_null() {
            return 0;
        }
        // SAFETY: The caller contract requires a readable buffer for the
        // declared count; the maximum was checked above and the source is
        // immediately copied into kernel-owned storage before rendering.
        let input = unsafe { core::slice::from_raw_parts(text, length as usize) };
        for (destination, source) in ascii.iter_mut().zip(input.iter().copied()) {
            *destination = if source.is_ascii() { source } else { b'?' };
        }
    }

    // Every copied byte is ASCII, so this conversion cannot fail.
    let output = match core::str::from_utf8(&ascii[..length as usize]) {
        Ok(output) => output,
        Err(_) => return 0,
    };
    match crate::graphics::draw_string_signed(x, y, output, crate::graphics::THEME.text_primary) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

/// Fills a signed, half-open rectangle with one of the demo brush colors.
///
/// `hdc` is currently ignored. The inclusive top-left and exclusive bottom
/// right coordinates are clipped against the framebuffer, then converted to
/// Horizon's `(x, y, width, height)` convention. Only the three `DEMO_*_BRUSH`
/// tokens are recognized; unknown handles fail rather than being dereferenced.
/// The return value follows Win32 `int` convention: nonzero on success.
///
/// # Safety
/// `rect` must reference a readable `Rect` in the current kernel address space.
/// This function does not dereference `brush`, but guest callers must not be
/// allowed to invoke it until handles and pointers are safely marshalled.
///
/// NOTE: Treat the brush values as test-only tokens, not as Win32-compatible
/// handles or a security boundary.
#[no_mangle]
pub unsafe extern "C" fn FillRect(hdc: Hdc, rect: *const Rect, brush: Hbrush) -> i32 {
    // TODO: Replace demo brush tokens with owned GDI objects and route validated
    // draw commands into a per-process canvas that a Vulkan backend can consume.
    let _ = hdc;
    if rect.is_null() {
        return 0;
    }
    let color = match brush as usize {
        1 => crate::graphics::THEME.control_neutral,
        2 => crate::graphics::THEME.status_success,
        3 => crate::graphics::THEME.destructive_muted,
        _ => return 0,
    };

    // SAFETY: The caller promises a readable Rect; an unaligned read avoids
    // imposing stronger alignment requirements than the C ABI requires.
    let rect = unsafe { rect.read_unaligned() };
    let dimensions = match crate::graphics::dimensions() {
        Ok(dimensions) => dimensions,
        Err(_) => return 0,
    };
    let (x, y, width, height) = match clipped_bounds(rect, dimensions.0, dimensions.1) {
        Ok(bounds) => bounds,
        Err(_) => return 0,
    };
    match crate::graphics::draw_rectangle(x, y, width, height, color) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

const MAX_TEXT_BYTES: usize = 256;

/// Converts GDI's signed half-open rectangle to clipped native origin/extents.
///
/// Rectangles with equal edges are valid empty operations. Inverted edges are
/// rejected; portions outside the framebuffer are clipped without arithmetic
/// overflow, including rectangles whose entire area lies above or left of zero.
fn clipped_bounds(
    rect: Rect,
    framebuffer_width: usize,
    framebuffer_height: usize,
) -> Result<(usize, usize, usize, usize), &'static str> {
    if rect.right < rect.left || rect.bottom < rect.top {
        return Err("GDI rectangle has inverted edges");
    }

    let left = i64::from(rect.left).max(0).min(framebuffer_width as i64) as usize;
    let top = i64::from(rect.top).max(0).min(framebuffer_height as i64) as usize;
    let right = i64::from(rect.right).max(0).min(framebuffer_width as i64) as usize;
    let bottom = i64::from(rect.bottom).max(0).min(framebuffer_height as i64) as usize;

    Ok((
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ))
}

#[cfg(test)]
mod tests {
    //! Pure coordinate tests for the ABI-to-framebuffer conversion.

    use super::{clipped_bounds, Rect};

    /// Keeps the inclusive/exclusive GDI edges as native origin and extents.
    #[test]
    fn converts_half_open_rectangle_to_native_extent() {
        assert_eq!(
            clipped_bounds(
                Rect {
                    left: 10,
                    top: 20,
                    right: 40,
                    bottom: 50,
                },
                100,
                100,
            ),
            Ok((10, 20, 30, 30))
        );
    }

    /// Clips negative GDI origins without unsigned wraparound.
    #[test]
    fn clips_negative_rectangle_origin() {
        assert_eq!(
            clipped_bounds(
                Rect {
                    left: -5,
                    top: -8,
                    right: 12,
                    bottom: 15,
                },
                100,
                100,
            ),
            Ok((0, 0, 12, 15))
        );
    }

    /// Produces an empty native area for a rectangle outside the framebuffer.
    #[test]
    fn clips_rectangle_outside_framebuffer_to_empty_area() {
        assert_eq!(
            clipped_bounds(
                Rect {
                    left: 120,
                    top: 10,
                    right: 140,
                    bottom: 20,
                },
                100,
                100,
            ),
            Ok((100, 10, 0, 10))
        );
    }

    /// Rejects malformed inverted edges instead of reinterpreting their extent.
    #[test]
    fn rejects_inverted_rectangle() {
        assert_eq!(
            clipped_bounds(
                Rect {
                    left: 4,
                    top: 0,
                    right: 3,
                    bottom: 1,
                },
                100,
                100,
            ),
            Err("GDI rectangle has inverted edges")
        );
    }
}
