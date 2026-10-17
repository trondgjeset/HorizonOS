//! Experimental Windows graphics-shaped entry points for native kernel tests.
//!
//! These modules currently provide ABI-shaped stubs and a native rendering
//! demonstration. They do not load PE images, enter from Ring 3, or impersonate
//! a complete Windows graphics subsystem. Exported hooks use the kernel's C
//! ABI; classic 32-bit Windows APIs use stdcall, so a guest import thunk and
//! stack-cleanup bridge are still required before a Windows binary can call
//! these functions.
//!
//! TODO: Add a validated guest-call dispatcher before routing any untrusted
//! application instructions to these kernel functions.

pub mod file_api;
pub mod gdi;
pub mod loader;
pub mod pipeline;
