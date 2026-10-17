//! Standalone integration tests for PE staging and the VFS loader boundary.
//!
//! The fixture is deliberately a harmless data-only PE32 image. Tests verify
//! section layout and import classification without mapping or executing it.

extern crate alloc;

#[path = "../src/win32/loader.rs"]
mod loader;
#[path = "../src/pe.rs"]
mod pe;

/// Builds a small PE32 fixture with `.text`, `.idata`, and two GDI imports.
///
/// NOTE: Open-source Contribution Area: add fixtures for malformed data
/// directories, relocation blocks, and permission edge cases as those formats
/// become supported.
fn test_fixture() -> Vec<u8> {
    let mut bytes = vec![0u8; 0x800];
    put_u16(&mut bytes, 0, 0x5a4d);
    put_u32(&mut bytes, 0x3c, 0x80);
    put_u32(&mut bytes, 0x80, 0x0000_4550);

    let coff = 0x84;
    put_u16(&mut bytes, coff, 0x014c);
    put_u16(&mut bytes, coff + 2, 2);
    put_u16(&mut bytes, coff + 16, 224);

    let optional = coff + 20;
    put_u16(&mut bytes, optional, 0x010b);
    put_u32(&mut bytes, optional + 16, 0x1000);
    put_u32(&mut bytes, optional + 28, 0x0040_0000);
    put_u32(&mut bytes, optional + 32, 0x1000);
    put_u32(&mut bytes, optional + 36, 0x200);
    put_u32(&mut bytes, optional + 56, 0x3000);
    put_u32(&mut bytes, optional + 60, 0x200);
    put_u32(&mut bytes, optional + 92, 16);
    put_u32(&mut bytes, optional + 104, 0x2000);
    put_u32(&mut bytes, optional + 108, 40);

    let sections = optional + 224;
    bytes[sections..sections + 8].copy_from_slice(b".text\0\0\0");
    put_u32(&mut bytes, sections + 8, 0x20);
    put_u32(&mut bytes, sections + 12, 0x1000);
    put_u32(&mut bytes, sections + 16, 0x200);
    put_u32(&mut bytes, sections + 20, 0x200);
    put_u32(&mut bytes, sections + 36, 0x6000_0020);
    bytes[0x200..0x20c].copy_from_slice(b"SAFE-PE-DATA");

    let idata = sections + 40;
    bytes[idata..idata + 8].copy_from_slice(b".idata\0\0");
    put_u32(&mut bytes, idata + 8, 0x400);
    put_u32(&mut bytes, idata + 12, 0x2000);
    put_u32(&mut bytes, idata + 16, 0x400);
    put_u32(&mut bytes, idata + 20, 0x400);
    put_u32(&mut bytes, idata + 36, 0xc000_0040);

    put_u32(&mut bytes, 0x400, 0x2040);
    put_u32(&mut bytes, 0x40c, 0x2080);
    put_u32(&mut bytes, 0x410, 0x2060);
    put_u32(&mut bytes, 0x440, 0x20a0);
    put_u32(&mut bytes, 0x444, 0x20b0);
    put_u32(&mut bytes, 0x448, 0);
    bytes[0x480..0x48a].copy_from_slice(b"GDI32.DLL\0");
    bytes[0x4a0..0x4aa].copy_from_slice(b"\0\0TextOutA");
    bytes[0x4b0..0x4ba].copy_from_slice(b"\0\0FillRect");
    bytes
}

/// Writes a little-endian 16-bit integer into a bounds-known test fixture.
///
/// NOTE: Open-source Contribution Area: share fixture-writing helpers if other
/// binary-format tests need the same construction primitives.
fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// Writes a little-endian 32-bit integer into a bounds-known test fixture.
///
/// NOTE: Open-source Contribution Area: share fixture-writing helpers if other
/// binary-format tests need the same construction primitives.
fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Provides the deterministic PE fixture through the loader's VFS boundary.
mod filesystem {
    /// Minimal file metadata used by the standalone loader test.
    pub struct EntryInfo {
        /// Fixture byte length.
        pub size: usize,
        /// The test image is always a regular file.
        pub is_directory: bool,
    }

    /// Resolves only the path used by the VFS-loader integration test.
    ///
    /// NOTE: Open-source Contribution Area: add path normalization and directory
    /// cases when the test harness models more of the production VFS.
    pub fn resolve_windows_path(path: &str) -> Result<EntryInfo, &'static str> {
        if !path.eq_ignore_ascii_case(r"C:\Apps\TEST.EXE") {
            return Err("file was not found on C:\\");
        }
        Ok(EntryInfo {
            size: crate::test_fixture().len(),
            is_directory: false,
        })
    }

    /// Copies the static test image into the caller's validated buffer.
    ///
    /// NOTE: Open-source Contribution Area: add read-failure and short-read
    /// injection once loader error-path coverage is expanded.
    pub fn read_file(path: &str, destination: &mut [u8]) -> Result<usize, &'static str> {
        if !path.eq_ignore_ascii_case(r"C:\Apps\TEST.EXE") {
            return Err("file was not found on C:\\");
        }
        let fixture = crate::test_fixture();
        if destination.len() < fixture.len() {
            return Err("destination buffer is too small for the file");
        }
        destination[..fixture.len()].copy_from_slice(&fixture);
        Ok(fixture.len())
    }
}

/// Stages the VFS fixture at RVA offsets and recognizes its two supported GDI imports.
///
/// NOTE: Open-source Contribution Area: assert relocation, permission, and
/// process-mapping results when the VMM supports dynamic user images.
#[test]
fn stages_vfs_pe_and_classifies_imports() {
    let image = loader::load_from_vfs(r"C:\Apps\TEST.EXE").unwrap();
    assert_eq!(image.image_base, 0x0040_0000);
    assert_eq!(image.preferred_entry_point().unwrap(), 0x0040_1000);
    assert_eq!(image.image_size, 0x3000);
    assert_eq!(image.section_count, 2);
    assert_eq!(image.summary().image_size, 0x3000);
    assert_eq!(&image.staged_bytes()[0x1000..0x100c], b"SAFE-PE-DATA");
    assert_eq!(image.imports.len(), 2);
    assert_eq!(image.imports[0].target, loader::HookTarget::GdiTextOutA);
    assert_eq!(image.imports[1].target, loader::HookTarget::GdiFillRect);
}

/// Rejects a malformed VFS file rather than returning a success-shaped stub.
///
/// NOTE: Open-source Contribution Area: extend malformed-input coverage using
/// property-based PE mutation tests once that tooling is available.
#[test]
fn rejects_non_pe_bytes() {
    assert!(loader::load_pe_bytes(b"not a PE image").is_err());
}
