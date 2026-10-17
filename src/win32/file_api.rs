//! Fail-closed Win32 file API names and a scoped native-shell save capability.
//!
//! The current kernel has no Ring 3 syscall dispatcher or safe user-memory
//! copying routine. Therefore the raw-pointer Win32 ABI entry points reject
//! every call without reading guest pointers. The native Save button uses a
//! separate, narrow capability path that can write only its fixed document.
//!
//! NOTE: Open-source Contribution Area: add syscall-backed guest pointer
//! marshalling and process identity before permitting any Ring 3 file API call.

use core::ffi::{c_char, c_void};

/// Win32 `ERROR_ACCESS_DENIED`, returned for all calls rejected at this gate.
pub const ERROR_ACCESS_DENIED: u32 = 5;

/// Win32 `GENERIC_WRITE` access bit for CreateFileA requests.
pub const GENERIC_WRITE: u32 = 0x4000_0000;

/// Win32 invalid-handle sentinel used by the C-compatible entry stub.
pub const INVALID_HANDLE_VALUE: *mut c_void = usize::MAX as *mut c_void;

/// Fixed user-document path authorized by the native Save button.
pub const USER_NOTES_PATH: &str = r"C:\Users\Default\Documents\notes.txt";

/// Unforgeable-outside-the-kernel grant scoped to the native Save button target.
///
/// The field is private and the grant is not exposed through a C ABI. It is a
/// kernel call-site capability, not a claim that the current Ring 3 task has a
/// process-specific GUI intent identity.
pub(crate) struct NativeSaveIntent {
    target_path: &'static str,
}

/// Issues the narrow save grant after the native desktop handles its Save click.
///
/// This token is intentionally limited to one fixed VFS destination. It is not
/// an authorization path for untrusted Windows processes.
///
/// NOTE: Open-source Contribution Area: replace this kernel-internal capability
/// with a process-bound, revocable user-intent grant when GUI broker syscalls
/// and application identities are implemented.
pub(crate) fn authorize_native_save_button() -> NativeSaveIntent {
    NativeSaveIntent {
        target_path: USER_NOTES_PATH,
    }
}

/// Stores bytes for the fixed notes document using a consumed native UI grant.
///
/// The VFS copies data into its bounded heap overlay transactionally. The grant
/// cannot select another path, and failures are returned to the caller instead
/// of being reported as success.
///
/// NOTE: Open-source Contribution Area: bind grants to a file-picker selection
/// and prompt state when native file picker UI exists.
pub(crate) fn save_with_intent(intent: NativeSaveIntent, data: &[u8]) -> Result<(), &'static str> {
    if intent.target_path != USER_NOTES_PATH {
        return Err("save intent does not authorize the notes document");
    }
    crate::filesystem::write_file(intent.target_path, data)
}

/// Win32-shaped `CreateFileA` entry that denies calls until safe guest marshalling exists.
///
/// Every call returns `INVALID_HANDLE_VALUE`. No argument is dereferenced,
/// because an incoming Ring 3 pointer cannot safely be read by this prototype.
/// The corresponding `GetLastError` stub reports `ERROR_ACCESS_DENIED`.
///
/// # Safety
/// The function is safe to call with arbitrary pointers because it does not
/// access them. It does not provide a usable Windows file handle.
///
/// NOTE: Open-source Contribution Area: implement access masks, share modes,
/// disposition semantics, process handles, and safe user-string copying.
#[no_mangle]
pub unsafe extern "C" fn CreateFileA(
    _file_name: *const c_char,
    _desired_access: u32,
    _share_mode: u32,
    _security_attributes: *const c_void,
    _creation_disposition: u32,
    _flags_and_attributes: u32,
    _template_file: *mut c_void,
) -> *mut c_void {
    INVALID_HANDLE_VALUE
}

/// Win32-shaped `WriteFile` entry that rejects all guest writes by default.
///
/// This prototype returns zero (FALSE), does not dereference the handle,
/// buffer, overlapped structure, or byte-count pointer, and leaves the last
/// error as access denied through the fail-closed `GetLastError` stub.
///
/// # Safety
/// The function is safe to call with arbitrary pointers because it does not
/// access them. It does not write to any file or output parameter.
///
/// NOTE: Open-source Contribution Area: validate handle ownership and copy
/// guest buffers through checked address-space access before implementing writes.
#[no_mangle]
pub unsafe extern "C" fn WriteFile(
    _file: *mut c_void,
    _buffer: *const c_void,
    _bytes_to_write: u32,
    _bytes_written: *mut u32,
    _overlapped: *mut c_void,
) -> i32 {
    0
}

/// Returns access denied for the current fail-closed Win32 file API gate.
///
/// This is not thread-local yet; all file API calls currently fail with the
/// same status because no guest-call dispatcher or successful file handle
/// implementation exists.
///
/// NOTE: Open-source Contribution Area: make last-error state thread-local and
/// set it through the guest ABI once per-process thread state is available.
#[no_mangle]
pub extern "C" fn GetLastError() -> u32 {
    ERROR_ACCESS_DENIED
}
