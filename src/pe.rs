#[derive(Clone, Copy)]
/// Borrowed view of a validated PE32 image; sections still reference file bytes.
pub struct PeImage<'a> {
    bytes: &'a [u8],
    section_table_offset: usize,
    pub entry_point_rva: u32,
    pub image_base: u32,
    pub image_size: u32,
    section_count: usize,
}

#[derive(Clone, Copy)]
/// Decoded PE section metadata, including its unmodified eight-byte name.
pub struct PeSection<'a> {
    pub name: &'a [u8; 8],
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub raw_size: u32,
    pub raw_offset: u32,
    pub characteristics: u32,
}

#[derive(Clone, Copy)]
/// Compact PE metadata suitable for display by the sample launcher.
pub struct PeSummary {
    pub entry_point_rva: u32,
    pub image_base: u32,
    pub image_size: u32,
    pub section_count: usize,
    pub first_section_name: [u8; 8],
    pub first_section_characteristics: u32,
}

impl PeImage<'_> {
    /// Returns the number of section headers validated by the parser.
    pub fn section_count(&self) -> usize {
        self.section_count
    }

    /// Decodes section `index`, or returns `None` if its header is unavailable.
    ///
    /// The section table offset and per-header stride were validated during
    /// parsing; checked arithmetic is still used when deriving the byte range.
    pub fn section(&self, index: usize) -> Option<PeSection<'_>> {
        if index >= self.section_count {
            return None;
        }

        let offset = self
            .section_table_offset
            .checked_add(index.checked_mul(40)?)?;
        let header = self.bytes.get(offset..offset.checked_add(40)?)?;
        let name = header.get(..8)?.try_into().ok()?;
        Some(PeSection {
            name,
            virtual_size: read_u32(header, 8).ok()?,
            virtual_address: read_u32(header, 12).ok()?,
            raw_size: read_u32(header, 16).ok()?,
            raw_offset: read_u32(header, 20).ok()?,
            characteristics: read_u32(header, 36).ok()?,
        })
    }

    /// Copies the image's entry-point, base, size, and first-section summary.
    pub fn summary(&self) -> PeSummary {
        let first_section = self.section(0);
        let first_section_name = first_section.map(|section| *section.name).unwrap_or([0; 8]);
        let first_section_characteristics = first_section
            .map(|section| section.characteristics)
            .unwrap_or(0);
        PeSummary {
            entry_point_rva: self.entry_point_rva,
            image_base: self.image_base,
            image_size: self.image_size,
            section_count: self.section_count(),
            first_section_name,
            first_section_characteristics,
        }
    }
}

/// Parses and validates the structural PE32 headers in `bytes`.
///
/// Returns a borrowed image view on success, or an error for malformed,
/// truncated, unsupported, or out-of-range headers. This validates metadata
/// only; it does not map sections or execute the image.
pub fn parse_pe32(bytes: &[u8]) -> Result<PeImage<'_>, &'static str> {
    if read_u16(bytes, 0)? != 0x5a4d {
        return Err("missing DOS MZ signature");
    }

    // The DOS e_lfanew field is a file offset, not a pointer. Every subsequent
    // header position is derived with checked arithmetic and bounded slices.
    let pe_offset = read_u32(bytes, 0x3c)? as usize;
    let coff = pe_offset
        .checked_add(4)
        .ok_or("PE header offset overflow")?;
    if read_u32(bytes, pe_offset)? != 0x0000_4550 {
        return Err("missing PE signature");
    }

    if read_u16(bytes, coff)? != 0x014c {
        return Err("unsupported PE machine (expected i386)");
    }
    let section_count = read_u16(bytes, coff + 2)? as usize;
    let optional_size = read_u16(bytes, coff + 16)? as usize;
    let optional = coff.checked_add(20).ok_or("optional header overflow")?;
    let optional_end = optional
        .checked_add(optional_size)
        .ok_or("optional header size overflow")?;
    let optional_header = bytes
        .get(optional..optional_end)
        .ok_or("truncated optional header")?;

    if optional_size < 64 || read_u16(optional_header, 0)? != 0x010b {
        return Err("unsupported or truncated PE32 optional header");
    }

    let entry_point_rva = read_u32(optional_header, 16)?;
    let image_base = read_u32(optional_header, 28)?;
    let section_alignment = read_u32(optional_header, 32)?;
    let file_alignment = read_u32(optional_header, 36)?;
    let image_size = read_u32(optional_header, 56)?;
    let headers_size = read_u32(optional_header, 60)? as usize;

    if section_alignment == 0 || file_alignment == 0 || image_size == 0 {
        return Err("invalid PE alignment or image size");
    }
    if entry_point_rva >= image_size {
        return Err("entry point lies outside the image");
    }
    image_base
        .checked_add(image_size)
        .ok_or("image virtual address overflow")?;
    if headers_size > bytes.len() {
        return Err("PE headers exceed file size");
    }

    // COFF SizeOfOptionalHeader determines where section headers begin; using
    // it avoids assuming a fixed Optional Header length.
    let section_table_offset = optional_end;
    let section_table_size = section_count
        .checked_mul(40)
        .ok_or("section table size overflow")?;
    let section_table_end = section_table_offset
        .checked_add(section_table_size)
        .ok_or("section table offset overflow")?;
    if section_table_end > bytes.len() || section_table_end > headers_size {
        return Err("truncated section table");
    }

    let image = PeImage {
        bytes,
        section_table_offset,
        entry_point_rva,
        image_base,
        image_size,
        section_count,
    };

    for index in 0..section_count {
        let section = image.section(index).ok_or("invalid section header")?;
        let mapped_size = section.virtual_size.max(section.raw_size);
        let section_end = section
            .virtual_address
            .checked_add(mapped_size)
            .ok_or("section virtual address overflow")?;
        if section_end > image_size {
            return Err("section lies outside the image");
        }

        if section.raw_size != 0 {
            let raw_end = section
                .raw_offset
                .checked_add(section.raw_size)
                .ok_or("section file range overflow")?;
            if raw_end as usize > bytes.len() {
                return Err("section raw data exceeds file size");
            }
        }
    }

    Ok(image)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, &'static str> {
    let value = bytes
        .get(offset..offset.checked_add(2).ok_or("field offset overflow")?)
        .ok_or("truncated PE field")?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, &'static str> {
    let value = bytes
        .get(offset..offset.checked_add(4).ok_or("field offset overflow")?)
        .ok_or("truncated PE field")?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

#[cfg(test)]
mod tests {
    use super::parse_pe32;

    fn fixture() -> [u8; 0x600] {
        let mut bytes = [0u8; 0x600];
        put_u16(&mut bytes, 0, 0x5a4d);
        put_u32(&mut bytes, 0x3c, 0x80);
        put_u32(&mut bytes, 0x80, 0x0000_4550);

        let coff = 0x84;
        put_u16(&mut bytes, coff, 0x014c);
        put_u16(&mut bytes, coff + 2, 2);
        put_u16(&mut bytes, coff + 18, 0x0102);
        put_u16(&mut bytes, coff + 16, 224);

        let optional = coff + 20;
        put_u16(&mut bytes, optional, 0x010b);
        put_u32(&mut bytes, optional + 16, 0x1000);
        put_u32(&mut bytes, optional + 28, 0x0040_0000);
        put_u32(&mut bytes, optional + 32, 0x1000);
        put_u32(&mut bytes, optional + 36, 0x200);
        put_u32(&mut bytes, optional + 56, 0x3000);
        put_u32(&mut bytes, optional + 60, 0x200);
        put_u16(&mut bytes, optional + 68, 3);

        let sections = optional + 224;
        bytes[sections..sections + 8].copy_from_slice(b".text\0\0\0");
        put_u32(&mut bytes, sections + 8, 0x10);
        put_u32(&mut bytes, sections + 12, 0x1000);
        put_u32(&mut bytes, sections + 16, 0x200);
        put_u32(&mut bytes, sections + 20, 0x200);
        put_u32(&mut bytes, sections + 36, 0x6000_0020);

        let data = sections + 40;
        bytes[data..data + 8].copy_from_slice(b".data\0\0\0");
        put_u32(&mut bytes, data + 8, 0x20);
        put_u32(&mut bytes, data + 12, 0x2000);
        put_u32(&mut bytes, data + 16, 0x200);
        put_u32(&mut bytes, data + 20, 0x400);
        put_u32(&mut bytes, data + 36, 0xc000_0040);
        bytes
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn parses_pe32_headers_and_sections() {
        let bytes = fixture();
        let image = parse_pe32(&bytes).unwrap();

        assert_eq!(image.entry_point_rva, 0x1000);
        assert_eq!(image.image_base, 0x0040_0000);
        assert_eq!(image.image_size, 0x3000);
        assert_eq!(image.section_count(), 2);

        let text = image.section(0).unwrap();
        assert_eq!(text.name, b".text\0\0\0");
        assert_eq!(text.virtual_address, 0x1000);
        assert_eq!(text.raw_offset, 0x200);

        let data = image.section(1).unwrap();
        assert_eq!(data.name, b".data\0\0\0");
        assert_eq!(data.virtual_address, 0x2000);

        let summary = image.summary();
        assert_eq!(summary.image_base, 0x0040_0000);
        assert_eq!(summary.entry_point_rva, 0x1000);
        assert_eq!(summary.section_count, 2);
        assert_eq!(summary.first_section_name, *b".text\0\0\0");
        assert_eq!(summary.first_section_characteristics, 0x6000_0020);
    }

    #[test]
    fn rejects_truncated_or_invalid_images() {
        assert!(parse_pe32(b"MZ").is_err());

        let mut bytes = fixture();
        put_u32(&mut bytes, 0x3c, u32::MAX);
        assert!(parse_pe32(&bytes).is_err());

        let mut bytes = fixture();
        put_u32(&mut bytes, 0x80, 0);
        assert!(parse_pe32(&bytes).is_err());

        let mut bytes = fixture();
        put_u32(&mut bytes, 0x178 + 20, u32::MAX - 1);
        assert!(parse_pe32(&bytes).is_err());
    }
}
