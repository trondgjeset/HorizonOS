//! Tests for fail-closed Win32 file entry points and the native-save capability.

#[path = "../src/win32/file_api.rs"]
mod file_api;

/// Records writes made by the scoped save-grant test.
mod filesystem {
    use std::sync::Mutex;

    /// Last VFS write observed by this standalone API test crate.
    pub static WRITES: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());

    /// Records a kernel-side write to verify the fixed-path capability.
    ///
    /// NOTE: Open-source Contribution Area: replace this test recorder with a
    /// real VFS mount when the file API tests run inside the kernel harness.
    pub fn write_file(path: &str, data: &[u8]) -> Result<(), &'static str> {
        WRITES
            .lock()
            .map_err(|_| "test write recorder is poisoned")?
            .push((path.to_owned(), data.to_vec()));
        Ok(())
    }
}

/// Proves arbitrary ABI pointers are rejected without being dereferenced.
///
/// NOTE: Open-source Contribution Area: add fuzzed pointer-contract testing
/// after a guest-memory marshalling layer exists.
#[test]
fn raw_win32_entry_points_fail_closed() {
    let handle = unsafe {
        file_api::CreateFileA(
            core::ptr::null(),
            file_api::GENERIC_WRITE,
            0,
            core::ptr::null(),
            2,
            0,
            core::ptr::null_mut(),
        )
    };
    assert_eq!(handle, file_api::INVALID_HANDLE_VALUE);
    assert_eq!(
        unsafe {
            file_api::WriteFile(
                core::ptr::null_mut(),
                core::ptr::null(),
                16,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        },
        0
    );
    assert_eq!(file_api::GetLastError(), file_api::ERROR_ACCESS_DENIED);
}

/// Proves the native UI capability can write only the configured notes path.
///
/// NOTE: Open-source Contribution Area: add expiry and process binding when
/// native intent tokens are connected to real process identities.
#[test]
fn native_save_capability_targets_the_user_notes_file() {
    let intent = file_api::authorize_native_save_button();
    file_api::save_with_intent(intent, b"hello from the editor").unwrap();

    let writes = filesystem::WRITES.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, r"C:\Users\Default\Documents\notes.txt");
    assert_eq!(writes[0].1, b"hello from the editor");
}
