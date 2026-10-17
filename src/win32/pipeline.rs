//! Fail-closed Direct3D and Vulkan-shaped prototype gateways.
//!
//! This kernel currently has no Ring 3 system-call dispatcher that can safely
//! identify, validate, and copy a graphics request from a guest process. These
//! entry points therefore do not create contexts or allocate guest-visible
//! buffers; unsupported graphics requests fail explicitly.
//!
//! TODO: Add guest ABI marshalling and per-process virtual-memory ownership
//! before wiring these stubs to software or hardware rendering.

use core::ffi::c_void;

/// Minimal mock of the Direct3D 9 factory entry point.
///
/// `sdk_version` is accepted to preserve the familiar C-level factory shape.
/// The function always returns null because there is no COM interface,
/// per-process context, or safe Ring 3 request path to return yet.
///
/// TODO: Implement a version-checked software D3D context only after the kernel
/// can allocate and revoke pages in the calling application's address space.
/// NOTE: This C ABI symbol is not a Windows PE import thunk or stdcall gateway.
#[no_mangle]
pub extern "C" fn Direct3DCreate9(sdk_version: u32) -> *mut c_void {
    // TODO: Once guest isolation exists, translate supported software D3D
    // commands to a backend that can later connect to hardware Vulkan loops.
    let _ = sdk_version;
    core::ptr::null_mut()
}

/// Returns the Vulkan `VK_ERROR_EXTENSION_NOT_PRESENT` result for this stub.
pub const VK_ERROR_EXTENSION_NOT_PRESENT: i32 = -7;

/// Fail-closed placeholder for an eventual Vulkan instance-creation gateway.
///
/// This function has a Horizon-specific name because exporting the real Vulkan
/// symbol without its platform calling convention, full structures, and loader
/// semantics would falsely imply binary compatibility.
///
/// TODO: Validate guest `VkInstanceCreateInfo` memory and negotiate extensions
/// before implementing software rendering or connecting to hardware-accelerated
/// Vulkan loops.
pub extern "C" fn horizon_vk_create_instance_placeholder() -> i32 {
    // TODO: Replace this explicit failure only after guest structures and
    // extensions are validated for software or hardware Vulkan dispatch.
    VK_ERROR_EXTENSION_NOT_PRESENT
}

#[cfg(test)]
mod tests {
    //! Regression tests for the explicit, fail-closed pipeline behavior.

    use super::{horizon_vk_create_instance_placeholder, Direct3DCreate9};

    /// Does not advertise a Direct3D context that has not actually been created.
    #[test]
    fn direct3d_factory_fails_closed() {
        assert!(Direct3DCreate9(32).is_null());
    }

    /// Reports an unsupported Vulkan extension instead of returning a fake handle.
    #[test]
    fn vulkan_placeholder_reports_unsupported() {
        assert_eq!(horizon_vk_create_instance_placeholder(), -7);
    }
}
