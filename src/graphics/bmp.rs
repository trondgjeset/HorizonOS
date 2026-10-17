//! Bounds-checked decoder and framebuffer renderer for uncompressed BMP files.

const BITMAP_FILE_HEADER_SIZE: usize = 14;
const BITMAP_INFO_HEADER_MIN_SIZE: usize = 40;
const BI_RGB: u32 = 0;
const MAX_BITMAP_PIXELS: usize = 16 * 1024 * 1024;
/// Maximum accepted BMP file size for wallpaper decoding.
///
/// NOTE: Contribution Area: adjust only alongside heap budgeting and parser
/// coverage for large dimensions.
pub const MAX_WALLPAPER_BYTES: usize = 8 * 1024 * 1024;

/// Validated BMP metadata borrowing the original file bytes.
///
/// NOTE: Contribution Area: support additional DIB header variants after
/// specifying and testing their color-mask and compression rules.
pub struct Bitmap<'a> {
    bytes: &'a [u8],
    pixel_offset: usize,
    width: usize,
    height: usize,
    row_stride: usize,
    bits_per_pixel: usize,
    top_down: bool,
}

impl<'a> Bitmap<'a> {
    /// Parses a Windows BMP with a BITMAPINFOHEADER-or-larger DIB header.
    ///
    /// Only positive-width, nonzero-height, uncompressed 24-bit or 32-bit
    /// `BI_RGB` images are accepted. Every offset and byte-count calculation is
    /// checked before a slice is accessed.
    ///
    /// NOTE: Contribution Area: add palette, bitfield, and compressed decoding
    /// only with explicit bounds and malformed-input tests.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_WALLPAPER_BYTES {
            return Err("BMP exceeds the wallpaper byte limit");
        }
        if bytes.len() < BITMAP_FILE_HEADER_SIZE + BITMAP_INFO_HEADER_MIN_SIZE {
            return Err("BMP is shorter than its file and info headers");
        }
        if bytes.get(0..2) != Some(b"BM") {
            return Err("BMP signature is not BM");
        }

        let declared_file_size = read_u32(bytes, 2)? as usize;
        let pixel_offset = read_u32(bytes, 10)? as usize;
        let dib_size = read_u32(bytes, BITMAP_FILE_HEADER_SIZE)? as usize;
        let header_end = BITMAP_FILE_HEADER_SIZE
            .checked_add(dib_size)
            .ok_or("BMP DIB header size overflow")?;
        if dib_size < BITMAP_INFO_HEADER_MIN_SIZE || header_end > bytes.len() {
            return Err("BMP DIB header is unsupported or truncated");
        }
        if pixel_offset < header_end || pixel_offset > bytes.len() {
            return Err("BMP pixel array offset is outside the image data");
        }

        let signed_width = read_i32(bytes, 18)?;
        let signed_height = read_i32(bytes, 22)?;
        if signed_width <= 0 || signed_height == 0 || signed_height == i32::MIN {
            return Err("BMP dimensions must have positive width and nonzero height");
        }
        let width = signed_width as usize;
        let height = signed_height.unsigned_abs() as usize;
        let pixel_count = width
            .checked_mul(height)
            .ok_or("BMP pixel count overflow")?;
        if pixel_count > MAX_BITMAP_PIXELS {
            return Err("BMP dimensions exceed the supported pixel limit");
        }
        if read_u16(bytes, 26)? != 1 {
            return Err("BMP must contain exactly one image plane");
        }
        let bits_per_pixel = read_u16(bytes, 28)? as usize;
        if bits_per_pixel != 24 && bits_per_pixel != 32 {
            return Err("only 24-bit and 32-bit BMP images are supported");
        }
        if read_u32(bytes, 30)? != BI_RGB {
            return Err("compressed and bitfield BMP images are not supported");
        }

        let row_bytes = width
            .checked_mul(bits_per_pixel / 8)
            .ok_or("BMP row byte count overflow")?;
        let row_stride = row_bytes
            .checked_add(3)
            .ok_or("BMP row alignment overflow")?
            & !3;
        let pixel_bytes = row_stride
            .checked_mul(height)
            .ok_or("BMP pixel array size overflow")?;
        let pixel_end = pixel_offset
            .checked_add(pixel_bytes)
            .ok_or("BMP pixel array end overflow")?;
        if pixel_end > bytes.len() {
            return Err("BMP pixel array is truncated");
        }
        if declared_file_size < pixel_end || declared_file_size > bytes.len() {
            return Err("BMP declared file size does not contain its pixel array");
        }

        Ok(Self {
            bytes,
            pixel_offset,
            width,
            height,
            row_stride,
            bits_per_pixel,
            top_down: signed_height < 0,
        })
    }

    /// Returns the decoded dimensions in source-image pixels.
    ///
    /// NOTE: Contribution Area: expose image metadata to the desktop wallpaper
    /// chooser when one is implemented.
    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Returns one source pixel as opaque `0xRRGGBBAA`, with bounds checking.
    ///
    /// BMP rows are padded to a four-byte boundary and stored bottom-up unless
    /// the height is negative. The fourth byte in a 32-bit `BI_RGB` pixel is
    /// treated as reserved rather than meaningful alpha.
    ///
    /// NOTE: Contribution Area: define an explicit alpha policy before
    /// supporting alpha-bearing V4/V5 BMP formats.
    pub fn pixel(&self, x: usize, y: usize) -> Result<u32, &'static str> {
        if x >= self.width || y >= self.height {
            return Err("BMP pixel coordinate is outside the image");
        }
        let source_row = if self.top_down {
            y
        } else {
            self.height - 1 - y
        };
        let bytes_per_pixel = self.bits_per_pixel / 8;
        let pixel_index = x
            .checked_mul(bytes_per_pixel)
            .ok_or("BMP pixel offset overflow")?;
        let pixel_start = self
            .pixel_offset
            .checked_add(
                source_row
                    .checked_mul(self.row_stride)
                    .ok_or("BMP row offset overflow")?,
            )
            .and_then(|offset| offset.checked_add(pixel_index))
            .ok_or("BMP pixel address overflow")?;
        let pixel_end = pixel_start
            .checked_add(bytes_per_pixel)
            .ok_or("BMP pixel end overflow")?;
        let source = self
            .bytes
            .get(pixel_start..pixel_end)
            .ok_or("BMP pixel bytes are outside the image data")?;
        Ok((u32::from(source[2]) << 24)
            | (u32::from(source[1]) << 16)
            | (u32::from(source[0]) << 8)
            | 0xff)
    }
}

/// Scales a validated BMP to the current framebuffer and draws it as wallpaper.
///
/// Nearest-neighbor scaling maps the complete source image over the full
/// framebuffer without temporary allocation. Pixels are written directly to
/// the compositor back buffer before window layers are drawn.
///
/// NOTE: Contribution Area: add aspect-preserving crop/letterbox modes and
/// higher-quality filtering after benchmarking on the target CPU.
pub fn draw_wallpaper(bmp_bytes: &[u8]) -> Result<(), &'static str> {
    let bitmap = Bitmap::parse(bmp_bytes)?;
    let framebuffer = super::current_framebuffer()?;
    // A full background redraw invalidates the previous frame's saved cursor
    // pixels; letting cursor restoration run would overwrite new window pixels.
    // SAFETY: the compositor calls this routine on its single desktop task,
    // the same context that owns all reads and writes to this cursor cache.
    unsafe {
        super::PREVIOUS_CURSOR = None;
    }
    let (source_width, source_height) = bitmap.dimensions();
    let source_x_step = source_width / framebuffer.width;
    let source_x_remainder = source_width % framebuffer.width;
    let mut source_y = 0usize;
    let source_y_step = source_height / framebuffer.height;
    let source_y_remainder = source_height % framebuffer.height;
    let mut y_remainder = 0usize;
    for destination_y in 0..framebuffer.height {
        let mut source_x = 0usize;
        let mut x_remainder = 0usize;
        for destination_x in 0..framebuffer.width {
            super::put_pixel_in(
                &framebuffer,
                destination_x,
                destination_y,
                bitmap.pixel(source_x, source_y)?,
            )?;
            advance_scale_coordinate(
                &mut source_x,
                &mut x_remainder,
                source_x_step,
                source_x_remainder,
                framebuffer.width,
            );
        }
        advance_scale_coordinate(
            &mut source_y,
            &mut y_remainder,
            source_y_step,
            source_y_remainder,
            framebuffer.height,
        );
    }
    Ok(())
}

/// Advances one nearest-neighbor source coordinate using an error accumulator.
///
/// NOTE: Contribution Area: replace nearest-neighbor sampling with a tested
/// interpolation kernel if wallpaper scaling quality is later improved.
fn advance_scale_coordinate(
    source_coordinate: &mut usize,
    remainder: &mut usize,
    source_step: usize,
    source_remainder: usize,
    destination_extent: usize,
) {
    *source_coordinate = source_coordinate.saturating_add(source_step);
    *remainder += source_remainder;
    if *remainder >= destination_extent {
        *source_coordinate = source_coordinate.saturating_add(1);
        *remainder -= destination_extent;
    }
}

/// Reads a little-endian unsigned 16-bit field without alignment assumptions.
///
/// NOTE: Contribution Area: centralize binary-format readers if other image
/// decoders are introduced.
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, &'static str> {
    let end = offset.checked_add(2).ok_or("BMP field offset overflow")?;
    let field = bytes
        .get(offset..end)
        .ok_or("BMP header field is truncated")?;
    Ok(u16::from_le_bytes([field[0], field[1]]))
}

/// Reads a little-endian unsigned 32-bit field without alignment assumptions.
///
/// NOTE: Contribution Area: centralize binary-format readers if other image
/// decoders are introduced.
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, &'static str> {
    let end = offset.checked_add(4).ok_or("BMP field offset overflow")?;
    let field = bytes
        .get(offset..end)
        .ok_or("BMP header field is truncated")?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

/// Reads a little-endian signed 32-bit field without alignment assumptions.
///
/// NOTE: Contribution Area: keep signed dimensions signed until orientation
/// and absolute-value overflow have been validated.
fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, &'static str> {
    let end = offset.checked_add(4).ok_or("BMP field offset overflow")?;
    let field = bytes
        .get(offset..end)
        .ok_or("BMP header field is truncated")?;
    Ok(i32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

#[cfg(test)]
mod tests {
    use super::{Bitmap, BI_RGB};
    use alloc::vec;
    use alloc::vec::Vec;

    /// Builds a tiny padded BMP fixture in either row orientation and pixel depth.
    ///
    /// NOTE: Contribution Area: extend fixtures with valid external BMP samples
    /// only when their licensing and test provenance are clear.
    fn fixture(bits_per_pixel: u16, signed_height: i32) -> Vec<u8> {
        let width = 3usize;
        let height = signed_height.unsigned_abs() as usize;
        let bytes_per_pixel = usize::from(bits_per_pixel / 8);
        let row_stride = (width * bytes_per_pixel + 3) & !3;
        let pixel_offset = 54usize;
        let file_size = pixel_offset + row_stride * height;
        let mut bytes = vec![0; file_size];
        bytes[0..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&(file_size as u32).to_le_bytes());
        bytes[10..14].copy_from_slice(&(pixel_offset as u32).to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&signed_height.to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&bits_per_pixel.to_le_bytes());
        bytes[30..34].copy_from_slice(&BI_RGB.to_le_bytes());

        for y in 0..height {
            let file_y = if signed_height < 0 { y } else { height - y - 1 };
            for x in 0..width {
                let pixel = pixel_offset + file_y * row_stride + x * bytes_per_pixel;
                bytes[pixel] = (x + 1) as u8;
                bytes[pixel + 1] = (y + 2) as u8;
                bytes[pixel + 2] = (y * width + x + 3) as u8;
                if bytes_per_pixel == 4 {
                    bytes[pixel + 3] = 0;
                }
            }
        }
        bytes
    }

    /// Confirms padded bottom-up 24-bit rows decode in display order.
    ///
    /// NOTE: Contribution Area: retain odd-width fixtures to guard row padding.
    #[test]
    fn decodes_padded_24_bit_bottom_up_rows() {
        let bytes = fixture(24, 2);
        let bitmap = Bitmap::parse(&bytes).unwrap();
        assert_eq!(bitmap.dimensions(), (3, 2));
        assert_eq!(bitmap.pixel(0, 0).unwrap(), 0x030201ff);
        assert_eq!(bitmap.pixel(2, 1).unwrap(), 0x080303ff);
    }

    /// Confirms top-down 32-bit BI_RGB data ignores its reserved alpha byte.
    ///
    /// NOTE: Contribution Area: assert explicit alpha handling for every new
    /// bit depth or DIB header variant.
    #[test]
    fn decodes_top_down_32_bit_pixels_as_opaque() {
        let bytes = fixture(32, -2);
        let bitmap = Bitmap::parse(&bytes).unwrap();
        assert_eq!(bitmap.pixel(1, 0).unwrap(), 0x040202ff);
    }

    /// Rejects malformed headers, unsupported formats, and truncated pixels.
    ///
    /// NOTE: Contribution Area: fuzz the parser after adding a hosted fuzz target.
    #[test]
    fn rejects_malformed_or_truncated_images() {
        let valid = fixture(24, 2);
        assert!(Bitmap::parse(&valid[..20]).is_err());
        let mut invalid = valid.clone();
        invalid[0] = b'X';
        assert!(Bitmap::parse(&invalid).is_err());
        let mut unsupported = valid.clone();
        unsupported[28..30].copy_from_slice(&8u16.to_le_bytes());
        assert!(Bitmap::parse(&unsupported).is_err());
        let mut invalid_offset = valid.clone();
        invalid_offset[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Bitmap::parse(&invalid_offset).is_err());
        let mut invalid_size = valid.clone();
        invalid_size[2..6].copy_from_slice(&54u32.to_le_bytes());
        assert!(Bitmap::parse(&invalid_size).is_err());
        assert!(Bitmap::parse(&valid[..valid.len() - 1]).is_err());
    }

    /// Verifies the incremental scaler matches exact nearest-neighbor coordinates.
    ///
    /// NOTE: Contribution Area: retain upscaling and downscaling cases when
    /// replacing the fixed-point-free error accumulator.
    #[test]
    fn incremental_scaler_matches_nearest_neighbor_math() {
        for (source_extent, destination_extent) in [(3usize, 8usize), (11, 4), (5, 5)] {
            let mut coordinate = 0usize;
            let mut remainder = 0usize;
            let step = source_extent / destination_extent;
            let step_remainder = source_extent % destination_extent;
            for destination in 0..destination_extent {
                assert_eq!(coordinate, destination * source_extent / destination_extent);
                super::advance_scale_coordinate(
                    &mut coordinate,
                    &mut remainder,
                    step,
                    step_remainder,
                    destination_extent,
                );
            }
        }
    }
}
