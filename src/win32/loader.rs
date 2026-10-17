//! Validates and stages PE32 images read from Horizon's read-only `C:\` drive.
//!
//! This loader intentionally stops before executable page mapping, relocation,
//! IAT patching, or scheduler registration. The current virtual-memory manager
//! owns one static user address space for the calculator and cannot allocate
//! or reclaim independent process page directories.
//!
//! NOTE: Open-source Contribution Area: build dynamic user address spaces with
//! owned page tables, per-section permissions, teardown, and user-pointer
//! validation before turning this staging loader into an execution gate.

extern crate alloc;

use crate::pe::{self, PeImage, PeSummary};
use alloc::vec::Vec;

/// Maximum source file size accepted from the RAM disk for this prototype.
///
/// NOTE: Open-source Contribution Area: tune this bound alongside resource
/// accounting once the kernel has a process memory quota mechanism.
const MAX_EXECUTABLE_FILE_SIZE: usize = 16 * 1024 * 1024;

/// Maximum virtual image size copied into the temporary kernel staging buffer.
///
/// NOTE: Open-source Contribution Area: replace this staging limit with
/// per-process committed-page quotas when dynamic address spaces are supported.
const MAX_STAGED_IMAGE_SIZE: usize = 32 * 1024 * 1024;

/// Maximum number of import descriptors accepted in one executable.
///
/// NOTE: Open-source Contribution Area: make the descriptor quota part of
/// process resource policy when the loader supports broader DLL catalogs.
const MAX_IMPORT_DESCRIPTORS: usize = 128;

/// Maximum individual imports accepted across an executable.
///
/// NOTE: Open-source Contribution Area: refine this bound with measured PE
/// workloads and explicit process memory limits.
const MAX_IMPORTS: usize = 512;

/// Maximum byte length for a DLL or imported symbol name, excluding NUL.
const MAX_IMPORT_NAME_LENGTH: usize = 63;

/// Recognized internal endpoint for an imported function name.
///
/// These variants are recognition metadata only. They are not executable
/// addresses and are never written into the image's Import Address Table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookTarget {
    /// Import name matches the experimental GDI `TextOutA` hook.
    GdiTextOutA,
    /// Import name matches the experimental GDI `FillRect` hook.
    GdiFillRect,
    /// Import name matches the nonfunctional Direct3D 9 factory placeholder.
    Direct3DCreate9Placeholder,
    /// No internal compatibility endpoint is registered for this import.
    Unsupported,
}

/// Imported function name or ordinal recorded during import-directory parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportSymbol {
    /// NUL-terminated ASCII import name, copied into a fixed-size field.
    Name {
        /// Name bytes, with unused bytes zero-filled.
        bytes: [u8; MAX_IMPORT_NAME_LENGTH + 1],
        /// Number of meaningful bytes in `bytes`.
        length: u8,
    },
    /// DLL-specific ordinal import.
    Ordinal(u16),
}

/// Import metadata and any matching in-kernel compatibility endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportBinding {
    /// Imported DLL name, copied into a fixed-size field.
    pub dll: [u8; MAX_IMPORT_NAME_LENGTH + 1],
    /// Number of meaningful bytes in `dll`.
    pub dll_length: u8,
    /// Imported name or ordinal.
    pub symbol: ImportSymbol,
    /// A recognized endpoint marker; this is not an IAT address binding.
    pub target: HookTarget,
}

/// PE image bytes arranged by RVA in a temporary kernel-owned staging buffer.
///
/// `image_base` and `entry_point` describe preferred virtual addresses from the
/// PE header. `image` itself is an ordinary kernel heap allocation and is not
/// mapped at those addresses or executable. `imports` records recognizable
/// DLL/function pairs but does not patch the guest IAT.
pub struct LoadedImage {
    /// PE preferred image base from the Optional Header.
    pub image_base: u32,
    /// Entry-point RVA from the Optional Header.
    pub entry_point_rva: u32,
    /// Total PE image size in bytes.
    pub image_size: usize,
    /// Number of validated PE section headers.
    pub section_count: usize,
    /// Validated PE summary copied before the source byte buffer is released.
    summary: PeSummary,
    /// Image bytes placed at RVA offsets in kernel-owned, non-executable memory.
    image: Vec<u8>,
    /// Import records with names classified against the small hook catalog.
    pub imports: Vec<ImportBinding>,
}

impl LoadedImage {
    /// Returns the preferred virtual entry point described by the image.
    ///
    /// The returned address is metadata only; it must not be called or jumped
    /// to until a real process address space and ABI entry path exist.
    ///
    /// NOTE: Open-source Contribution Area: apply relocations when the image
    /// cannot be mapped at its preferred image base.
    pub fn preferred_entry_point(&self) -> Result<u32, &'static str> {
        self.image_base
            .checked_add(self.entry_point_rva)
            .ok_or("PE preferred entry-point address overflow")
    }

    /// Returns the staged image bytes for inspection and loader tests.
    ///
    /// The slice is not an executable mapping and must not be exposed to Ring 3.
    ///
    /// NOTE: Open-source Contribution Area: replace broad inspection access with
    /// a read-only image view once process-owned pages are available.
    pub fn staged_bytes(&self) -> &[u8] {
        &self.image
    }

    /// Copies the PE header summary for existing desktop diagnostics.
    ///
    /// NOTE: Open-source Contribution Area: extend diagnostics with relocation
    /// and import-resolution status when those stages are implemented.
    pub(crate) fn summary(&self) -> PeSummary {
        self.summary
    }
}

/// Reads a Windows-style executable path from the RAM disk and stages its PE32 image.
///
/// The file must exist on the read-only `C:\` drive, be a regular file, and be
/// no larger than `MAX_EXECUTABLE_FILE_SIZE`. The result represents copied and
/// validated image data only: it is not mapped into a process and is not run.
///
/// # Errors
/// Returns an error for missing paths, directories, unsupported PE images,
/// malformed imports, allocation failure, or an image exceeding loader limits.
///
/// NOTE: Open-source Contribution Area: add executable policy checks (signing,
/// architecture, permissions, and resource quotas) before allowing execution.
pub fn load_from_vfs(path: &str) -> Result<LoadedImage, &'static str> {
    let entry = crate::filesystem::resolve_windows_path(path)?;
    if entry.is_directory {
        return Err("executable path names a directory");
    }
    if entry.size == 0 || entry.size > MAX_EXECUTABLE_FILE_SIZE {
        return Err("executable file size is empty or exceeds the loader limit");
    }

    let mut file_bytes = Vec::new();
    file_bytes
        .try_reserve_exact(entry.size)
        .map_err(|_| "not enough heap memory to read executable file")?;
    file_bytes.resize(entry.size, 0);
    let read_length = crate::filesystem::read_file(path, &mut file_bytes)?;
    if read_length != entry.size {
        return Err("VFS executable read returned an unexpected byte count");
    }
    load_pe_bytes(&file_bytes)
}

/// Validates PE32 bytes, creates an RVA-layout staging image, and inspects imports.
///
/// `bytes` must contain one complete PE32 file. Headers are copied to RVA zero;
/// raw section bytes are copied to their virtual-address/RVA offsets, and
/// zero-initialized virtual tails remain zero. The PE preferred image base is
/// retained as metadata but is not used as a physical pointer.
///
/// NOTE: Open-source Contribution Area: add validated base relocations and
/// page-granular section permissions when a dynamic user mapper is available.
pub fn load_pe_bytes(bytes: &[u8]) -> Result<LoadedImage, &'static str> {
    let parsed = pe::parse_pe32(bytes)?;
    let image_size = parsed.image_size as usize;
    if image_size == 0 || image_size > MAX_STAGED_IMAGE_SIZE {
        return Err("PE image size is empty or exceeds the staging limit");
    }

    let mut image = Vec::new();
    image
        .try_reserve_exact(image_size)
        .map_err(|_| "not enough heap memory to stage PE image")?;
    image.resize(image_size, 0);
    parsed.materialize(&mut image)?;
    let imports = parse_imports(&parsed)?;
    let summary = parsed.summary();

    Ok(LoadedImage {
        image_base: parsed.image_base,
        entry_point_rva: parsed.entry_point_rva,
        image_size,
        section_count: parsed.section_count(),
        summary,
        image,
        imports,
    })
}

/// Parses the Import Directory and recognizes the small set of GDI/D3D names.
///
/// Supported names are classified as hook metadata only; this function neither
/// writes the IAT nor provides kernel function pointers to an untrusted image.
/// All import descriptors, names, and thunk arrays are read by validated RVA
/// ranges and are subject to fixed descriptor/symbol/name limits.
///
/// NOTE: Open-source Contribution Area: add DLL catalogs and real per-process
/// import thunks only after guest calls are marshalled through a safe syscall ABI.
pub fn parse_imports(image: &PeImage<'_>) -> Result<Vec<ImportBinding>, &'static str> {
    let Some((directory_rva, directory_size)) = image.data_directory(1) else {
        return Ok(Vec::new());
    };
    if directory_rva == 0 && directory_size == 0 {
        return Ok(Vec::new());
    }
    if directory_rva == 0 || directory_size < IMPORT_DESCRIPTOR_SIZE as u32 {
        return Err("PE import directory has an invalid RVA or size");
    }

    let descriptor_count = directory_size as usize / IMPORT_DESCRIPTOR_SIZE;
    if directory_size as usize % IMPORT_DESCRIPTOR_SIZE != 0 {
        return Err("PE import directory size is not descriptor-aligned");
    }
    if descriptor_count == 0 || descriptor_count > MAX_IMPORT_DESCRIPTORS {
        return Err("PE import descriptor count exceeds the loader limit");
    }
    let mut imports = Vec::new();
    let mut found_terminator = false;

    for descriptor_index in 0..descriptor_count {
        let offset = descriptor_index
            .checked_mul(IMPORT_DESCRIPTOR_SIZE)
            .and_then(|offset| directory_rva.checked_add(offset as u32))
            .ok_or("PE import descriptor RVA overflow")?;
        let descriptor = image
            .read_rva(offset, IMPORT_DESCRIPTOR_SIZE)
            .ok_or("PE import descriptor is not file-backed")?;
        let original_thunk = read_u32(descriptor, 0)?;
        let name_rva = read_u32(descriptor, 12)?;
        let first_thunk = read_u32(descriptor, 16)?;
        if original_thunk == 0 && name_rva == 0 && first_thunk == 0 {
            found_terminator = true;
            break;
        }
        if name_rva == 0 || first_thunk == 0 {
            return Err("PE import descriptor has a missing DLL name or IAT");
        }
        let (dll, dll_length) = read_name(image, name_rva)?;
        let thunk_rva = if original_thunk == 0 {
            first_thunk
        } else {
            original_thunk
        };

        let mut thunk_terminator = false;
        for thunk_index in 0..MAX_IMPORTS {
            let delta = thunk_index
                .checked_mul(core::mem::size_of::<u32>())
                .and_then(|value| u32::try_from(value).ok())
                .ok_or("PE import thunk offset overflow")?;
            let value_rva = thunk_rva
                .checked_add(delta)
                .ok_or("PE import thunk RVA overflow")?;
            let thunk_bytes = image
                .read_rva(value_rva, 4)
                .ok_or("PE import thunk is not file-backed")?;
            let value = read_u32(thunk_bytes, 0)?;
            if value == 0 {
                thunk_terminator = true;
                break;
            }
            if imports.len() >= MAX_IMPORTS {
                return Err("PE import count exceeds the loader limit");
            }

            let symbol = if value & 0x8000_0000 != 0 {
                ImportSymbol::Ordinal((value & 0xffff) as u16)
            } else {
                let hint_and_name = image
                    .read_rva(value, 2)
                    .ok_or("PE import-by-name hint is not file-backed")?;
                let _hint = read_u16(hint_and_name, 0)?;
                let name_rva = value
                    .checked_add(2)
                    .ok_or("PE import-by-name RVA overflow")?;
                let (name, length) = read_name(image, name_rva)?;
                ImportSymbol::Name {
                    bytes: name,
                    length: length as u8,
                }
            };
            let target = recognize_hook(&dll[..dll_length], symbol);
            imports
                .try_reserve(1)
                .map_err(|_| "not enough heap memory for PE imports")?;
            imports.push(ImportBinding {
                dll,
                dll_length: dll_length as u8,
                symbol,
                target,
            });
        }
        if !thunk_terminator {
            return Err("PE import thunk table has no terminator");
        }
    }
    if !found_terminator {
        return Err("PE import directory has no terminating descriptor");
    }
    Ok(imports)
}

/// Classifies supported import names without exposing executable kernel addresses.
///
/// Only GDI32 `TextOutA`/`FillRect` and D3D9 `Direct3DCreate9` are recognized.
/// DLL names are case-insensitive and an optional `.dll` suffix is ignored.
///
/// NOTE: Open-source Contribution Area: expand this table as individual API
/// implementations become safe and documented; never accept arbitrary exports.
fn recognize_hook(dll: &[u8], symbol: ImportSymbol) -> HookTarget {
    let ImportSymbol::Name { bytes, length } = symbol else {
        return HookTarget::Unsupported;
    };
    let name = &bytes[..length as usize];
    if dll.eq_ignore_ascii_case(b"gdi32.dll") || dll.eq_ignore_ascii_case(b"gdi32") {
        if name.eq_ignore_ascii_case(b"TextOutA") {
            return HookTarget::GdiTextOutA;
        }
        if name.eq_ignore_ascii_case(b"FillRect") {
            return HookTarget::GdiFillRect;
        }
    }
    if (dll.eq_ignore_ascii_case(b"d3d9.dll") || dll.eq_ignore_ascii_case(b"d3d9"))
        && name.eq_ignore_ascii_case(b"Direct3DCreate9")
    {
        return HookTarget::Direct3DCreate9Placeholder;
    }
    HookTarget::Unsupported
}

/// Copies a NUL-terminated import string from a file-backed PE RVA.
///
/// Returns a zero-padded fixed buffer and its actual byte length. Empty,
/// unterminated, or non-ASCII names are rejected rather than silently changed.
///
/// NOTE: Open-source Contribution Area: support validated hint/name records
/// and encoding policy for wider Windows import metadata.
fn read_name(
    image: &PeImage<'_>,
    start_rva: u32,
) -> Result<([u8; MAX_IMPORT_NAME_LENGTH + 1], usize), &'static str> {
    let mut result = [0u8; MAX_IMPORT_NAME_LENGTH + 1];
    for index in 0..=MAX_IMPORT_NAME_LENGTH {
        let delta = u32::try_from(index).map_err(|_| "PE import name offset overflow")?;
        let rva = start_rva
            .checked_add(delta)
            .ok_or("PE import name RVA overflow")?;
        let byte = image
            .read_rva(rva, 1)
            .ok_or("PE import name is not file-backed")?[0];
        if byte == 0 {
            if index == 0 {
                return Err("PE import name is empty");
            }
            return Ok((result, index));
        }
        if !byte.is_ascii() {
            return Err("PE import name is not ASCII");
        }
        result[index] = byte;
    }
    Err("PE import name exceeds the loader limit")
}

const IMPORT_DESCRIPTOR_SIZE: usize = 20;

/// Reads a little-endian 16-bit value from an already bounded byte slice.
///
/// NOTE: Open-source Contribution Area: centralize PE integer readers if
/// additional binary formats reuse the same checked decoding logic.
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, &'static str> {
    let value = bytes
        .get(offset..offset.checked_add(2).ok_or("PE field offset overflow")?)
        .ok_or("truncated PE field")?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

/// Reads a little-endian 32-bit value from an already bounded byte slice.
///
/// NOTE: Open-source Contribution Area: centralize PE integer readers if
/// additional binary formats reuse the same checked decoding logic.
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, &'static str> {
    let value = bytes
        .get(offset..offset.checked_add(4).ok_or("PE field offset overflow")?)
        .ok_or("truncated PE field")?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}
