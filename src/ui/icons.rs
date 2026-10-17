//! Grid-aligned desktop icons, geometric silhouettes, and double-click state.
//!
//! The icon layer owns only presentation and click state. It returns a typed
//! action to the window manager; it never opens files or mutates VFS state.
//!
//! NOTE: Geometric UI Rendering: keep hit bounds, label centering, and symbol
//! dimensions in the same grid cell when extending this fixed desktop column.

use crate::{graphics, mouse::DoubleClickTracker};

/// Traditional system shortcut represented by one desktop icon.
///
/// NOTE: Geometric UI Rendering: add a matching symbol and label when adding a
/// new first-party shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconType {
    /// Opens the computer and its available virtual drive list.
    ThisPC,
    /// Opens the current user's Documents directory.
    UserFolder,
    /// Opens the virtual sandbox trash directory.
    RecycleBin,
}

/// Grid geometry, selection, and activation timing for one desktop shortcut.
///
/// NOTE: Geometric UI Rendering: preserve the distinction between clickable
/// bounds and the smaller vector-symbol drawing area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesktopIcon {
    /// Left edge of the clickable icon-and-label bounds.
    pub x: usize,
    /// Top edge of the clickable icon-and-label bounds.
    pub y: usize,
    /// Width of the icon-and-label bounds.
    pub width: usize,
    /// Height of the icon-and-label bounds.
    pub height: usize,
    /// Whether this icon is currently selected.
    pub is_selected: bool,
    /// Built-in semantic action represented by this icon.
    pub icon_type: IconType,
    /// PIT tick of the latest first click awaiting a possible second click.
    pub activation_tick: Option<u32>,
}

/// Owns the fixed desktop icon grid and most-recent icon click.
///
/// NOTE: Geometric UI Rendering: keep icon data statically bounded until the
/// shell supports user-configurable shortcut storage.
pub struct DesktopIcons {
    icons: [DesktopIcon; 3],
    click_tracker: DoubleClickTracker,
}

impl DesktopIcons {
    /// Creates a left-edge icon column positioned above the taskbar.
    ///
    /// `screen_height` determines how much vertical spacing is available.
    ///
    /// NOTE: Geometric UI Rendering: distribute the three cells evenly between
    /// the title-safe top margin and taskbar-safe bottom margin.
    pub fn new(screen_height: usize) -> Self {
        const ICON_HEIGHT: usize = 72;
        const FIRST_Y: usize = 56;
        const TASKBAR_HEIGHT: usize = 56;
        const BOTTOM_MARGIN: usize = 8;
        let last_y = screen_height
            .saturating_sub(TASKBAR_HEIGHT + BOTTOM_MARGIN)
            .saturating_sub(ICON_HEIGHT)
            .max(FIRST_Y);
        let spacing = last_y.saturating_sub(FIRST_Y) / 2;
        let icon = |row: usize, icon_type| DesktopIcon {
            x: 18,
            y: FIRST_Y.saturating_add(row.saturating_mul(spacing)),
            width: 96,
            height: ICON_HEIGHT,
            is_selected: false,
            icon_type,
            activation_tick: None,
        };

        Self {
            icons: [
                icon(0, IconType::ThisPC),
                icon(1, IconType::UserFolder),
                icon(2, IconType::RecycleBin),
            ],
            click_tracker: DoubleClickTracker::default(),
        }
    }

    /// Paints selected backgrounds, geometric symbols, and labels.
    ///
    /// This runs immediately after wallpaper composition and before windows so
    /// desktop icons remain behind floating application surfaces.
    ///
    /// NOTE: Geometric UI Rendering: use shared `Theme` tokens and bounded
    /// primitives for every state and silhouette.
    pub fn render(&self) -> Result<(), &'static str> {
        for icon in self.icons {
            if icon.is_selected {
                graphics::draw_rounded_rectangle(
                    icon.x.saturating_sub(4),
                    icon.y.saturating_sub(4),
                    icon.width.saturating_add(8),
                    icon.height.saturating_add(8),
                    8,
                    (graphics::THEME.accent_highlight & 0xffff_ff00) | 0x58,
                )?;
            }
            self.render_symbol(icon)?;
            let label = match icon.icon_type {
                IconType::ThisPC => "This PC",
                IconType::UserFolder => "User Folder",
                IconType::RecycleBin => "Recycle Bin",
            };
            let label_width = label.len().saturating_mul(8);
            let label_x = icon
                .x
                .saturating_add(icon.width.saturating_sub(label_width) / 2);
            graphics::draw_string(
                label_x,
                icon.y.saturating_add(57),
                label,
                graphics::THEME.text_primary,
            )?;
        }
        Ok(())
    }

    /// Selects an icon and returns its type only on a matching double click.
    ///
    /// Clicks must land within the same icon bounds and within the mouse
    /// controller's [`crate::mouse::DOUBLE_CLICK_TICKS`]. The controller uses
    /// unsigned wrapping subtraction across PIT counter rollover.
    ///
    /// NOTE: Geometric UI Rendering: dispatch only when both clicks resolve to
    /// the same icon bounds and fit the controller's timing and movement rule.
    pub fn press(&mut self, x: usize, y: usize, tick: u32) -> Option<IconType> {
        let hit = self.icons.iter().position(|icon| {
            x >= icon.x
                && x < icon.x.saturating_add(icon.width)
                && y >= icon.y
                && y < icon.y.saturating_add(icon.height)
        });
        let Some(index) = hit else {
            self.clear_selection();
            return None;
        };

        for (icon_index, icon) in self.icons.iter_mut().enumerate() {
            icon.is_selected = icon_index == index;
            if icon_index != index {
                icon.activation_tick = None;
            }
        }
        let icon_type = self.icons[index].icon_type;
        let previous_tick = self.icons[index].activation_tick;
        let nearby_double_click = self.click_tracker.register_press(x, y, tick);
        let is_double_click = nearby_double_click
            && previous_tick.is_some_and(|previous| {
                tick.wrapping_sub(previous) <= crate::mouse::DOUBLE_CLICK_TICKS
            });
        if is_double_click {
            self.icons[index].activation_tick = None;
            Some(icon_type)
        } else {
            self.icons[index].activation_tick = Some(tick);
            None
        }
    }

    /// Clears icon selection when the user clicks elsewhere on the desktop.
    ///
    /// NOTE: Geometric UI Rendering: clear visual selection and pending click
    /// timing when another desktop target takes interaction focus.
    pub fn clear_selection(&mut self) {
        for icon in &mut self.icons {
            icon.is_selected = false;
            icon.activation_tick = None;
        }
        self.click_tracker.clear();
    }

    /// Returns the icon occupying `index`, for layout tests and accessibility.
    ///
    /// NOTE: Geometric UI Rendering: keep this inspection helper test-only so
    /// runtime callers use hit testing rather than icon storage order.
    #[cfg(test)]
    pub fn get(&self, index: usize) -> Option<DesktopIcon> {
        self.icons.get(index).copied()
    }

    /// Draws the monitor, folder, or trash-bin symbol for one icon.
    ///
    /// NOTE: Geometric UI Rendering: preserve the fixed primitive bounds when
    /// revising symbol silhouettes for different framebuffer sizes.
    fn render_symbol(&self, icon: DesktopIcon) -> Result<(), &'static str> {
        let x = icon.x.saturating_add(icon.width.saturating_sub(40) / 2);
        let y = icon.y.saturating_add(2);
        match icon.icon_type {
            IconType::ThisPC => {
                graphics::draw_rounded_rectangle(x, y, 36, 27, 4, graphics::THEME.border_subtle)?;
                graphics::draw_rounded_rectangle(
                    x.saturating_add(3),
                    y.saturating_add(3),
                    30,
                    20,
                    2,
                    graphics::THEME.surface_glass,
                )?;
                graphics::draw_rounded_rectangle(
                    x.saturating_add(6),
                    y.saturating_add(6),
                    24,
                    2,
                    1,
                    graphics::THEME.accent_highlight,
                )?;
                graphics::draw_rectangle(
                    x.saturating_add(16),
                    y.saturating_add(27),
                    4,
                    8,
                    graphics::THEME.text_secondary,
                )?;
                graphics::draw_rounded_rectangle(
                    x.saturating_add(10),
                    y.saturating_add(34),
                    16,
                    3,
                    1,
                    graphics::THEME.border_subtle,
                )?;
                graphics::draw_rectangle(
                    x.saturating_add(17),
                    y.saturating_add(34),
                    3,
                    2,
                    graphics::THEME.accent_highlight,
                )?;
            }
            IconType::UserFolder => {
                graphics::draw_rounded_rectangle(
                    x,
                    y.saturating_add(8),
                    40,
                    28,
                    4,
                    graphics::THEME.border_subtle,
                )?;
                graphics::draw_rounded_rectangle(
                    x.saturating_add(3),
                    y.saturating_add(13),
                    34,
                    20,
                    3,
                    graphics::THEME.control_neutral,
                )?;
                graphics::draw_rounded_rectangle(
                    x.saturating_add(3),
                    y.saturating_add(5),
                    16,
                    8,
                    3,
                    graphics::THEME.border_subtle,
                )?;
                graphics::draw_rounded_rectangle(
                    x.saturating_add(5),
                    y.saturating_add(15),
                    30,
                    16,
                    2,
                    graphics::THEME.surface_glass,
                )?;
            }
            IconType::RecycleBin => {
                graphics::draw_rounded_rectangle(
                    x.saturating_add(7),
                    y.saturating_add(10),
                    26,
                    27,
                    4,
                    graphics::THEME.border_subtle,
                )?;
                graphics::draw_rounded_rectangle(
                    x.saturating_add(10),
                    y.saturating_add(14),
                    20,
                    19,
                    2,
                    graphics::THEME.surface_glass,
                )?;
                graphics::draw_rectangle(
                    x.saturating_add(5),
                    y.saturating_add(7),
                    30,
                    3,
                    graphics::THEME.text_secondary,
                )?;
                graphics::draw_rectangle(
                    x.saturating_add(15),
                    y.saturating_add(3),
                    10,
                    3,
                    graphics::THEME.border_subtle,
                )?;
                for offset in [15, 21, 27] {
                    graphics::draw_rectangle(
                        x.saturating_add(offset),
                        y.saturating_add(18),
                        1,
                        10,
                        graphics::THEME.text_secondary,
                    )?;
                }
            }
        }
        Ok(())
    }
}
