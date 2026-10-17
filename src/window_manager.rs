use crate::{
    clock::DateTime,
    filesystem::EntryInfo,
    graphics,
    mouse::MouseState,
    pe::PeSummary,
    ui::icons::{DesktopIcons, IconType},
};
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};

const START_BUTTON_INDEX: usize = 2;
/// Reserved compositor slot populated only when the user opens System Utility.
const MAIN_WINDOW_INDEX: usize = 3;
const START_MENU_INDEX: usize = 4;

const TITLE_BAR_HEIGHT: usize = 32;
const TASKBAR_HEIGHT: usize = 56;
const APPLICATION_WIDTH: usize = 460;
const APPLICATION_HEIGHT: usize = 300;
const APPLICATION_CONTENT_WIDTH: usize = APPLICATION_WIDTH - 8;
const APPLICATION_CONTENT_HEIGHT: usize = APPLICATION_HEIGHT - TITLE_BAR_HEIGHT - 4;
const MENU_WIDTH: usize = 280;
const MENU_HEIGHT: usize = 280;
const MENU_ITEM_COUNT: usize = 6;
const MENU_ITEM_TOP: usize = 34;
const MENU_ITEM_SPACING: usize = 36;
const MENU_ITEM_HEIGHT: usize = 32;
const SAVE_BUTTON_WIDTH: usize = 84;
const SAVE_BUTTON_HEIGHT: usize = 26;
const SAVE_BUTTON_MARGIN: usize = 16;
const FILE_BROWSER_WIDTH: usize = 460;
const FILE_BROWSER_HEIGHT: usize = 320;
const FILE_BROWSER_CONTENT_WIDTH: usize = FILE_BROWSER_WIDTH - 8;
const FILE_BROWSER_CONTENT_HEIGHT: usize = FILE_BROWSER_HEIGHT - TITLE_BAR_HEIGHT - 4;
const SYSTEM_CONTROL_WIDTH: usize = 440;
const SYSTEM_CONTROL_HEIGHT: usize = 300;
const SYSTEM_CONTROL_CONTENT_WIDTH: usize = SYSTEM_CONTROL_WIDTH - 8;
const SYSTEM_CONTROL_CONTENT_HEIGHT: usize = SYSTEM_CONTROL_HEIGHT - TITLE_BAR_HEIGHT - 4;
const WIN32_DEMO_WIDTH: usize = 460;
const WIN32_DEMO_HEIGHT: usize = 300;
const TASKBAR_TAB_X: usize = 140;
const TASKBAR_TAB_WIDTH: usize = 112;
const TASKBAR_TAB_SPACING: usize = 8;
const CONTROL_SIZE: usize = 14;
const CONTROL_GAP: usize = 2;
const FILE_DOUBLE_CLICK_TICKS: u32 = 45;
const FILE_LIST_TOP: usize = 22;
const FILE_LIST_ROW_HEIGHT: usize = 18;
const FILE_LIST_VISIBLE_ROWS: usize = 12;
const WINDOW_CORNER_RADIUS: usize = 10;
/// PIT tick interval for each visible/hidden caret phase at the 100 Hz timer rate.
///
/// NOTE: Contribution Area: tune this interval to change the text caret blink
/// rhythm while preserving tick-based timing.
pub const CARET_BLINK_TICKS: u32 = 50;

/// Index of the currently focused slot, shared with the keyboard event path.
pub static FOCUSED_WINDOW_ID: AtomicUsize = AtomicUsize::new(usize::MAX);
const APPLICATION_CONTENT: [u32; APPLICATION_CONTENT_WIDTH * APPLICATION_CONTENT_HEIGHT] =
    [graphics::THEME.surface_window; APPLICATION_CONTENT_WIDTH * APPLICATION_CONTENT_HEIGHT];
const MENU_CONTENT: [u32; MENU_WIDTH * MENU_HEIGHT] = [0; MENU_WIDTH * MENU_HEIGHT];
const FILE_BROWSER_CONTENT: [u32; FILE_BROWSER_CONTENT_WIDTH * FILE_BROWSER_CONTENT_HEIGHT] =
    [graphics::THEME.surface_window; FILE_BROWSER_CONTENT_WIDTH * FILE_BROWSER_CONTENT_HEIGHT];
const SYSTEM_CONTROL_CONTENT: [u32; SYSTEM_CONTROL_CONTENT_WIDTH * SYSTEM_CONTROL_CONTENT_HEIGHT] =
    [graphics::THEME.surface_window; SYSTEM_CONTROL_CONTENT_WIDTH * SYSTEM_CONTROL_CONTENT_HEIGHT];

#[derive(Clone, Copy, PartialEq, Eq)]
/// Semantic role used to select each window's hit testing and renderer.
enum WindowKind {
    Empty,
    Desktop,
    Taskbar,
    StartButton,
    Application,
    StartMenu,
    /// Browses one configured virtual filesystem directory.
    FileBrowser,
    /// Lists mounted and placeholder drive roots.
    Computer,
    SystemControl,
    TextViewer,
    Win32Demo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Current presentation state for a window.
pub enum WindowViewState {
    Normal,
    Maximized,
    Minimized,
}

#[derive(Clone, Copy)]
/// Title-bar control action selected by hit testing.
enum WindowControl {
    Minimize,
    Maximize,
    Close,
}

#[derive(Clone, Copy)]
/// Geometry preserved while a window is maximized.
pub struct SavedGeometry {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Action returned to the desktop executive after a user interaction.
pub enum UiAction {
    /// Displays metadata from the boot-module PE parser demonstration.
    RunSamplePe,
    /// Opens the native framebuffer demo for the experimental GDI hooks.
    RunWin32Demo,
    /// Reads and stages the fixed PE demonstration file from the RAM disk.
    LaunchExternalApp,
    /// Opens the on-demand editor and background-calculation display.
    ///
    /// NOTE: Action Routing: returned only when Start Menu hit testing selects
    /// the System Utility row.
    ShowSystemUtility,
    /// Saves the main-window input field after an explicit native button click.
    SaveEditorFile,
    /// Opens a selected regular file in the text viewer.
    OpenTextFile(EntryInfo),
    /// Opens the computer view containing the available drive list.
    OpenThisPC,
    /// Opens the current user's Documents folder.
    OpenUserFolder,
    /// Opens the virtual sandbox trash folder.
    OpenRecycleBin,
}

/// Editable text field embedded in a window's content surface.
struct TextInputField {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    text: String,
    focused: bool,
}

impl TextInputField {
    fn empty() -> Self {
        Self {
            x: 16,
            y: 0,
            width: 0,
            height: 26,
            text: String::new(),
            focused: false,
        }
    }

    fn contains(&self, window: &Window, x: usize, y: usize) -> bool {
        let left = window.x.saturating_add(self.x);
        let top = window.y.saturating_add(self.y);
        self.width > 0
            && x >= left
            && x < left.saturating_add(self.width)
            && y >= top
            && y < top.saturating_add(self.height)
    }
}

/// Desktop window geometry, decoration, backing surface, and dynamic contents.
pub struct Window {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub z_order: usize,
    pub visible: bool,
    pub draggable: bool,
    pub view_state: WindowViewState,
    pub saved_geometry: Option<SavedGeometry>,
    title_bar_height: usize,
    background_color: u32,
    title: &'static str,
    pixel_buffer: &'static [u32],
    buffer_width: usize,
    buffer_height: usize,
    kind: WindowKind,
    text_buffer: String,
    input_field: TextInputField,
    viewer_path: String,
    viewer_contents: String,
    /// VFS directory displayed by this File Explorer instance.
    browser_path: &'static str,
}

#[derive(Clone, Copy)]
/// Active pointer drag and the cursor-to-window offset captured on press.
struct DragState {
    window_index: usize,
    offset_x: usize,
    offset_y: usize,
}

/// Owns desktop windows, z-order, focus, clock state, and pointer interactions.
pub struct WindowManager {
    windows: Vec<Window>,
    screen_width: usize,
    screen_height: usize,
    left_button_was_down: bool,
    drag: Option<DragState>,
    clock: Option<DateTime>,
    clock_error: Option<&'static str>,
    background_counter: u32,
    pe_report: Option<PeSummary>,
    pe_error: Option<&'static str>,
    last_file_click: Option<(EntryInfo, u32)>,
    save_status: Option<&'static str>,
    wallpaper: Option<Vec<u8>>,
    icons: DesktopIcons,
}

impl Window {
    fn new(
        kind: WindowKind,
        title: &'static str,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        z_order: usize,
        visible: bool,
        draggable: bool,
        title_bar_height: usize,
        background_color: u32,
        pixel_buffer: &'static [u32],
        buffer_width: usize,
        buffer_height: usize,
    ) -> Self {
        Self {
            x,
            y,
            width,
            height,
            z_order,
            visible,
            draggable,
            view_state: WindowViewState::Normal,
            saved_geometry: None,
            title_bar_height,
            background_color,
            title,
            pixel_buffer,
            buffer_width,
            buffer_height,
            kind,
            text_buffer: String::new(),
            input_field: TextInputField::empty(),
            viewer_path: String::new(),
            viewer_contents: String::new(),
            browser_path: r"C:\",
        }
    }

    fn empty() -> Self {
        Self::new(
            WindowKind::Empty,
            "",
            0,
            0,
            0,
            0,
            0,
            false,
            false,
            0,
            0,
            &[],
            0,
            0,
        )
    }

    fn contains(&self, x: usize, y: usize) -> bool {
        self.visible
            && x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }

    fn contains_title_bar(&self, x: usize, y: usize) -> bool {
        self.draggable && self.contains(x, y) && y < self.y.saturating_add(self.title_bar_height)
    }
}

impl WindowManager {
    /// Creates the desktop shell with the application slot initially empty.
    ///
    /// `screen_width` and `screen_height` are framebuffer dimensions. Returns
    /// an error when the screen is too small or the initial window vector cannot
    /// be reserved. System Utility is allocated only after a launcher action.
    pub fn new(screen_width: usize, screen_height: usize) -> Result<Self, &'static str> {
        if screen_width < 320 || screen_height < 240 {
            return Err("screen is too small for the desktop layout");
        }

        let taskbar_height = TASKBAR_HEIGHT.min(screen_height);
        let menu_width = MENU_WIDTH.min(screen_width);
        let menu_height = MENU_HEIGHT.min(screen_height.saturating_sub(taskbar_height));

        let mut windows = Vec::new();
        windows
            .try_reserve_exact(8)
            .map_err(|_| "not enough heap memory for desktop windows")?;
        windows.extend([
            Window::new(
                WindowKind::Desktop,
                "Desktop",
                0,
                0,
                screen_width,
                screen_height,
                0,
                true,
                false,
                0,
                graphics::THEME.background_canvas,
                &[],
                0,
                0,
            ),
            Window::new(
                WindowKind::Taskbar,
                "Taskbar",
                0,
                screen_height - taskbar_height,
                screen_width,
                taskbar_height,
                1,
                true,
                false,
                0,
                graphics::THEME.surface_glass,
                &[],
                0,
                0,
            ),
            Window::new(
                WindowKind::StartButton,
                "Start",
                20,
                screen_height.saturating_sub(44),
                32,
                32,
                2,
                true,
                false,
                0,
                graphics::THEME.control_neutral,
                &[],
                0,
                0,
            ),
            // Keep the stable application slot empty until the user launches
            // System Utility from Start; boot shows only the desktop shell.
            Window::empty(),
            Window::new(
                WindowKind::StartMenu,
                "Start Menu",
                12,
                screen_height - taskbar_height - menu_height,
                menu_width,
                menu_height,
                4,
                false,
                false,
                0,
                graphics::THEME.surface_glass,
                &MENU_CONTENT,
                MENU_WIDTH,
                MENU_HEIGHT,
            ),
            Window::empty(),
            Window::empty(),
            Window::empty(),
        ]);

        let manager = Self {
            windows,
            screen_width,
            screen_height,
            left_button_was_down: false,
            drag: None,
            clock: None,
            clock_error: None,
            background_counter: 0,
            pe_report: None,
            pe_error: None,
            last_file_click: None,
            save_status: None,
            wallpaper: None,
            icons: DesktopIcons::new(screen_height),
        };
        FOCUSED_WINDOW_ID.store(usize::MAX, Ordering::Release);
        Ok(manager)
    }

    /// Updates the displayed clock, retaining any RTC error for the UI.
    ///
    /// `result` is the latest clock read or its failure message.
    pub fn update_clock(&mut self, result: Result<DateTime, &'static str>) {
        match result {
            Ok(datetime) => {
                self.clock = Some(datetime);
                self.clock_error = None;
            }
            Err(message) => {
                self.clock = None;
                self.clock_error = Some(message);
            }
        }
    }

    /// Updates the displayed value from the independent background task.
    ///
    /// `counter` is the latest shared-page calculation count. Returns `true`
    /// only when a visible scratchpad needs repainting.
    pub fn update_background_counter(&mut self, counter: u32) -> bool {
        let changed = self.background_counter != counter;
        self.background_counter = counter;
        changed
            && self.windows[MAIN_WINDOW_INDEX].kind == WindowKind::Application
            && self.windows[MAIN_WINDOW_INDEX].visible
    }

    /// Validates and stores VFS wallpaper bytes for subsequent desktop redraws.
    ///
    /// The owned byte vector avoids retaining a borrowed VFS buffer across
    /// event-loop iterations. Decoding remains bounded by the BMP parser.
    ///
    /// NOTE: Contribution Area: add user-selected wallpaper replacement and
    /// aspect-ratio preferences when the desktop settings UI is available.
    pub fn set_wallpaper(&mut self, wallpaper: Vec<u8>) -> Result<(), &'static str> {
        graphics::bmp::Bitmap::parse(&wallpaper)?;
        self.wallpaper = Some(wallpaper);
        Ok(())
    }

    /// Reports whether the primary editor input field currently owns keyboard focus.
    ///
    /// The desktop event loop uses this to avoid repainting solely for a hidden
    /// caret while the user is typing in another window or control.
    ///
    /// NOTE: Contribution Area: route this query through a general focused
    /// control model when additional editable widgets are introduced.
    pub fn input_caret_focused(&self) -> bool {
        self.windows[MAIN_WINDOW_INDEX].kind == WindowKind::Application
            && self.windows[MAIN_WINDOW_INDEX].visible
            && self.windows[MAIN_WINDOW_INDEX].input_field.focused
    }

    /// Saves the editor field to the fixed user document after a native save click.
    ///
    /// The intent capability is created only on this kernel-side action path.
    /// The returned error is also retained for rendering so the user sees that
    /// no file was saved when validation or allocation fails.
    ///
    /// NOTE: Open-source Contribution Area: connect a native file picker and
    /// user-confirmed path grant instead of the current fixed demo destination.
    pub fn save_editor_file(&mut self) -> Result<(), &'static str> {
        let contents = self.windows[MAIN_WINDOW_INDEX].input_field.text.as_bytes();
        let intent = crate::win32::file_api::authorize_native_save_button();
        self.pe_report = None;
        self.pe_error = None;
        match crate::win32::file_api::save_with_intent(intent, contents) {
            Ok(()) => {
                self.save_status = Some("Saved to Users\\Default\\Documents\\notes.txt");
                Ok(())
            }
            Err(message) => {
                self.save_status = Some("Save failed; see kernel debug log");
                Err(message)
            }
        }
    }

    /// Opens System Utility and displays the PE parser result there.
    ///
    /// `result` contains validated PE metadata or the parser's error message.
    /// Returns an allocation error if the on-demand scratchpad cannot be opened.
    pub fn show_pe_result(
        &mut self,
        result: Result<PeSummary, &'static str>,
    ) -> Result<(), &'static str> {
        self.spawn_scratchpad()?;
        match result {
            Ok(report) => {
                self.pe_report = Some(report);
                self.pe_error = None;
            }
            Err(message) => {
                self.pe_report = None;
                self.pe_error = Some(message);
            }
        }
        self.windows[MAIN_WINDOW_INDEX].visible = true;
        self.raise_window(MAIN_WINDOW_INDEX);
        self.set_focus(MAIN_WINDOW_INDEX);
        Ok(())
    }

    /// Opens or restores the editor and calculator display on user request.
    ///
    /// The first invocation allocates the input buffer from the kernel heap;
    /// subsequent invocations restore the same window and retain its contents.
    ///
    /// NOTE: Action Routing: keep creation behind an explicit launcher action.
    ///
    /// NOTE: Open-source Contribution Area: persist utility contents only
    /// after the user has selected an explicit save destination.
    pub fn show_scratchpad(&mut self) -> Result<(), &'static str> {
        self.spawn_scratchpad()
    }

    /// Opens or restores the native window that exercises the Win32 GDI shims.
    ///
    /// The demo invokes the kernel's GDI-shaped functions while composing its
    /// window. It is a kernel-side smoke test, not execution of a Windows guest.
    pub fn show_win32_demo(&mut self) -> Result<(), &'static str> {
        self.spawn_win32_demo()
    }

    /// Opens or restores the computer window for the `This PC` desktop icon.
    ///
    /// NOTE: Open-source Contribution Area: populate additional drive entries
    /// from mounted volumes when Horizon adds mount metadata.
    pub fn show_this_pc(&mut self) -> Result<(), &'static str> {
        self.spawn_computer()
    }

    /// Opens the user's Documents directory in File Explorer.
    ///
    /// NOTE: Open-source Contribution Area: source the home directory from
    /// session identity once user profiles are configurable.
    pub fn show_user_folder(&mut self) -> Result<(), &'static str> {
        self.spawn_file_browser_at(r"C:\Users\Default\Documents")
    }

    /// Opens the virtual sandbox trash directory in File Explorer.
    ///
    /// NOTE: Open-source Contribution Area: connect this view to recoverable
    /// deletion metadata when trash operations are implemented.
    pub fn show_recycle_bin(&mut self) -> Result<(), &'static str> {
        self.spawn_file_browser_at(r"C:\System\Trash")
    }

    /// Reads an indexed file and opens its contents in a new Text Viewer window.
    ///
    /// `entry` is the selected VFS entry. Returns an error for directories,
    /// invalid paths, failed reads, or insufficient heap memory.
    pub fn show_text_file(&mut self, entry: EntryInfo) -> Result<(), &'static str> {
        if entry.is_directory || entry.path_length > entry.path.len() {
            return Err("File Explorer selection is not a file");
        }
        let path = core::str::from_utf8(&entry.path[..entry.path_length])
            .map_err(|_| "selected file path is not valid ASCII")?;
        let mut contents = Vec::new();
        contents
            .try_reserve_exact(entry.size)
            .map_err(|_| "not enough heap memory to read the selected file")?;
        contents.resize(entry.size, 0);
        let length = crate::filesystem::read_file(path, &mut contents)?;
        contents.truncate(length);
        let mut display_contents = String::new();
        display_contents
            .try_reserve_exact(length)
            .map_err(|_| "not enough heap memory for text viewer contents")?;
        for byte in contents {
            display_contents.push(
                if byte.is_ascii_graphic()
                    || byte == b' '
                    || byte == b'\n'
                    || byte == b'\r'
                    || byte == b'\t'
                {
                    byte as char
                } else {
                    '?'
                },
            );
        }

        let width = 600.min(self.screen_width.saturating_sub(24));
        let height = 420.min(self.screen_height.saturating_sub(TASKBAR_HEIGHT));
        let mut viewer = Window::new(
            WindowKind::TextViewer,
            "Text Viewer",
            (self.screen_width - width) / 2,
            (self.screen_height.saturating_sub(TASKBAR_HEIGHT + height)) / 2,
            width,
            height,
            self.next_z_order(),
            true,
            true,
            TITLE_BAR_HEIGHT,
            graphics::THEME.surface_window,
            &[],
            0,
            0,
        );
        viewer
            .viewer_path
            .try_reserve_exact(path.len())
            .map_err(|_| "not enough heap memory for text viewer path")?;
        viewer.viewer_path.push_str(path);
        viewer.viewer_contents = display_contents;
        let index = self.insert_window(viewer)?;
        self.raise_window(index);
        self.set_focus(index);
        Ok(())
    }

    /// Returns the slot index that receives keyboard input.
    pub fn focused_window_id(&self) -> usize {
        FOCUSED_WINDOW_ID.load(Ordering::Acquire)
    }

    /// Routes one ASCII/backspace character to the focused window or input field.
    ///
    /// `character` is a translated byte from the keyboard queue. Returns an
    /// error if the focused dynamic text buffer cannot grow.
    ///
    /// NOTE: Contribution Area: add selection, cursor movement, and
    /// per-control input policies in this task-context editing path.
    pub fn handle_character(&mut self, character: u8) -> Result<(), &'static str> {
        let index = self.focused_window_id();
        if index >= self.windows.len() {
            return Ok(());
        }
        let window = &mut self.windows[index];
        if !window.visible || !window.draggable || window.kind == WindowKind::Empty {
            return Ok(());
        }

        if window.input_field.focused {
            match character {
                0x08 => {
                    window.input_field.text.pop();
                }
                0x20..=0x7e => {
                    window
                        .input_field
                        .text
                        .try_reserve(1)
                        .map_err(|_| "not enough heap memory for keyboard input")?;
                    window.input_field.text.push(character as char);
                }
                _ => {}
            }
            return Ok(());
        }

        match character {
            0x08 => {
                window.text_buffer.pop();
            }
            b'\n' => {
                Self::append_text(window, b'\n')?;
            }
            0x20..=0x7e => {
                Self::append_text(window, character)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Composes wallpaper, desktop icons, and visible windows into the back buffer.
    ///
    /// The retained wallpaper is decoded into the back buffer first, then
    /// Desktop icons are painted directly over the wallpaper before floating
    /// windows; windows then render in z-order. Returns an error if any layer
    /// or the temporary z-order list reports a failure.
    ///
    /// NOTE: Contribution Area: add wallpaper caching/dirty-region redraw once
    /// the compositor tracks stable background damage.
    pub fn render(&self) -> Result<(), &'static str> {
        if let Some(wallpaper) = &self.wallpaper {
            graphics::bmp::draw_wallpaper(wallpaper)?;
        } else {
            graphics::clear_screen(graphics::THEME.background_canvas)?;
        }
        self.icons.render()?;
        let mut order: Vec<usize> = Vec::new();
        order
            .try_reserve_exact(self.windows.len())
            .map_err(|_| "not enough heap memory to compose the desktop")?;
        order.extend(0..self.windows.len());
        // The slot vector is storage order, not stacking order. Sort a temporary
        // index list so rendering does not mutate the compositor's window slots.
        order.sort_unstable_by_key(|index| self.windows[*index].z_order);
        for index in order {
            let window = &self.windows[index];
            if window.visible {
                self.render_window(index, window)?;
            }
        }
        Ok(())
    }

    /// Applies a pointer snapshot to controls, icon clicks, and window dragging.
    ///
    /// `state` contains screen coordinates, button bits, and an optional press
    /// timestamp. Repeated presses on the same icon bounds within the icon
    /// double-click interval return the corresponding desktop action.
    ///
    /// NOTE: Open-source Contribution Area: retain the bounded timestamp and
    /// hit-test flow if more pointer devices or shortcut types are added.
    pub fn handle_mouse(&mut self, state: MouseState) -> Result<Option<UiAction>, &'static str> {
        let left_down = state.buttons & 1 != 0;
        let left_pressed =
            state.left_pressed_at.is_some() || (left_down && !self.left_button_was_down);
        let mut action = None;

        if left_pressed {
            self.drag = None;
            self.windows[MAIN_WINDOW_INDEX].input_field.focused = false;
            if self.windows[START_MENU_INDEX].contains(state.x, state.y) {
                self.icons.clear_selection();
                if let Some(item) = self.start_menu_item_at(state.x, state.y) {
                    self.windows[START_MENU_INDEX].visible = false;
                    match item {
                        0 => self.spawn_file_browser()?,
                        1 => self.spawn_system_control()?,
                        2 => action = Some(UiAction::RunSamplePe),
                        3 => action = Some(UiAction::RunWin32Demo),
                        4 => action = Some(UiAction::LaunchExternalApp),
                        // NOTE: Action Routing: this index must match the
                        // System Utility row rendered by `render_start_menu`.
                        5 => action = Some(UiAction::ShowSystemUtility),
                        _ => {}
                    }
                }
            } else {
                if self.windows[START_MENU_INDEX].visible {
                    self.windows[START_MENU_INDEX].visible = false;
                }
                if let Some(index) = self.taskbar_tab_at(state.x, state.y) {
                    self.icons.clear_selection();
                    self.restore_window(index);
                } else if self.windows[START_BUTTON_INDEX].contains(state.x, state.y) {
                    self.icons.clear_selection();
                    self.toggle_start_menu();
                } else if let Some(index) = self.top_window_at(state.x, state.y) {
                    self.icons.clear_selection();
                    self.set_focus(index);
                    if let Some(control) = self.control_at(index, state.x, state.y) {
                        self.apply_window_control(index, control);
                    } else if index == MAIN_WINDOW_INDEX
                        && self.windows[index].input_field.contains(
                            &self.windows[index],
                            state.x,
                            state.y,
                        )
                    {
                        self.windows[index].input_field.focused = true;
                    } else if index == MAIN_WINDOW_INDEX
                        && self.save_button_contains(&self.windows[index], state.x, state.y)
                    {
                        action = Some(UiAction::SaveEditorFile);
                    } else if self.windows[index].kind == WindowKind::FileBrowser {
                        action = self.file_browser_click(
                            index,
                            state.x,
                            state.y,
                            state.left_pressed_at.unwrap_or_else(crate::clock::ticks),
                        )?;
                    } else if self.windows[index].contains_title_bar(state.x, state.y) {
                        let offset_x = state.x.saturating_sub(self.windows[index].x);
                        let offset_y = state.y.saturating_sub(self.windows[index].y);
                        self.drag = Some(DragState {
                            window_index: index,
                            offset_x,
                            offset_y,
                        });
                        self.raise_window(index);
                    }
                } else {
                    match self.icons.press(
                        state.x,
                        state.y,
                        state.left_pressed_at.unwrap_or_else(crate::clock::ticks),
                    ) {
                        Some(IconType::ThisPC) => action = Some(UiAction::OpenThisPC),
                        Some(IconType::UserFolder) => action = Some(UiAction::OpenUserFolder),
                        Some(IconType::RecycleBin) => action = Some(UiAction::OpenRecycleBin),
                        None => {}
                    }
                }
            }
        }

        if left_down {
            if let Some(drag) = self.drag {
                self.move_window(drag, state.x, state.y);
            }
        } else {
            self.drag = None;
        }

        self.left_button_was_down = left_down;
        Ok(action)
    }

    fn append_text(window: &mut Window, character: u8) -> Result<(), &'static str> {
        window
            .text_buffer
            .try_reserve(1)
            .map_err(|_| "not enough heap memory for window text")?;
        window.text_buffer.push(character as char);
        Ok(())
    }

    /// Routes a click to a row in the specific File Explorer window hit-tested.
    ///
    /// `window_index` preserves the selected directory when multiple Explorer
    /// windows are open; only a repeated click on the same file opens a viewer.
    ///
    /// NOTE: Open-source Contribution Area: retain per-window row identity when
    /// adding sorting or pagination to directory listings.
    fn file_browser_click(
        &mut self,
        window_index: usize,
        mouse_x: usize,
        mouse_y: usize,
        click_tick: u32,
    ) -> Result<Option<UiAction>, &'static str> {
        let Some(window) = self.windows.get(window_index) else {
            self.last_file_click = None;
            return Ok(None);
        };
        if window.kind != WindowKind::FileBrowser {
            self.last_file_click = None;
            return Err("File Explorer click targeted a different window kind");
        }
        let list_top = window
            .y
            .saturating_add(TITLE_BAR_HEIGHT + 16 + FILE_LIST_TOP);
        let Some(relative_y) = mouse_y.checked_sub(list_top) else {
            self.last_file_click = None;
            return Ok(None);
        };
        if relative_y / FILE_LIST_ROW_HEIGHT >= FILE_LIST_VISIBLE_ROWS
            || mouse_x < window.x.saturating_add(60)
            || mouse_x >= window.x.saturating_add(window.width).saturating_sub(8)
        {
            self.last_file_click = None;
            return Ok(None);
        }

        let index = relative_y / FILE_LIST_ROW_HEIGHT;
        let browser_path = window.browser_path;
        let Some(entry) = browser_child_at(browser_path, index)? else {
            self.last_file_click = None;
            return Ok(None);
        };
        if entry.is_directory {
            self.last_file_click = None;
            return Ok(None);
        }

        let is_double_click = self
            .last_file_click
            .is_some_and(|(previous, previous_tick)| {
                // Require the same indexed entry within a wrap-safe PIT interval.
                previous == entry
                    && click_tick.wrapping_sub(previous_tick) <= FILE_DOUBLE_CLICK_TICKS
            });
        if is_double_click {
            self.last_file_click = None;
            Ok(Some(UiAction::OpenTextFile(entry)))
        } else {
            self.last_file_click = Some((entry, click_tick));
            Ok(None)
        }
    }

    fn set_focus(&self, index: usize) {
        if index < self.windows.len()
            && self.windows[index].visible
            && self.windows[index].draggable
            && self.windows[index].kind != WindowKind::Empty
        {
            FOCUSED_WINDOW_ID.store(index, Ordering::Release);
        }
    }

    fn choose_fallback_focus(&self) {
        // When the focused window closes or minimizes, focus the visible,
        // keyboard-capable window with the greatest stacking order.
        let mut top = None;
        for index in 0..self.windows.len() {
            let window = &self.windows[index];
            if window.visible
                && window.draggable
                && window.kind != WindowKind::Empty
                && top.is_none_or(|current: usize| self.windows[current].z_order < window.z_order)
            {
                top = Some(index);
            }
        }
        if let Some(index) = top {
            FOCUSED_WINDOW_ID.store(index, Ordering::Release);
        } else {
            FOCUSED_WINDOW_ID.store(usize::MAX, Ordering::Release);
        }
    }

    fn top_window_at(&self, x: usize, y: usize) -> Option<usize> {
        let mut top = None;
        for index in 0..self.windows.len() {
            let window = &self.windows[index];
            if window.draggable
                && window.contains(x, y)
                && top.is_none_or(|current: usize| self.windows[current].z_order < window.z_order)
            {
                top = Some(index);
            }
        }
        top
    }

    fn taskbar_tab_at(&self, x: usize, y: usize) -> Option<usize> {
        let taskbar_y = self.screen_height.saturating_sub(TASKBAR_HEIGHT);
        if y < taskbar_y || y >= self.screen_height {
            return None;
        }
        let mut slot = 0;
        for (index, window) in self.windows.iter().enumerate() {
            if !window.draggable || window.view_state != WindowViewState::Minimized {
                continue;
            }
            let left = TASKBAR_TAB_X + slot * (TASKBAR_TAB_WIDTH + TASKBAR_TAB_SPACING);
            if x >= left && x < left + TASKBAR_TAB_WIDTH {
                return Some(index);
            }
            slot += 1;
        }
        None
    }

    fn control_at(&self, index: usize, x: usize, y: usize) -> Option<WindowControl> {
        let window = &self.windows[index];
        if !window.contains_title_bar(x, y) {
            return None;
        }
        let top = window.y.saturating_add(7);
        let close_x = window
            .x
            .saturating_add(window.width)
            .saturating_sub(CONTROL_SIZE + 6);
        let maximize_x = close_x.saturating_sub(CONTROL_SIZE + CONTROL_GAP);
        let minimize_x = maximize_x.saturating_sub(CONTROL_SIZE + CONTROL_GAP);
        if y < top || y >= top + CONTROL_SIZE {
            return None;
        }
        if x >= close_x && x < close_x + CONTROL_SIZE {
            Some(WindowControl::Close)
        } else if x >= maximize_x && x < maximize_x + CONTROL_SIZE {
            Some(WindowControl::Maximize)
        } else if x >= minimize_x && x < minimize_x + CONTROL_SIZE {
            Some(WindowControl::Minimize)
        } else {
            None
        }
    }

    fn apply_window_control(&mut self, index: usize, control: WindowControl) {
        match control {
            WindowControl::Close => {
                self.windows[index] = Window::empty();
                self.choose_fallback_focus();
            }
            WindowControl::Minimize => {
                self.windows[index].view_state = WindowViewState::Minimized;
                self.windows[index].visible = false;
                self.choose_fallback_focus();
            }
            WindowControl::Maximize => self.toggle_maximize(index),
        }
    }

    fn toggle_maximize(&mut self, index: usize) {
        let window = &mut self.windows[index];
        if window.view_state == WindowViewState::Maximized {
            if let Some(geometry) = window.saved_geometry.take() {
                window.x = geometry.x;
                window.y = geometry.y;
                window.width = geometry.width;
                window.height = geometry.height;
            }
            window.view_state = WindowViewState::Normal;
        } else {
            window.saved_geometry = Some(SavedGeometry {
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
            });
            window.x = 0;
            window.y = 0;
            window.width = self.screen_width;
            window.height = self.screen_height.saturating_sub(TASKBAR_HEIGHT);
            window.view_state = WindowViewState::Maximized;
        }
        self.raise_window(index);
    }

    fn restore_window(&mut self, index: usize) {
        let window = &mut self.windows[index];
        if window.view_state == WindowViewState::Minimized {
            window.view_state = if window.saved_geometry.is_some() {
                WindowViewState::Maximized
            } else {
                WindowViewState::Normal
            };
            window.visible = true;
        }
        self.raise_window(index);
        self.set_focus(index);
    }

    /// Composes one window's shadow, surface, controls, and content into the back buffer.
    ///
    /// `index` identifies focus for active title-bar styling. Rendering follows
    /// z-order in `render`, so shadows are painted before their owning surfaces.
    ///
    /// NOTE: Brand Styling: tune the shared corner radius, shadow depth, and
    /// active/inactive title colors here for consistent window chrome.
    fn render_window(&self, index: usize, window: &Window) -> Result<(), &'static str> {
        if window.kind == WindowKind::Desktop {
            graphics::draw_string(24, 24, "Horizon OS", graphics::THEME.text_secondary)?;
            return graphics::put_pixel(
                window.width.saturating_sub(16),
                16,
                graphics::THEME.border_subtle,
            );
        }

        if window.draggable || window.kind == WindowKind::StartMenu {
            graphics::draw_drop_shadow(window.x, window.y, window.width, window.height, 3, 10)?;
        }
        graphics::draw_rounded_rectangle(
            window.x,
            window.y,
            window.width,
            window.height,
            WINDOW_CORNER_RADIUS,
            graphics::THEME.border_subtle,
        )?;
        graphics::draw_rounded_rectangle(
            window.x.saturating_add(2),
            window.y.saturating_add(2),
            window.width.saturating_sub(4),
            window.height.saturating_sub(4),
            WINDOW_CORNER_RADIUS.saturating_sub(2),
            window.background_color,
        )?;

        if window.buffer_width > 0 && window.buffer_height > 0 {
            let inset = if window.title_bar_height > 0 {
                TITLE_BAR_HEIGHT
            } else {
                4
            };
            graphics::draw_buffer(
                window.x.saturating_add(4),
                window.y.saturating_add(inset),
                window.buffer_width,
                window.buffer_height,
                window.width.saturating_sub(8).min(window.buffer_width),
                window
                    .height
                    .saturating_sub(inset + 4)
                    .min(window.buffer_height),
                window.pixel_buffer,
            )?;
        }

        if window.title_bar_height > 0 {
            let focused = FOCUSED_WINDOW_ID.load(Ordering::Relaxed) == index;
            let title_color = if focused {
                graphics::THEME.window_bar_active
            } else {
                graphics::THEME.window_bar_inactive
            };
            graphics::draw_rounded_rectangle(
                window.x.saturating_add(2),
                window.y.saturating_add(2),
                window.width.saturating_sub(4),
                window.title_bar_height.saturating_sub(2),
                WINDOW_CORNER_RADIUS.saturating_sub(2),
                title_color,
            )?;
            graphics::draw_string(
                window.x.saturating_add(14),
                window.y.saturating_add(12),
                window.title,
                graphics::THEME.text_primary,
            )?;
            self.render_window_controls(window)?;
        }

        match window.kind {
            WindowKind::Taskbar => {
                graphics::draw_rectangle(
                    window.x,
                    window.y,
                    window.width,
                    2,
                    graphics::THEME.border_subtle,
                )?;
                graphics::draw_string(
                    window.x.saturating_add(68),
                    window.y.saturating_add(24),
                    "Horizon",
                    graphics::THEME.text_primary,
                )?;
                self.render_taskbar_tabs(window)?;
                self.render_clock(window)?;
            }
            WindowKind::StartButton => {
                // Show ice blue only as the active launcher indicator.
                if self.windows[START_MENU_INDEX].visible {
                    graphics::draw_rounded_rectangle(
                        window.x.saturating_add(2),
                        window.y.saturating_add(window.height.saturating_sub(4)),
                        window.width.saturating_sub(4),
                        2,
                        1,
                        graphics::THEME.accent_highlight,
                    )?;
                }
                for row in 0..2 {
                    for column in 0..2 {
                        graphics::draw_rectangle(
                            window.x + 8 + column * 9,
                            window.y + 8 + row * 9,
                            7,
                            7,
                            graphics::THEME.text_primary,
                        )?;
                    }
                }
            }
            WindowKind::Application => self.render_application_content(window)?,
            WindowKind::StartMenu => self.render_start_menu(window)?,
            WindowKind::FileBrowser => self.render_file_browser(window)?,
            WindowKind::Computer => self.render_computer(window)?,
            WindowKind::SystemControl => self.render_system_control(window)?,
            WindowKind::TextViewer => self.render_text_viewer(window)?,
            WindowKind::Win32Demo => self.render_win32_demo_content(window)?,
            WindowKind::Empty | WindowKind::Desktop => {}
        }
        Ok(())
    }

    /// Draws neutral minimize/maximize controls and the themed close control.
    ///
    /// NOTE: Brand Styling: use palette tokens for every control surface and
    /// glyph so chrome stays consistent when the global theme changes.
    fn render_window_controls(&self, window: &Window) -> Result<(), &'static str> {
        let y = window.y.saturating_add(7);
        let close_x = window
            .x
            .saturating_add(window.width)
            .saturating_sub(CONTROL_SIZE + 6);
        let maximize_x = close_x.saturating_sub(CONTROL_SIZE + CONTROL_GAP);
        let minimize_x = maximize_x.saturating_sub(CONTROL_SIZE + CONTROL_GAP);

        graphics::draw_rounded_rectangle(
            minimize_x,
            y,
            CONTROL_SIZE,
            CONTROL_SIZE,
            4,
            graphics::THEME.control_neutral,
        )?;
        graphics::draw_rounded_rectangle(
            maximize_x,
            y,
            CONTROL_SIZE,
            CONTROL_SIZE,
            4,
            graphics::THEME.control_neutral,
        )?;
        graphics::draw_rounded_rectangle(
            close_x,
            y,
            CONTROL_SIZE,
            CONTROL_SIZE,
            4,
            graphics::THEME.destructive_muted,
        )?;
        graphics::draw_rectangle(minimize_x + 4, y + 9, 6, 2, graphics::THEME.text_primary)?;
        for edge in 0..6 {
            graphics::put_pixel(maximize_x + 4 + edge, y + 4, graphics::THEME.text_primary)?;
            graphics::put_pixel(maximize_x + 4 + edge, y + 10, graphics::THEME.text_primary)?;
            graphics::put_pixel(maximize_x + 4, y + 4 + edge, graphics::THEME.text_primary)?;
            graphics::put_pixel(maximize_x + 9, y + 4 + edge, graphics::THEME.text_primary)?;
            graphics::put_pixel(
                close_x + 4 + edge,
                y + 4 + edge,
                graphics::THEME.text_primary,
            )?;
            graphics::put_pixel(
                close_x + 9 - edge,
                y + 4 + edge,
                graphics::THEME.text_primary,
            )?;
        }
        Ok(())
    }

    /// Draws minimized-window tabs with subtle idle wireframe borders.
    ///
    /// NOTE: Brand Styling: keep tabs neutral until a taskbar selection is
    /// active; reserve Ice Blue for explicit focus feedback.
    fn render_taskbar_tabs(&self, taskbar: &Window) -> Result<(), &'static str> {
        let mut slot = 0;
        for window in &self.windows {
            if !window.draggable || window.view_state != WindowViewState::Minimized {
                continue;
            }
            let x = TASKBAR_TAB_X + slot * (TASKBAR_TAB_WIDTH + TASKBAR_TAB_SPACING);
            graphics::draw_rounded_rectangle(
                x,
                taskbar.y.saturating_add(10),
                TASKBAR_TAB_WIDTH,
                36,
                7,
                graphics::THEME.border_subtle,
            )?;
            graphics::draw_rounded_rectangle(
                x.saturating_add(1),
                taskbar.y.saturating_add(11),
                TASKBAR_TAB_WIDTH.saturating_sub(2),
                34,
                6,
                graphics::THEME.control_neutral,
            )?;
            graphics::draw_string(
                x.saturating_add(8),
                taskbar.y.saturating_add(24),
                match window.kind {
                    WindowKind::FileBrowser => "Explorer",
                    WindowKind::Computer => "This PC",
                    WindowKind::SystemControl => "System",
                    WindowKind::TextViewer => "Reader",
                    WindowKind::Win32Demo => "GDI Demo",
                    _ => "Horizon",
                },
                graphics::THEME.text_primary,
            )?;
            slot += 1;
        }
        Ok(())
    }

    fn render_clock(&self, taskbar: &Window) -> Result<(), &'static str> {
        let x = taskbar.x.saturating_add(taskbar.width.saturating_sub(172));
        let y = taskbar.y.saturating_add(24);
        if let Some(datetime) = self.clock {
            let mut text = [0u8; 19];
            write_decimal4(&mut text[0..4], datetime.year);
            text[4] = b'-';
            write_decimal2(&mut text[5..7], datetime.month);
            text[7] = b'-';
            write_decimal2(&mut text[8..10], datetime.day);
            text[10] = b' ';
            write_decimal2(&mut text[11..13], datetime.hour);
            text[13] = b':';
            write_decimal2(&mut text[14..16], datetime.minute);
            text[16] = b':';
            write_decimal2(&mut text[17..19], datetime.second);
            let text = core::str::from_utf8(&text).map_err(|_| "failed to format taskbar clock")?;
            graphics::draw_string(x, y, text, graphics::THEME.text_primary)
        } else {
            let message = if self.clock_error.is_some() {
                "Clock unavailable"
            } else {
                "Clock starting"
            };
            graphics::draw_string(x, y, message, graphics::THEME.text_primary)
        }
    }

    fn render_application_content(&self, window: &Window) -> Result<(), &'static str> {
        let x = window.x.saturating_add(16);
        let y = window.y.saturating_add(TITLE_BAR_HEIGHT + 16);
        graphics::draw_string(x, y, "Welcome to Horizon OS", graphics::THEME.text_primary)?;
        graphics::draw_string(
            x,
            y.saturating_add(18),
            "Open Start to launch a utility.",
            graphics::THEME.text_secondary,
        )?;
        draw_decimal_line(
            x,
            y.saturating_add(36),
            "Calc:",
            self.background_counter as usize,
            graphics::THEME.text_secondary,
        )?;
        self.render_input_field(window)?;
        self.render_save_button(window)?;
        if let Some(status) = self.save_status {
            let color = if status.starts_with("Saved") {
                graphics::THEME.status_success
            } else {
                graphics::THEME.status_error
            };
            graphics::draw_string(x, y.saturating_add(54), status, color)?;
        }

        if let Some(error) = self.pe_error {
            graphics::draw_string(
                x,
                y.saturating_add(54),
                "PE32 load/parse failed:",
                graphics::THEME.status_error,
            )?;
            graphics::draw_string(x, y.saturating_add(72), error, graphics::THEME.status_error)?;
            self.render_window_text(window, y.saturating_add(184))?;
            return Ok(());
        }

        if let Some(report) = self.pe_report {
            graphics::draw_string(
                x,
                y.saturating_add(48),
                "PE32 image validated (not executed)",
                graphics::THEME.text_secondary,
            )?;
            self.draw_hex_value(x, y.saturating_add(68), "Image base:", report.image_base)?;
            self.draw_hex_value(
                x,
                y.saturating_add(86),
                "Entry RVA:",
                report.entry_point_rva,
            )?;
            self.draw_hex_value(x, y.saturating_add(104), "Image size:", report.image_size)?;
            draw_decimal_line(
                x,
                y.saturating_add(122),
                "Sections:",
                report.section_count,
                graphics::THEME.text_primary,
            )?;
            let name_length = report
                .first_section_name
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(report.first_section_name.len());
            if name_length > 0 {
                if let Ok(name) = core::str::from_utf8(&report.first_section_name[..name_length]) {
                    graphics::draw_string(
                        x,
                        y.saturating_add(140),
                        "First section:",
                        graphics::THEME.text_primary,
                    )?;
                    graphics::draw_string(
                        x + 112,
                        y.saturating_add(140),
                        name,
                        graphics::THEME.text_primary,
                    )?;
                    let characteristics = report.first_section_characteristics;
                    let readable = characteristics & 0x4000_0000 != 0;
                    let writable = characteristics & 0x8000_0000 != 0;
                    let executable = characteristics & 0x2000_0000 != 0;
                    let access = match (readable, writable, executable) {
                        (true, false, true) => "Read / Execute",
                        (true, true, false) => "Read / Write",
                        (true, true, true) => "Read / Write / Execute",
                        (true, false, false) => "Read only",
                        _ => "Other",
                    };
                    graphics::draw_string(
                        x,
                        y.saturating_add(158),
                        "Access:",
                        graphics::THEME.text_primary,
                    )?;
                    graphics::draw_string(
                        x.saturating_add(56),
                        y.saturating_add(158),
                        access,
                        graphics::THEME.text_primary,
                    )?;
                }
            }
        }
        self.render_window_text(window, y.saturating_add(184))?;
        Ok(())
    }

    /// Renders the neutral, subtly outlined Save File control beside the input.
    ///
    /// NOTE: Brand Styling: keep the idle button in the neutral family and
    /// use Ice Blue only for an explicit pressed/focused state.
    fn render_save_button(&self, window: &Window) -> Result<(), &'static str> {
        let x = window
            .x
            .saturating_add(window.width)
            .saturating_sub(SAVE_BUTTON_MARGIN + SAVE_BUTTON_WIDTH);
        let y = window.y.saturating_add(window.input_field.y);
        graphics::draw_rounded_rectangle(
            x,
            y,
            SAVE_BUTTON_WIDTH,
            SAVE_BUTTON_HEIGHT,
            6,
            graphics::THEME.border_subtle,
        )?;
        graphics::draw_rounded_rectangle(
            x.saturating_add(1),
            y.saturating_add(1),
            SAVE_BUTTON_WIDTH.saturating_sub(2),
            SAVE_BUTTON_HEIGHT.saturating_sub(2),
            5,
            graphics::THEME.control_neutral,
        )?;
        graphics::draw_string(
            x.saturating_add(18),
            y.saturating_add(9),
            "Save File",
            graphics::THEME.text_primary,
        )
    }

    fn render_window_text(&self, window: &Window, top: usize) -> Result<(), &'static str> {
        let text = window.text_buffer.as_bytes();
        if text.is_empty() {
            return Ok(());
        }
        let maximum_columns = window.width.saturating_sub(32) / 8;
        if maximum_columns == 0 {
            return Ok(());
        }

        let mut line_number = 0;
        let mut line_start = 0;
        while line_start < text.len() {
            let remaining = &text[line_start..];
            let line_length = remaining
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(remaining.len())
                .min(maximum_columns);
            self.draw_text_line(window, top, line_number, &remaining[..line_length])?;
            line_number += 1;
            line_start += line_length;
            if text.get(line_start) == Some(&b'\n') {
                line_start += 1;
            }
        }
        Ok(())
    }

    fn draw_text_line(
        &self,
        window: &Window,
        top: usize,
        line_number: usize,
        line: &[u8],
    ) -> Result<(), &'static str> {
        let y = top.saturating_add(line_number.saturating_mul(12));
        if y.saturating_add(8) > window.y.saturating_add(window.height) {
            return Ok(());
        }
        let text = core::str::from_utf8(line).map_err(|_| "window text is not valid ASCII")?;
        graphics::draw_string(
            window.x.saturating_add(16),
            y,
            text,
            graphics::THEME.text_primary,
        )
    }

    fn draw_hex_value(
        &self,
        x: usize,
        y: usize,
        label: &str,
        value: u32,
    ) -> Result<(), &'static str> {
        graphics::draw_string(x, y, label, graphics::THEME.text_primary)?;
        let mut buffer = [0u8; 10];
        buffer[..2].copy_from_slice(b"0x");
        for (index, shift) in (0..8).rev().enumerate() {
            buffer[index + 2] = b"0123456789ABCDEF"[((value >> (shift * 4)) & 0xf) as usize];
        }
        let value = core::str::from_utf8(&buffer).map_err(|_| "failed to format PE metadata")?;
        graphics::draw_string(x.saturating_add(88), y, value, graphics::THEME.text_primary)
    }

    /// Draws launcher rows in the same order used by the pointer action router.
    ///
    /// NOTE: Action Routing: keep the System Utility row's position aligned
    /// with the corresponding `handle_mouse` match arm.
    fn render_start_menu(&self, window: &Window) -> Result<(), &'static str> {
        graphics::draw_string(
            window.x.saturating_add(18),
            window.y.saturating_add(14),
            "Horizon Menu",
            graphics::THEME.text_primary,
        )?;
        // NOTE: Action Routing: keep labels in the same order as the hit-test arms.
        for (index, title) in [
            "File Explorer",
            "System Control",
            "Run Sample PE",
            "Run Win32 Demo",
            "Launch External App",
            "System Utility",
        ]
        .iter()
        .enumerate()
        {
            let y = window
                .y
                .saturating_add(MENU_ITEM_TOP + index * MENU_ITEM_SPACING);
            graphics::draw_rounded_rectangle(
                window.x.saturating_add(12),
                y,
                window.width.saturating_sub(24),
                MENU_ITEM_HEIGHT,
                5,
                graphics::THEME.border_subtle,
            )?;
            graphics::draw_rounded_rectangle(
                window.x.saturating_add(13),
                y.saturating_add(1),
                window.width.saturating_sub(26),
                MENU_ITEM_HEIGHT.saturating_sub(2),
                4,
                graphics::THEME.control_neutral,
            )?;
            graphics::draw_string(
                window.x.saturating_add(22),
                y.saturating_add(14),
                title,
                graphics::THEME.text_primary,
            )?;
        }
        Ok(())
    }

    /// Renders a sample through the exported GDI-shaped kernel hooks.
    ///
    /// GDI `RECT` coordinates are signed, half-open, and top-left-origin; the
    /// hooks clip them to the framebuffer and convert them to native origin
    /// plus extent. This prototype targets the kernel framebuffer directly,
    /// rather than a guest device context or isolated window pixel buffer.
    fn render_win32_demo_content(&self, window: &Window) -> Result<(), &'static str> {
        let left = i32::try_from(window.x.saturating_add(18))
            .map_err(|_| "Win32 demo window X coordinate exceeds GDI range")?;
        let top = i32::try_from(window.y.saturating_add(TITLE_BAR_HEIGHT + 18))
            .map_err(|_| "Win32 demo window Y coordinate exceeds GDI range")?;
        let rect = crate::win32::gdi::Rect {
            left,
            top,
            right: left.saturating_add(176),
            bottom: top.saturating_add(52),
        };

        // SAFETY: `rect` is a live kernel-owned value for the duration of the
        // call, and the demo text below is a valid, readable static byte slice.
        let filled = unsafe {
            crate::win32::gdi::FillRect(
                core::ptr::null_mut(),
                &rect,
                crate::win32::gdi::DEMO_BLUE_BRUSH,
            )
        };
        if filled == 0 {
            return Err("Win32 demo FillRect hook failed");
        }

        let text_y = top.saturating_add(72);
        // SAFETY: the static ASCII string and its exact byte length are valid
        // for the duration of the hook call; coordinates use screen-space
        // framebuffer pixels after adding the containing window's origin.
        let text = b"TextOutA -> Horizon font blitter";
        let text_drawn = unsafe {
            crate::win32::gdi::TextOutA(
                core::ptr::null_mut(),
                left,
                text_y,
                text.as_ptr(),
                text.len() as i32,
            )
        };
        if text_drawn == 0 {
            return Err("Win32 demo TextOutA hook failed");
        }
        Ok(())
    }

    /// Renders only direct VFS children of the browser's configured directory.
    ///
    /// NOTE: Open-source Contribution Area: add pagination and per-column
    /// metadata when the directory index outgrows the visible row limit.
    fn render_file_browser(&self, window: &Window) -> Result<(), &'static str> {
        let x = window.x.saturating_add(16);
        let y = window.y.saturating_add(TITLE_BAR_HEIGHT + 16);
        graphics::draw_string(x, y, window.browser_path, graphics::THEME.text_secondary)?;
        let entry_count = browser_child_count(window.browser_path)?;
        for index in 0..entry_count.min(FILE_LIST_VISIBLE_ROWS) {
            let entry = browser_child_at(window.browser_path, index)?
                .ok_or("filesystem directory changed while rendering File Explorer")?;
            let name = browser_child_name(&entry, window.browser_path)?
                .ok_or("File Explorer row is no longer a direct child")?;
            let name = core::str::from_utf8(name)
                .map_err(|_| "filesystem entry name is not valid ASCII")?;
            let row_y = y.saturating_add(FILE_LIST_TOP + index * FILE_LIST_ROW_HEIGHT);
            graphics::draw_string(
                x.saturating_add(8),
                row_y,
                if entry.is_directory {
                    "[DIR]"
                } else {
                    "[FILE]"
                },
                if entry.is_directory {
                    graphics::THEME.text_secondary
                } else {
                    graphics::THEME.text_secondary
                },
            )?;
            graphics::draw_string(
                x.saturating_add(58),
                row_y,
                name,
                graphics::THEME.text_primary,
            )?;
        }
        if entry_count > FILE_LIST_VISIBLE_ROWS {
            graphics::draw_string(
                x.saturating_add(8),
                y.saturating_add(FILE_LIST_TOP + FILE_LIST_VISIBLE_ROWS * FILE_LIST_ROW_HEIGHT),
                "More items are available in this folder",
                graphics::THEME.text_secondary,
            )?;
        }
        Ok(())
    }

    /// Displays the currently available virtual volumes for the This PC view.
    ///
    /// `C:\` is backed by the initrd plus volatile overlay. `D:\` is shown as
    /// an unmounted placeholder so the UI does not imply that it is accessible.
    ///
    /// NOTE: Open-source Contribution Area: enumerate actual mounted volumes
    /// when storage mount/unmount events are available.
    fn render_computer(&self, window: &Window) -> Result<(), &'static str> {
        let x = window.x.saturating_add(24);
        let y = window.y.saturating_add(TITLE_BAR_HEIGHT + 24);
        for (row, (drive, description)) in
            [("C:\\", "System and user files"), ("D:\\", "Not mounted")]
                .iter()
                .enumerate()
        {
            let row_y = y.saturating_add(row * 58);
            graphics::draw_rounded_rectangle(
                x,
                row_y,
                window.width.saturating_sub(48),
                48,
                6,
                graphics::THEME.border_subtle,
            )?;
            graphics::draw_rounded_rectangle(
                x.saturating_add(1),
                row_y.saturating_add(1),
                window.width.saturating_sub(50),
                46,
                5,
                graphics::THEME.control_neutral,
            )?;
            graphics::draw_string(
                x.saturating_add(12),
                row_y.saturating_add(10),
                drive,
                graphics::THEME.text_primary,
            )?;
            graphics::draw_string(
                x.saturating_add(84),
                row_y.saturating_add(10),
                description,
                graphics::THEME.text_secondary,
            )?;
        }
        Ok(())
    }

    /// Renders the rounded editor field, scrolled text, and tick-driven caret.
    ///
    /// `window` supplies the field geometry and text; its input focus controls
    /// whether the caret is eligible to blink.
    ///
    /// NOTE: Brand Styling: the neutral border becomes Ice Blue only while this
    /// field is focused.
    ///
    /// NOTE: Contribution Area: add selection and horizontal-scroll state when
    /// keyboard navigation is introduced.
    fn render_input_field(&self, window: &Window) -> Result<(), &'static str> {
        let field = &window.input_field;
        if field.width == 0 {
            return Ok(());
        }
        let x = window.x.saturating_add(field.x);
        let y = window.y.saturating_add(field.y);
        graphics::draw_rounded_rectangle(
            x,
            y,
            field.width,
            field.height,
            6,
            if field.focused {
                graphics::THEME.accent_highlight
            } else {
                graphics::THEME.border_subtle
            },
        )?;
        graphics::draw_rounded_rectangle(
            x.saturating_add(1),
            y.saturating_add(1),
            field.width.saturating_sub(2),
            field.height.saturating_sub(2),
            5,
            graphics::THEME.background_canvas,
        )?;

        // NOTE: Open-source Contribution Area: add horizontal scrolling when
        // the editor grows beyond the fixed single-line input widget.
        let text = if field.text.is_empty() {
            "(type here)"
        } else {
            let maximum_columns = field.width.saturating_sub(16) / 8;
            let visible_start = field.text.len().saturating_sub(maximum_columns);
            &field.text[visible_start..]
        };
        let text_x = x.saturating_add(8);
        let text_y = y.saturating_add(9);
        graphics::draw_string(
            text_x,
            text_y,
            text,
            if field.text.is_empty() {
                graphics::THEME.text_secondary
            } else {
                graphics::THEME.text_primary
            },
        )?;
        if field.focused && (crate::clock::ticks() / CARET_BLINK_TICKS).is_multiple_of(2) {
            let visible_columns = field.text.len().min(field.width.saturating_sub(16) / 8);
            let caret_x = text_x.saturating_add(visible_columns.saturating_mul(8));
            if caret_x < x.saturating_add(field.width).saturating_sub(4) {
                graphics::draw_rectangle(
                    caret_x,
                    y.saturating_add(6),
                    2,
                    14,
                    graphics::THEME.accent_highlight,
                )?;
            }
        }
        Ok(())
    }

    fn render_text_viewer(&self, window: &Window) -> Result<(), &'static str> {
        let x = window.x.saturating_add(16);
        let top = window.y.saturating_add(TITLE_BAR_HEIGHT + 16);
        graphics::draw_string(x, top, &window.viewer_path, graphics::THEME.text_secondary)?;

        let maximum_columns = window.width.saturating_sub(40) / 8;
        if maximum_columns == 0 {
            return Ok(());
        }
        let contents = window.viewer_contents.as_bytes();
        let mut line_start = 0usize;
        let mut line_number = 0usize;
        let maximum_rows = window.height.saturating_sub(TITLE_BAR_HEIGHT + 48) / 12;
        while line_start < contents.len() && line_number < maximum_rows {
            let remaining = &contents[line_start..];
            let line_length = remaining
                .iter()
                .position(|byte| *byte == b'\n' || *byte == b'\r')
                .unwrap_or(remaining.len())
                .min(maximum_columns);
            draw_ascii_line(
                x,
                top.saturating_add(20 + line_number * 12),
                &remaining[..line_length],
            )?;
            line_number += 1;
            line_start += line_length;
            if matches!(contents.get(line_start), Some(b'\n' | b'\r')) {
                let separator = contents[line_start];
                line_start += 1;
                if separator == b'\r' && contents.get(line_start) == Some(&b'\n') {
                    line_start += 1;
                }
            }
        }
        Ok(())
    }

    fn render_system_control(&self, window: &Window) -> Result<(), &'static str> {
        let x = window.x.saturating_add(16);
        let y = window.y.saturating_add(TITLE_BAR_HEIGHT + 16);
        graphics::draw_string(x, y, "System Control", graphics::THEME.text_secondary)?;
        graphics::draw_string(
            x,
            y.saturating_add(24),
            "Kernel: Horizon OS",
            graphics::THEME.text_primary,
        )?;
        graphics::draw_string(
            x,
            y.saturating_add(44),
            "Input: PS/2 mouse",
            graphics::THEME.text_primary,
        )?;
        graphics::draw_string(
            x,
            y.saturating_add(64),
            "Display: linear framebuffer",
            graphics::THEME.text_primary,
        )?;
        self.render_window_text(window, y.saturating_add(92))
    }

    fn start_menu_item_at(&self, x: usize, y: usize) -> Option<usize> {
        let menu = &self.windows[START_MENU_INDEX];
        if x < menu.x.saturating_add(12)
            || x >= menu.x.saturating_add(menu.width).saturating_sub(12)
        {
            return None;
        }
        let relative_y = y.checked_sub(menu.y.saturating_add(MENU_ITEM_TOP))?;
        let item = relative_y / MENU_ITEM_SPACING;
        if item >= MENU_ITEM_COUNT || relative_y % MENU_ITEM_SPACING >= MENU_ITEM_HEIGHT {
            return None;
        }
        Some(item)
    }

    /// Tests screen coordinates against the main-window Save File button.
    ///
    /// NOTE: Open-source Contribution Area: centralize control geometry as
    /// additional editor actions and themes are introduced.
    fn save_button_contains(&self, window: &Window, x: usize, y: usize) -> bool {
        let left = window
            .x
            .saturating_add(window.width)
            .saturating_sub(SAVE_BUTTON_MARGIN + SAVE_BUTTON_WIDTH);
        let top = window.y.saturating_add(window.input_field.y);
        x >= left
            && x < left.saturating_add(SAVE_BUTTON_WIDTH)
            && y >= top
            && y < top.saturating_add(SAVE_BUTTON_HEIGHT)
    }

    fn toggle_start_menu(&mut self) {
        let menu = &mut self.windows[START_MENU_INDEX];
        menu.visible = !menu.visible;
        if menu.visible {
            self.raise_window(START_MENU_INDEX);
        }
    }

    /// Opens or restores the root C-drive File Explorer window.
    ///
    /// NOTE: Open-source Contribution Area: route this through a shell
    /// navigation service when the desktop gains address-bar navigation.
    fn spawn_file_browser(&mut self) -> Result<(), &'static str> {
        self.spawn_file_browser_at(r"C:\")
    }

    /// Opens or restores a File Explorer window rooted at an indexed directory.
    ///
    /// `path` must resolve to an existing VFS directory. The static path keeps
    /// built-in shortcut destinations allocation-free and bounded.
    ///
    /// NOTE: Open-source Contribution Area: accept canonicalized VFS paths
    /// from an address bar only after path navigation has UI validation.
    fn spawn_file_browser_at(&mut self, path: &'static str) -> Result<(), &'static str> {
        let directory = crate::filesystem::resolve_windows_path(path)?;
        if !directory.is_directory {
            return Err("File Explorer destination is not a directory");
        }
        if let Some(index) = self.windows.iter().position(|window| {
            window.kind == WindowKind::FileBrowser && window.browser_path.eq_ignore_ascii_case(path)
        }) {
            self.windows[index].visible = true;
            self.windows[index].view_state = WindowViewState::Normal;
            self.raise_window(index);
            self.set_focus(index);
            return Ok(());
        }

        let width = FILE_BROWSER_WIDTH.min(self.screen_width.saturating_sub(24));
        let height = FILE_BROWSER_HEIGHT.min(self.screen_height.saturating_sub(TASKBAR_HEIGHT));
        let x = ((self.screen_width - width) / 2 + 28).min(self.screen_width - width);
        let y = ((self.screen_height.saturating_sub(TASKBAR_HEIGHT + height)) / 2 + 18)
            .min(self.screen_height - height);
        let z_order = self.next_z_order();
        let mut browser = Window::new(
            WindowKind::FileBrowser,
            "File Explorer",
            x,
            y,
            width,
            height,
            z_order,
            true,
            true,
            TITLE_BAR_HEIGHT,
            graphics::THEME.surface_window,
            &FILE_BROWSER_CONTENT,
            FILE_BROWSER_CONTENT_WIDTH,
            FILE_BROWSER_CONTENT_HEIGHT,
        );
        browser.browser_path = path;
        let index = self.insert_window(browser)?;
        self.raise_window(index);
        self.set_focus(index);
        Ok(())
    }

    /// Opens or restores a window listing the virtual drive roots.
    ///
    /// NOTE: Open-source Contribution Area: enumerate mounted volumes from
    /// filesystem metadata rather than hard-coded presentation rows.
    fn spawn_computer(&mut self) -> Result<(), &'static str> {
        if let Some(index) = self.find_window(WindowKind::Computer) {
            self.windows[index].visible = true;
            self.windows[index].view_state = WindowViewState::Normal;
            self.raise_window(index);
            self.set_focus(index);
            return Ok(());
        }

        let width = FILE_BROWSER_WIDTH.min(self.screen_width.saturating_sub(24));
        let height = FILE_BROWSER_HEIGHT.min(self.screen_height.saturating_sub(TASKBAR_HEIGHT));
        let x = self.screen_width.saturating_sub(width) / 2;
        let y = self.screen_height.saturating_sub(TASKBAR_HEIGHT + height) / 2;
        let index = self.insert_window(Window::new(
            WindowKind::Computer,
            "This PC",
            x,
            y,
            width,
            height,
            self.next_z_order(),
            true,
            true,
            TITLE_BAR_HEIGHT,
            graphics::THEME.surface_window,
            &FILE_BROWSER_CONTENT,
            FILE_BROWSER_CONTENT_WIDTH,
            FILE_BROWSER_CONTENT_HEIGHT,
        ))?;
        self.raise_window(index);
        self.set_focus(index);
        Ok(())
    }

    fn spawn_system_control(&mut self) -> Result<(), &'static str> {
        if let Some(index) = self.find_window(WindowKind::SystemControl) {
            self.windows[index].visible = true;
            self.windows[index].view_state = WindowViewState::Normal;
            self.raise_window(index);
            self.set_focus(index);
            return Ok(());
        }

        let width = SYSTEM_CONTROL_WIDTH.min(self.screen_width.saturating_sub(24));
        let height = SYSTEM_CONTROL_HEIGHT.min(self.screen_height.saturating_sub(TASKBAR_HEIGHT));
        let x = ((self.screen_width - width) / 2 + 42).min(self.screen_width - width);
        let y = ((self.screen_height.saturating_sub(TASKBAR_HEIGHT + height)) / 2 + 32)
            .min(self.screen_height - height);
        let z_order = self.next_z_order();
        let index = self.insert_window(Window::new(
            WindowKind::SystemControl,
            "System Control",
            x,
            y,
            width,
            height,
            z_order,
            true,
            true,
            TITLE_BAR_HEIGHT,
            graphics::THEME.surface_window,
            &SYSTEM_CONTROL_CONTENT,
            SYSTEM_CONTROL_CONTENT_WIDTH,
            SYSTEM_CONTROL_CONTENT_HEIGHT,
        ))?;
        self.raise_window(index);
        self.set_focus(index);
        Ok(())
    }

    /// Opens or restores the editor and calculator display in its reserved slot.
    ///
    /// A first launch creates the window only after the user selects its Start
    /// Menu action and reserves heap-backed text capacity before publishing it.
    ///
    /// NOTE: Action Routing: allocate this window only after a Start Menu action,
    /// not while constructing the boot desktop.
    ///
    /// NOTE: Open-source Contribution Area: tune the initial input reservation
    /// after measuring realistic scratchpad usage and kernel heap pressure.
    fn spawn_scratchpad(&mut self) -> Result<(), &'static str> {
        if self.windows[MAIN_WINDOW_INDEX].kind == WindowKind::Application {
            self.windows[MAIN_WINDOW_INDEX].visible = true;
            self.windows[MAIN_WINDOW_INDEX].view_state = WindowViewState::Normal;
            self.raise_window(MAIN_WINDOW_INDEX);
            self.set_focus(MAIN_WINDOW_INDEX);
            return Ok(());
        }

        let width = APPLICATION_WIDTH.min(self.screen_width.saturating_sub(24));
        let taskbar_top = self.screen_height.saturating_sub(TASKBAR_HEIGHT);
        let height = APPLICATION_HEIGHT.min(taskbar_top);
        let mut window = Window::new(
            WindowKind::Application,
            "System Utility",
            self.screen_width.saturating_sub(width) / 2,
            taskbar_top.saturating_sub(height) / 2,
            width,
            height,
            self.next_z_order(),
            true,
            true,
            TITLE_BAR_HEIGHT,
            graphics::THEME.surface_window,
            &APPLICATION_CONTENT,
            APPLICATION_CONTENT_WIDTH,
            APPLICATION_CONTENT_HEIGHT,
        );
        window.input_field.width =
            width.saturating_sub(32 + SAVE_BUTTON_WIDTH + SAVE_BUTTON_MARGIN);
        window.input_field.y = height.saturating_sub(38);
        // NOTE: Action Routing: reserve editable storage before publishing the
        // new window so allocation failure leaves the desktop unchanged.
        window
            .input_field
            .text
            .try_reserve(128)
            .map_err(|_| "not enough heap memory to create the scratchpad input field")?;

        let index = self.insert_window(window)?;
        self.raise_window(index);
        self.set_focus(index);
        Ok(())
    }

    /// Opens or restores the standard decorated Win32 GDI demonstration window.
    ///
    /// NOTE: This is kept separate from scratchpad creation so only explicit
    /// launcher actions populate application slots.
    fn spawn_win32_demo(&mut self) -> Result<(), &'static str> {
        if let Some(index) = self.find_window(WindowKind::Win32Demo) {
            self.windows[index].visible = true;
            self.windows[index].view_state = WindowViewState::Normal;
            self.raise_window(index);
            self.set_focus(index);
            return Ok(());
        }

        let width = WIN32_DEMO_WIDTH.min(self.screen_width.saturating_sub(24));
        let height = WIN32_DEMO_HEIGHT.min(self.screen_height.saturating_sub(TASKBAR_HEIGHT));
        let x = self.screen_width.saturating_sub(width) / 2;
        let y = self.screen_height.saturating_sub(TASKBAR_HEIGHT + height) / 2;
        let z_order = self.next_z_order();
        let index = self.insert_window(Window::new(
            WindowKind::Win32Demo,
            "Win32 GDI Demo",
            x,
            y,
            width,
            height,
            z_order,
            true,
            true,
            TITLE_BAR_HEIGHT,
            graphics::THEME.surface_window,
            &[],
            0,
            0,
        ))?;
        self.raise_window(index);
        self.set_focus(index);
        Ok(())
    }

    fn raise_window(&mut self, index: usize) {
        self.windows[index].z_order = self.next_z_order();
    }

    fn next_z_order(&self) -> usize {
        self.windows
            .iter()
            .map(|window| window.z_order)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    fn find_window(&self, kind: WindowKind) -> Option<usize> {
        self.windows.iter().position(|window| window.kind == kind)
    }

    fn insert_window(&mut self, window: Window) -> Result<usize, &'static str> {
        // Keep the application index reserved for System Utility;
        // other launchers must not make its editor and save paths alias them.
        if window.kind == WindowKind::Application {
            if self.windows[MAIN_WINDOW_INDEX].kind != WindowKind::Empty {
                return Err("scratchpad window slot is already occupied");
            }
            self.windows[MAIN_WINDOW_INDEX] = window;
            return Ok(MAIN_WINDOW_INDEX);
        }

        // Closed/minimized slots are reused before the vector grows while
        // preserving fixed desktop, taskbar, launcher, and menu indices.
        if let Some(index) = self
            .windows
            .iter()
            .enumerate()
            .find_map(|(index, candidate)| {
                (index != MAIN_WINDOW_INDEX && candidate.kind == WindowKind::Empty).then_some(index)
            })
        {
            self.windows[index] = window;
            Ok(index)
        } else {
            self.windows
                .try_reserve(1)
                .map_err(|_| "not enough heap memory to create a window")?;
            self.windows.push(window);
            Ok(self.windows.len() - 1)
        }
    }

    fn move_window(&mut self, drag: DragState, mouse_x: usize, mouse_y: usize) {
        let window = &mut self.windows[drag.window_index];
        let max_x = self.screen_width.saturating_sub(window.width);
        let max_y = self.screen_height.saturating_sub(window.height);
        window.x = mouse_x.saturating_sub(drag.offset_x).min(max_x);
        window.y = mouse_y.saturating_sub(drag.offset_y).min(max_y);
    }
}

/// Returns the indexed direct child at `child_index` within a VFS directory.
///
/// The VFS is queried by index and filtered without building a temporary
/// allocation, so read-only archive entries and writable overlay entries use
/// the same path.
///
/// NOTE: Open-source Contribution Area: add a stable directory snapshot before
/// introducing pagination while concurrent file writes are supported.
fn browser_child_at(
    directory: &str,
    child_index: usize,
) -> Result<Option<EntryInfo>, &'static str> {
    let entry_count = crate::filesystem::entry_count()?;
    let mut visible_index = 0;
    for index in 0..entry_count {
        let Some(entry) = crate::filesystem::entry_at(index)? else {
            return Err("filesystem index changed while reading directory");
        };
        if browser_child_name(&entry, directory)?.is_some() {
            if visible_index == child_index {
                return Ok(Some(entry));
            }
            visible_index += 1;
        }
    }
    Ok(None)
}

/// Counts direct VFS children of one configured File Explorer path.
///
/// NOTE: Open-source Contribution Area: derive this from a directory iterator
/// when the VFS exposes mount-aware listing operations.
fn browser_child_count(directory: &str) -> Result<usize, &'static str> {
    let entry_count = crate::filesystem::entry_count()?;
    let mut visible_count = 0;
    for index in 0..entry_count {
        let Some(entry) = crate::filesystem::entry_at(index)? else {
            return Err("filesystem index changed while counting directory");
        };
        if browser_child_name(&entry, directory)?.is_some() {
            visible_count += 1;
        }
    }
    Ok(visible_count)
}

/// Validates an entry as a direct child and returns its display basename.
///
/// Entries below a subdirectory are excluded from its parent listing; paths
/// are compared case-insensitively to match VFS lookup semantics.
///
/// NOTE: Open-source Contribution Area: keep display-name decoding separate
/// from canonical path comparison if Unicode paths are added.
fn browser_child_name<'a>(
    entry: &'a EntryInfo,
    directory: &str,
) -> Result<Option<&'a [u8]>, &'static str> {
    if entry.path_length > entry.path.len() {
        return Err("filesystem entry path length is invalid");
    }
    let entry_path = &entry.path[..entry.path_length];
    let directory = directory.as_bytes();
    let remainder = if directory.last() == Some(&b'\\') {
        if entry_path.len() <= directory.len()
            || !entry_path[..directory.len()].eq_ignore_ascii_case(directory)
        {
            return Ok(None);
        }
        &entry_path[directory.len()..]
    } else {
        if entry_path.len() <= directory.len()
            || !entry_path[..directory.len()].eq_ignore_ascii_case(directory)
            || entry_path[directory.len()] != b'\\'
        {
            return Ok(None);
        }
        &entry_path[directory.len() + 1..]
    };
    if remainder.is_empty() || remainder.contains(&b'\\') {
        return Ok(None);
    }
    Ok(Some(remainder))
}

/// Writes a decimal number into the supplied two-byte buffer.
///
/// NOTE: Open-source Contribution Area: share number formatting with other
/// fixed-buffer UI renderers if a common no-allocation formatter is added.
fn write_decimal2(target: &mut [u8], value: u8) {
    target[0] = b'0' + value / 10;
    target[1] = b'0' + value % 10;
}

fn write_decimal4(target: &mut [u8], value: u16) {
    target[0] = b'0' + ((value / 1000) % 10) as u8;
    target[1] = b'0' + ((value / 100) % 10) as u8;
    target[2] = b'0' + ((value / 10) % 10) as u8;
    target[3] = b'0' + (value % 10) as u8;
}

fn draw_decimal_line(
    x: usize,
    y: usize,
    label: &str,
    value: usize,
    color: u32,
) -> Result<(), &'static str> {
    graphics::draw_string(x, y, label, color)?;
    let mut buffer = [0u8; 20];
    let mut offset = buffer.len();
    let mut value = value;
    loop {
        offset -= 1;
        buffer[offset] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let text =
        core::str::from_utf8(&buffer[offset..]).map_err(|_| "failed to format PE section count")?;
    graphics::draw_string(x.saturating_add(72), y, text, color)
}

fn draw_ascii_line(x: usize, y: usize, line: &[u8]) -> Result<(), &'static str> {
    let text = core::str::from_utf8(line).map_err(|_| "text viewer line is not valid ASCII")?;
    graphics::draw_string(x, y, text, graphics::THEME.text_primary)
}
