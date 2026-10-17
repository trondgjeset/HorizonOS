//! Standalone coverage for the pure pixel-math helpers in the graphics module.
#![allow(dead_code)]

extern crate alloc;

#[path = "../src/font8x8.rs"]
mod font8x8;

/// Supplies the framebuffer mapper symbol required by the included renderer.
///
/// The unit tests exercise pure color and geometry functions without mapping
/// any device memory.
///
/// NOTE: Design Customization: keep this test seam inert unless framebuffer
/// initialization itself is covered by a dedicated hosted test harness.
mod memory {
    /// Returns an error because standalone pixel-math tests never map hardware.
    ///
    /// NOTE: Design Customization: preserve the production mapper signature
    /// if the graphics module gains a different framebuffer setup API.
    pub fn map_framebuffer(
        _physical_address: u64,
        _buffer_size: usize,
    ) -> Result<*mut u8, &'static str> {
        Err("framebuffer mapping is unavailable in hosted unit tests")
    }
}

#[path = "../src/graphics.rs"]
mod graphics;
