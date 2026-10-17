//! Hosted tests for desktop icon selection and double-click behavior.
//!
//! NOTE: Geometric UI Rendering: keep host stubs limited to icon-facing
//! dependencies so these tests remain independent from framebuffer hardware.
#![allow(dead_code)]

/// Minimal color tokens and no-op drawing calls required by the icon module.
mod graphics {
    use std::sync::Mutex;

    /// Captures label locations so tests can verify fixed-width centering.
    ///
    /// NOTE: Geometric UI Rendering: keep this recording seam test-only and
    /// independent from kernel framebuffer storage.
    pub(super) static DRAWN_STRINGS: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());

    /// Palette subset consumed by desktop shortcut rendering.
    pub struct TestTheme {
        /// Active selection indicator.
        pub accent_highlight: u32,
        /// Neutral wireframe border.
        pub border_subtle: u32,
        /// Neutral elevated control surface.
        pub control_neutral: u32,
        /// Translucent window-like interior surface.
        pub surface_glass: u32,
        /// Supporting label and detail text.
        pub text_primary: u32,
        /// Secondary symbol details.
        pub text_secondary: u32,
    }

    /// Fixed test palette independent of framebuffer initialization.
    pub const THEME: TestTheme = TestTheme {
        accent_highlight: 0x4a90e2ff,
        border_subtle: 0x394553ff,
        control_neutral: 0x2a3440ff,
        surface_glass: 0x171f2bdd,
        text_primary: 0xe7ebf0ff,
        text_secondary: 0xaeb8c4ff,
    };

    /// Test stub for the rounded-rectangle painter.
    ///
    /// NOTE: Geometric UI Rendering: this stub intentionally avoids framebuffer
    /// access while preserving the icon module's painter signature.
    pub fn draw_rounded_rectangle(
        _x: usize,
        _y: usize,
        _width: usize,
        _height: usize,
        _radius: usize,
        _color: u32,
    ) -> Result<(), &'static str> {
        Ok(())
    }

    /// Test stub for the rectangle painter.
    ///
    /// NOTE: Geometric UI Rendering: this stub keeps vector-symbol tests
    /// independent from kernel graphics initialization.
    pub fn draw_rectangle(
        _x: usize,
        _y: usize,
        _width: usize,
        _height: usize,
        _color: u32,
    ) -> Result<(), &'static str> {
        Ok(())
    }

    /// Test stub for the font renderer.
    ///
    /// NOTE: Geometric UI Rendering: this stub allows label layout code to be
    /// compiled without a framebuffer or embedded font asset.
    pub fn draw_string(x: usize, _y: usize, text: &str, _color: u32) -> Result<(), &'static str> {
        DRAWN_STRINGS
            .lock()
            .map_err(|_| "test label recorder is poisoned")?
            .push((x, text.to_owned()));
        Ok(())
    }
}

/// Pointer click-tracker seam used to keep icon tests independent of x86 I/O.
mod mouse {
    /// Maximum time between a matching pair of clicks.
    pub const DOUBLE_CLICK_TICKS: u32 = 45;

    /// Small host-side stand-in for the production mouse tracker.
    #[derive(Default)]
    pub struct DoubleClickTracker {
        previous: Option<(usize, usize, u32)>,
    }

    impl DoubleClickTracker {
        /// Recognizes nearby clicks with wrap-safe tick arithmetic.
        ///
        /// NOTE: Geometric UI Rendering: mirror the production tracker's
        /// accepted pointer tolerance and tick interval.
        pub fn register_press(&mut self, x: usize, y: usize, tick: u32) -> bool {
            let is_double = self.previous.is_some_and(|(old_x, old_y, old_tick)| {
                x.abs_diff(old_x) <= 8
                    && y.abs_diff(old_y) <= 8
                    && tick.wrapping_sub(old_tick) <= DOUBLE_CLICK_TICKS
            });
            self.previous = if is_double { None } else { Some((x, y, tick)) };
            is_double
        }

        /// Clears pending click history.
        ///
        /// NOTE: Geometric UI Rendering: clear all pending click state when
        /// the desktop focus target changes.
        pub fn clear(&mut self) {
            self.previous = None;
        }
    }
}

#[path = "../src/ui/icons.rs"]
mod icons;

/// Confirms a first click selects, then same-icon click dispatches.
///
/// NOTE: Geometric UI Rendering: selection and pending activation state must
/// agree after both the first and second click.
#[test]
fn same_icon_clicks_within_threshold_dispatch_once() {
    let mut icons = icons::DesktopIcons::new(768);
    let icon = icons.get(0).unwrap();
    assert_eq!(icons.press(icon.x + 1, icon.y + 1, 100), None);
    let selected = icons.get(0).unwrap();
    assert!(selected.is_selected);
    assert_eq!(selected.activation_tick, Some(100));
    assert_eq!(
        icons.press(icon.x + 2, icon.y + 2, 100 + mouse::DOUBLE_CLICK_TICKS),
        Some(icons::IconType::ThisPC)
    );
    assert_eq!(icons.get(0).unwrap().activation_tick, None);
    assert_eq!(icons.press(icon.x + 2, icon.y + 2, 200), None);
}

/// Confirms a different icon or expired tick interval cannot activate.
///
/// NOTE: Geometric UI Rendering: changing desktop targets discards stale
/// activation timing before recording a new first click.
#[test]
fn click_mismatch_and_expiration_do_not_dispatch() {
    let mut icons = icons::DesktopIcons::new(768);
    let first = icons.get(0).unwrap();
    let second = icons.get(1).unwrap();
    assert_eq!(icons.press(first.x + 1, first.y + 1, 10), None);
    assert_eq!(icons.press(second.x + 1, second.y + 1, 11), None);
    assert_eq!(icons.get(0).unwrap().activation_tick, None);
    assert!(icons.get(1).unwrap().is_selected);
    assert_eq!(
        icons.press(
            second.x + 1,
            second.y + 1,
            11 + mouse::DOUBLE_CLICK_TICKS + 1
        ),
        None
    );
}

/// Confirms tick wraparound retains correct elapsed-time semantics.
///
/// NOTE: Geometric UI Rendering: preserve wrap-safe activation behavior across
/// PIT counter rollover.
#[test]
fn double_click_detection_is_wrap_safe() {
    let mut icons = icons::DesktopIcons::new(768);
    let icon = icons.get(2).unwrap();
    assert_eq!(icons.press(icon.x + 1, icon.y + 1, u32::MAX - 10), None);
    assert_eq!(
        icons.press(icon.x + 1, icon.y + 1, 10),
        Some(icons::IconType::RecycleBin)
    );
}

/// Confirms the desktop column is centered in fixed-width cells and avoids the
/// title and taskbar regions on the standard 1024x768 layout.
///
/// NOTE: Geometric UI Rendering: update these expected grid anchors alongside
/// any intentional desktop layout change.
#[test]
fn icons_use_even_vertical_grid_spacing() {
    let icons = icons::DesktopIcons::new(768);
    let top = icons.get(0).unwrap();
    let middle = icons.get(1).unwrap();
    let bottom = icons.get(2).unwrap();

    assert_eq!((top.x, top.y, top.width, top.height), (18, 56, 96, 72));
    assert_eq!(middle.y - top.y, bottom.y - middle.y);
    assert_eq!(bottom.y + bottom.height, 704);
}

/// Confirms a click outside every icon clears selection and any pending
/// double-click activation.
///
/// NOTE: Geometric UI Rendering: retain this reset behavior when adding more
/// interactive desktop layers.
#[test]
fn clicking_empty_desktop_clears_icon_state() {
    let mut icons = icons::DesktopIcons::new(768);
    let icon = icons.get(0).unwrap();
    icons.press(icon.x + 1, icon.y + 1, 1);
    icons.press(0, 0, 2);

    let cleared = icons.get(0).unwrap();
    assert!(!cleared.is_selected);
    assert_eq!(cleared.activation_tick, None);
    assert_eq!(icons.press(icon.x + 1, icon.y + 1, 3), None);
}

/// Confirms each embedded-font label is centered within its 96-pixel icon
/// cell, including the longest built-in label.
///
/// NOTE: Geometric UI Rendering: keep the expected positions in sync with the
/// embedded font's eight-pixel glyph width and desktop cell width.
#[test]
fn icon_labels_are_centered() {
    let mut labels = graphics::DRAWN_STRINGS.lock().unwrap();
    labels.clear();
    drop(labels);

    icons::DesktopIcons::new(768).render().unwrap();
    let labels = graphics::DRAWN_STRINGS.lock().unwrap();

    assert_eq!(
        labels.as_slice(),
        [
            (38, "This PC".to_owned()),
            (22, "User Folder".to_owned()),
            (22, "Recycle Bin".to_owned())
        ]
    );
}
