#[derive(Clone, Copy)]
/// Borrowed view of a validated PE32 image; sections still reference file bytes.
pub struct PeImage<'a> {
    bytes: &'a [u8],
    section_table_offset: usize,
    optional_header_offset: usize,
    optional_header_size: usize,
    headers_size: usize,
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

    /// Returns PE data-directory entry `index`, when it is present.
    ///
    /// The pair contains a virtual address and byte size. An absent directory
    /// is `None`; malformed optional-header bounds are also treated as absent
    /// here because `parse_pe32` validates the header extent before creating
    /// the image view.
    ///
    /// NOTE: Open-source Contribution Area: add named directory constants and
    /// format-specific validation as additional PE directory types are used.
    pub fn data_directory(&self, index: usize) -> Option<(u32, u32)> {
        if self.optional_header_size < 96 {
            return None;
        }
        let count_offset = self.optional_header_offset.checked_add(92)?;
        let count = read_u32(self.bytes, count_offset).ok()? as usize;
        if index >= count {
            return None;
        }
        let entry_offset = self
            .optional_header_offset
            .checked_add(96)?
            .checked_add(index.checked_mul(8)?)?;
        let entry_end = entry_offset.checked_add(8)?;
        if entry_end
            > self
                .optional_header_offset
                .checked_add(self.optional_header_size)?
        {
            return None;
        }
        Some((
            read_u32(self.bytes, entry_offset).ok()?,
            read_u32(self.bytes, entry_offset + 4).ok()?,
        ))
    }

    /// Returns a file-backed byte range for an RVA wholly within headers or one section.
    ///
    /// This deliberately refuses ranges that cross PE mapping regions or refer
    /// to zero-filled virtual tails, which prevents import parsing from reading
    /// unrelated raw file bytes by treating an RVA as a file offset.
    ///
    /// NOTE: Open-source Contribution Area: support checked ranges spanning
    /// adjacent mapped sections if a future PE directory format requires them.
    pub fn read_rva(&self, rva: u32, length: usize) -> Option<&[u8]> {
        let end_rva = (rva as usize).checked_add(length)?;
        if end_rva <= self.headers_size {
            return self.bytes.get(rva as usize..end_rva);
        }
        for index in 0..self.section_count {
            let section = self.section(index)?;
            let section_start = section.virtual_address as usize;
            let raw_end_rva = section_start.checked_add(section.raw_size as usize)?;
            if rva as usize >= section_start && end_rva <= raw_end_rva {
                let raw_offset =
                    (section.raw_offset as usize).checked_add(rva as usize - section_start)?;
                return self.bytes.get(raw_offset..raw_offset.checked_add(length)?);
            }
        }
        None
    }

    /// Copies PE headers and section bytes into an image-sized, zero-filled buffer.
    ///
    /// Section data is placed at its RVA and virtual-only tails remain zero.
    /// The destination is a staging buffer, not a page mapping and must never
    /// be executed directly by the kernel or treated as a Ring 3 image.
    ///
    /// NOTE: Open-source Contribution Area: add validated relocations and
    /// per-section page permissions before connecting this staging format to
    /// the virtual-memory manager.
    pub fn materialize(&self, destination: &mut [u8]) -> Result<(), &'static str> {
        if destination.len() < self.image_size as usize {
            return Err("PE staging buffer is smaller than SizeOfImage");
        }
        let headers = self
            .bytes
            .get(..self.headers_size)
            .ok_or("PE headers exceed the source file")?;
        destination[..self.image_size as usize].fill(0);
        destination[..self.headers_size].copy_from_slice(headers);

        for index in 0..self.section_count {
            let section = self
                .section(index)
                .ok_or("invalid PE section during materialization")?;
            if section.raw_size == 0 {
                continue;
            }
            let source_start = section.raw_offset as usize;
            let source_end = source_start
                .checked_add(section.raw_size as usize)
                .ok_or("PE section source range overflow")?;
            let source = self
                .bytes
                .get(source_start..source_end)
                .ok_or("PE section source range exceeds file")?;
            let destination_start = section.virtual_address as usize;
            let destination_end = destination_start
                .checked_add(section.raw_size as usize)
                .ok_or("PE section destination range overflow")?;
            let target = destination
                .get_mut(destination_start..destination_end)
                .ok_or("PE section destination range exceeds image")?;
            target.copy_from_slice(source);
        }
        Ok(())
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
///
/// NOTE: Open-source Contribution Area: expand validation as optional-header
/// directories and loader features are implemented.
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
    if optional_size >= 96 {
        let directory_count = read_u32(optional_header, 92)? as usize;
        if directory_count > (optional_size - 96) / 8 {
            return Err("PE data-directory count exceeds the optional header");
        }
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
    if headers_size > image_size as usize {
        return Err("PE headers exceed the declared image size");
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
        optional_header_offset: optional,
        optional_header_size: optional_size,
        headers_size,
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
        if section.virtual_address < headers_size as u32 {
            return Err("section overlaps the PE headers");
        }
        for earlier_index in 0..index {
            let earlier = image
                .section(earlier_index)
                .ok_or("invalid earlier PE section header")?;
            let earlier_end = earlier
                .virtual_address
                .checked_add(earlier.virtual_size.max(earlier.raw_size))
                .ok_or("earlier section virtual address overflow")?;
            if section.virtual_address < earlier_end && earlier.virtual_address < section_end {
                return Err("PE sections overlap in virtual memory");
            }
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
