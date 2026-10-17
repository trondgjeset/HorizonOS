use crate::{clock::DateTime, filesystem::EntryInfo, graphics, mouse::MouseState, pe::PeSummary};
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};

const START_BUTTON_INDEX: usize = 2;
const MAIN_WINDOW_INDEX: usize = 3;
const START_MENU_INDEX: usize = 4;

const TITLE_BAR_HEIGHT: usize = 32;
const TASKBAR_HEIGHT: usize = 56;
const APPLICATION_WIDTH: usize = 460;
const APPLICATION_HEIGHT: usize = 300;
const APPLICATION_CONTENT_WIDTH: usize = APPLICATION_WIDTH - 8;
const APPLICATION_CONTENT_HEIGHT: usize = APPLICATION_HEIGHT - TITLE_BAR_HEIGHT - 4;
const MENU_WIDTH: usize = 280;
const MENU_HEIGHT: usize = 240;
const MENU_ITEM_COUNT: usize = 3;
const MENU_ITEM_TOP: usize = 42;
const MENU_ITEM_SPACING: usize = 52;
const MENU_ITEM_HEIGHT: usize = 36;
const FILE_BROWSER_WIDTH: usize = 460;
const FILE_BROWSER_HEIGHT: usize = 320;
const FILE_BROWSER_CONTENT_WIDTH: usize = FILE_BROWSER_WIDTH - 8;
const FILE_BROWSER_CONTENT_HEIGHT: usize = FILE_BROWSER_HEIGHT - TITLE_BAR_HEIGHT - 4;
const SYSTEM_CONTROL_WIDTH: usize = 440;
const SYSTEM_CONTROL_HEIGHT: usize = 300;
const SYSTEM_CONTROL_CONTENT_WIDTH: usize = SYSTEM_CONTROL_WIDTH - 8;
const SYSTEM_CONTROL_CONTENT_HEIGHT: usize = SYSTEM_CONTROL_HEIGHT - TITLE_BAR_HEIGHT - 4;
const TASKBAR_TAB_X: usize = 140;
const TASKBAR_TAB_WIDTH: usize = 112;
const TASKBAR_TAB_SPACING: usize = 8;
const CONTROL_SIZE: usize = 14;
const CONTROL_GAP: usize = 2;
const FILE_DOUBLE_CLICK_TICKS: u32 = 45;
const FILE_LIST_TOP: usize = 22;
const FILE_LIST_ROW_HEIGHT: usize = 18;
const FILE_LIST_VISIBLE_ROWS: usize = 12;

/// Index of the currently focused slot, shared with the keyboard event path.
pub static FOCUSED_WINDOW_ID: AtomicUsize = AtomicUsize::new(MAIN_WINDOW_INDEX);
const APPLICATION_CONTENT: [u32; APPLICATION_CONTENT_WIDTH * APPLICATION_CONTENT_HEIGHT] =
    [0x1c2936ff; APPLICATION_CONTENT_WIDTH * APPLICATION_CONTENT_HEIGHT];
const MENU_CONTENT: [u32; MENU_WIDTH * MENU_HEIGHT] = [0x202c3aff; MENU_WIDTH * MENU_HEIGHT];
const FILE_BROWSER_CONTENT: [u32; FILE_BROWSER_CONTENT_WIDTH * FILE_BROWSER_CONTENT_HEIGHT] =
    [0x1c2936ff; FILE_BROWSER_CONTENT_WIDTH * FILE_BROWSER_CONTENT_HEIGHT];
const SYSTEM_CONTROL_CONTENT: [u32; SYSTEM_CONTROL_CONTENT_WIDTH * SYSTEM_CONTROL_CONTENT_HEIGHT] =
    [0x1c2936ff; SYSTEM_CONTROL_CONTENT_WIDTH * SYSTEM_CONTROL_CONTENT_HEIGHT];

#[derive(Clone, Copy, PartialEq, Eq)]
/// Semantic role used to select each window's hit testing and renderer.
enum WindowKind {
    Empty,
    Desktop,
    Taskbar,
    StartButton,
    Application,
    StartMenu,
    FileBrowser,
    SystemControl,
    TextViewer,
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
    RunSamplePe,
    OpenTextFile(EntryInfo),
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
    title_color: u32,
    title: &'static str,
    pixel_buffer: &'static [u32],
    buffer_width: usize,
    buffer_height: usize,
    kind: WindowKind,
    text_buffer: String,
    input_field: TextInputField,
    viewer_path: String,
    viewer_contents: String,
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
        title_color: u32,
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
            title_color,
            title,
            pixel_buffer,
            buffer_width,
            buffer_height,
            kind,
            text_buffer: String::new(),
            input_field: TextInputField::empty(),
            viewer_path: String::new(),
            viewer_contents: String::new(),
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
    /// Creates the desktop, taskbar, launcher, primary window, and spare slots.
    ///
    /// `screen_width` and `screen_height` are framebuffer dimensions. Returns
    /// an error when the screen is too small or the initial window vector cannot
    /// be reserved.
    pub fn new(screen_width: usize, screen_height: usize) -> Result<Self, &'static str> {
        if screen_width < 320 || screen_height < 240 {
            return Err("screen is too small for the desktop layout");
        }

        let taskbar_height = TASKBAR_HEIGHT.min(screen_height);
        let app_width = APPLICATION_WIDTH.min(screen_width.saturating_sub(24));
        let app_height = APPLICATION_HEIGHT.min(screen_height.saturating_sub(taskbar_height));
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
                0x17212fff,
                0,
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
                0x101820ff,
                0,
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
                0x50b8f2ff,
                0,
                &[],
                0,
                0,
            ),
            Window::new(
                WindowKind::Application,
                "Horizon OS",
                (screen_width - app_width) / 2,
                (screen_height - taskbar_height - app_height) / 2,
                app_width,
                app_height,
                3,
                true,
                true,
                TITLE_BAR_HEIGHT,
                0x202c3aff,
                0x26374aff,
                &APPLICATION_CONTENT,
                APPLICATION_CONTENT_WIDTH,
                APPLICATION_CONTENT_HEIGHT,
            ),
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
                0x202c3aff,
                0,
                &MENU_CONTENT,
                MENU_WIDTH,
                MENU_HEIGHT,
            ),
            Window::empty(),
            Window::empty(),
            Window::empty(),
        ]);

        let mut manager = Self {
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
        };
        manager.windows[MAIN_WINDOW_INDEX].input_field.width = app_width.saturating_sub(32);
        manager.windows[MAIN_WINDOW_INDEX].input_field.y = app_height.saturating_sub(38);
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
    /// `counter` is the latest shared-page calculation count.
    pub fn update_background_counter(&mut self, counter: u32) {
        self.background_counter = counter;
    }

    /// Makes the sample PE parse result visible in the primary application window.
    ///
    /// `result` contains validated PE metadata or the parser's error message.
    pub fn show_pe_result(&mut self, result: Result<PeSummary, &'static str>) {
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
            0x202c3aff,
            0x26374aff,
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

    /// Composes visible windows from lowest to highest z-order into the back buffer.
    ///
    /// Returns an error if the temporary z-order list cannot be allocated or a
    /// window renderer reports a graphics failure.
    pub fn render(&self) -> Result<(), &'static str> {
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
                self.render_window(window)?;
            }
        }
        Ok(())
    }

    /// Applies a pointer snapshot to controls, focus, dragging, and launcher hit tests.
    ///
    /// `state` contains screen coordinates, button bits, and an optional press
    /// timestamp. Returns an optional executive action such as opening a file
    /// or running the PE fixture, or an error from the UI allocation/render path.
    pub fn handle_mouse(&mut self, state: MouseState) -> Result<Option<UiAction>, &'static str> {
        let left_down = state.buttons & 1 != 0;
        let left_pressed =
            state.left_pressed_at.is_some() || (left_down && !self.left_button_was_down);
        let mut action = None;

        if left_pressed {
            self.drag = None;
            self.windows[MAIN_WINDOW_INDEX].input_field.focused = false;
            if self.windows[START_MENU_INDEX].contains(state.x, state.y) {
                if let Some(item) = self.start_menu_item_at(state.x, state.y) {
                    self.windows[START_MENU_INDEX].visible = false;
                    match item {
                        0 => self.spawn_file_browser()?,
                        1 => self.spawn_system_control()?,
                        2 => action = Some(UiAction::RunSamplePe),
                        _ => {}
                    }
                }
            } else {
                if self.windows[START_MENU_INDEX].visible {
                    self.windows[START_MENU_INDEX].visible = false;
                }
                if let Some(index) = self.taskbar_tab_at(state.x, state.y) {
                    self.restore_window(index);
                } else if self.windows[START_BUTTON_INDEX].contains(state.x, state.y) {
                    self.toggle_start_menu();
                } else if let Some(index) = self.top_window_at(state.x, state.y) {
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
                    } else if self.windows[index].kind == WindowKind::FileBrowser {
                        action = self.file_browser_click(
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

    fn file_browser_click(
        &mut self,
        mouse_x: usize,
        mouse_y: usize,
        click_tick: u32,
    ) -> Result<Option<UiAction>, &'static str> {
        let Some(window) = self
            .windows
            .iter()
            .find(|window| window.kind == WindowKind::FileBrowser)
        else {
            self.last_file_click = None;
            return Ok(None);
        };
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
        let entry = crate::filesystem::entry_at(index)?
            .ok_or("File Explorer row has no corresponding filesystem entry")?;
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

    fn render_window(&self, window: &Window) -> Result<(), &'static str> {
        if window.kind == WindowKind::Desktop {
            graphics::clear_screen(window.background_color)?;
            graphics::draw_string(24, 24, "Horizon OS", 0x91c9edff)?;
            return graphics::put_pixel(window.width.saturating_sub(16), 16, 0xffd166ff);
        }

        graphics::draw_rectangle(window.x, window.y, window.width, window.height, 0x080d12ff)?;
        graphics::draw_rectangle(
            window.x.saturating_add(2),
            window.y.saturating_add(2),
            window.width.saturating_sub(4),
            window.height.saturating_sub(4),
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
            graphics::draw_rectangle(
                window.x.saturating_add(2),
                window.y.saturating_add(2),
                window.width.saturating_sub(4),
                window.title_bar_height.saturating_sub(2),
                window.title_color,
            )?;
            graphics::draw_string(
                window.x.saturating_add(14),
                window.y.saturating_add(12),
                window.title,
                0xffffffff,
            )?;
            self.render_window_controls(window)?;
        }

        match window.kind {
            WindowKind::Taskbar => {
                graphics::draw_rectangle(window.x, window.y, window.width, 2, 0x34485aff)?;
                graphics::draw_string(
                    window.x.saturating_add(68),
                    window.y.saturating_add(24),
                    "Horizon",
                    0xd7e4efff,
                )?;
                self.render_taskbar_tabs(window)?;
                self.render_clock(window)?;
            }
            WindowKind::StartButton => {
                for row in 0..2 {
                    for column in 0..2 {
                        graphics::draw_rectangle(
                            window.x + 8 + column * 9,
                            window.y + 8 + row * 9,
                            7,
                            7,
                            0xffffffff,
                        )?;
                    }
                }
            }
            WindowKind::Application => self.render_application_content(window)?,
            WindowKind::StartMenu => self.render_start_menu(window)?,
            WindowKind::FileBrowser => self.render_file_browser(window)?,
            WindowKind::SystemControl => self.render_system_control(window)?,
            WindowKind::TextViewer => self.render_text_viewer(window)?,
            WindowKind::Empty | WindowKind::Desktop => {}
        }
        Ok(())
    }

    fn render_window_controls(&self, window: &Window) -> Result<(), &'static str> {
        let y = window.y.saturating_add(7);
        let close_x = window
            .x
            .saturating_add(window.width)
            .saturating_sub(CONTROL_SIZE + 6);
        let maximize_x = close_x.saturating_sub(CONTROL_SIZE + CONTROL_GAP);
        let minimize_x = maximize_x.saturating_sub(CONTROL_SIZE + CONTROL_GAP);

        graphics::draw_rectangle(minimize_x, y, CONTROL_SIZE, CONTROL_SIZE, 0x526577ff)?;
        graphics::draw_rectangle(maximize_x, y, CONTROL_SIZE, CONTROL_SIZE, 0x526577ff)?;
        graphics::draw_rectangle(close_x, y, CONTROL_SIZE, CONTROL_SIZE, 0xff6b6bff)?;
        graphics::draw_rectangle(minimize_x + 4, y + 9, 6, 2, 0xffffffff)?;
        for edge in 0..6 {
            graphics::put_pixel(maximize_x + 4 + edge, y + 4, 0xffffffff)?;
            graphics::put_pixel(maximize_x + 4 + edge, y + 10, 0xffffffff)?;
            graphics::put_pixel(maximize_x + 4, y + 4 + edge, 0xffffffff)?;
            graphics::put_pixel(maximize_x + 9, y + 4 + edge, 0xffffffff)?;
            graphics::put_pixel(close_x + 4 + edge, y + 4 + edge, 0xffffffff)?;
            graphics::put_pixel(close_x + 9 - edge, y + 4 + edge, 0xffffffff)?;
        }
        Ok(())
    }

    fn render_taskbar_tabs(&self, taskbar: &Window) -> Result<(), &'static str> {
        let mut slot = 0;
        for window in &self.windows {
            if !window.draggable || window.view_state != WindowViewState::Minimized {
                continue;
            }
            let x = TASKBAR_TAB_X + slot * (TASKBAR_TAB_WIDTH + TASKBAR_TAB_SPACING);
            graphics::draw_rectangle(
                x,
                taskbar.y.saturating_add(10),
                TASKBAR_TAB_WIDTH,
                36,
                0x26374aff,
            )?;
            graphics::draw_string(
                x.saturating_add(8),
                taskbar.y.saturating_add(24),
                match window.kind {
                    WindowKind::FileBrowser => "Explorer",
                    WindowKind::SystemControl => "System",
                    WindowKind::TextViewer => "Reader",
                    _ => "Horizon",
                },
                0xd7e4efff,
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
            graphics::draw_string(x, y, text, 0xd7e4efff)
        } else {
            let message = if self.clock_error.is_some() {
                "Clock unavailable"
            } else {
                "Clock starting"
            };
            graphics::draw_string(x, y, message, 0xd7e4efff)
        }
    }

    fn render_application_content(&self, window: &Window) -> Result<(), &'static str> {
        let x = window.x.saturating_add(16);
        let y = window.y.saturating_add(TITLE_BAR_HEIGHT + 16);
        graphics::draw_string(x, y, "Welcome to Horizon OS", 0xffffffff)?;
        graphics::draw_string(
            x,
            y.saturating_add(18),
            "Open Start to launch a utility.",
            0xb9c8d6ff,
        )?;
        draw_decimal_line(
            x,
            y.saturating_add(36),
            "Calc:",
            self.background_counter as usize,
            0x7ee0a0ff,
        )?;
        self.render_input_field(window)?;

        if let Some(error) = self.pe_error {
            graphics::draw_string(
                x,
                y.saturating_add(54),
                "Sample PE parse failed:",
                0xffa6a6ff,
            )?;
            graphics::draw_string(x, y.saturating_add(72), error, 0xffa6a6ff)?;
            self.render_window_text(window, y.saturating_add(184))?;
            return Ok(());
        }

        if let Some(report) = self.pe_report {
            graphics::draw_string(
                x,
                y.saturating_add(48),
                "Sample PE32 fixture parsed",
                0x7ee0a0ff,
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
                0xd7e4efff,
            )?;
            let name_length = report
                .first_section_name
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(report.first_section_name.len());
            if name_length > 0 {
                if let Ok(name) = core::str::from_utf8(&report.first_section_name[..name_length]) {
                    graphics::draw_string(x, y.saturating_add(140), "First section:", 0xd7e4efff)?;
                    graphics::draw_string(x + 112, y.saturating_add(140), name, 0xd7e4efff)?;
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
                    graphics::draw_string(x, y.saturating_add(158), "Access:", 0xd7e4efff)?;
                    graphics::draw_string(
                        x.saturating_add(56),
                        y.saturating_add(158),
                        access,
                        0xd7e4efff,
                    )?;
                }
            }
        }
        self.render_window_text(window, y.saturating_add(184))?;
        Ok(())
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
        graphics::draw_string(window.x.saturating_add(16), y, text, 0xd7e4efff)
    }

    fn draw_hex_value(
        &self,
        x: usize,
        y: usize,
        label: &str,
        value: u32,
    ) -> Result<(), &'static str> {
        graphics::draw_string(x, y, label, 0xd7e4efff)?;
        let mut buffer = [0u8; 10];
        buffer[..2].copy_from_slice(b"0x");
        for (index, shift) in (0..8).rev().enumerate() {
            buffer[index + 2] = b"0123456789ABCDEF"[((value >> (shift * 4)) & 0xf) as usize];
        }
        let value = core::str::from_utf8(&buffer).map_err(|_| "failed to format PE metadata")?;
        graphics::draw_string(x.saturating_add(88), y, value, 0xd7e4efff)
    }

    fn render_start_menu(&self, window: &Window) -> Result<(), &'static str> {
        graphics::draw_string(
            window.x.saturating_add(18),
            window.y.saturating_add(14),
            "Horizon Menu",
            0xffffffff,
        )?;
        for (index, title) in ["File Explorer", "System Control", "Run Sample PE"]
            .iter()
            .enumerate()
        {
            let y = window
                .y
                .saturating_add(MENU_ITEM_TOP + index * MENU_ITEM_SPACING);
            graphics::draw_rectangle(
                window.x.saturating_add(12),
                y,
                window.width.saturating_sub(24),
                MENU_ITEM_HEIGHT,
                if index == 0 { 0x34485aff } else { 0x26374aff },
            )?;
            graphics::draw_string(
                window.x.saturating_add(22),
                y.saturating_add(14),
                title,
                0xffffffff,
            )?;
        }
        Ok(())
    }

    fn render_file_browser(&self, window: &Window) -> Result<(), &'static str> {
        let x = window.x.saturating_add(16);
        let y = window.y.saturating_add(TITLE_BAR_HEIGHT + 16);
        graphics::draw_string(x, y, "C:\\", 0x7ec8f8ff)?;
        let entry_count = crate::filesystem::entry_count()?;
        for index in 0..entry_count.min(FILE_LIST_VISIBLE_ROWS) {
            let entry = crate::filesystem::entry_at(index)?
                .ok_or("filesystem index changed while rendering File Explorer")?;
            let path = core::str::from_utf8(&entry.path[..entry.path_length])
                .map_err(|_| "filesystem path is not valid ASCII")?;
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
                    0x7ec8f8ff
                } else {
                    0xb9c8d6ff
                },
            )?;
            graphics::draw_string(x.saturating_add(58), row_y, path, 0xd7e4efff)?;
        }
        if entry_count > FILE_LIST_VISIBLE_ROWS {
            graphics::draw_string(
                x.saturating_add(8),
                y.saturating_add(FILE_LIST_TOP + FILE_LIST_VISIBLE_ROWS * FILE_LIST_ROW_HEIGHT),
                "More items are available on C:\\",
                0xb9c8d6ff,
            )?;
        }
        let readme = crate::filesystem::resolve_windows_path("C:\\README.TXT")?;
        if readme.is_directory {
            return Err("C:\\README.TXT resolved to a directory");
        }
        let mut contents = [0u8; 96];
        let length = crate::filesystem::read_file("C:\\README.TXT", &mut contents)?;
        let first_line = contents[..length]
            .split(|byte| *byte == b'\n' || *byte == b'\r')
            .next()
            .unwrap_or_default();
        let first_line =
            core::str::from_utf8(first_line).map_err(|_| "C:\\README.TXT is not valid ASCII")?;
        graphics::draw_string(
            x.saturating_add(8),
            y.saturating_add(224),
            first_line,
            0x7ee0a0ff,
        )?;
        self.render_window_text(window, y.saturating_add(250))
    }

    fn render_input_field(&self, window: &Window) -> Result<(), &'static str> {
        let field = &window.input_field;
        if field.width == 0 {
            return Ok(());
        }
        let x = window.x.saturating_add(field.x);
        let y = window.y.saturating_add(field.y);
        graphics::draw_rectangle(x, y, field.width, field.height, 0x91a4b6ff)?;
        graphics::draw_rectangle(
            x.saturating_add(1),
            y.saturating_add(1),
            field.width.saturating_sub(2),
            field.height.saturating_sub(2),
            0x101820ff,
        )?;

        let text = if field.text.is_empty() {
            "(type here)"
        } else {
            field.text.as_str()
        };
        let text_x = x.saturating_add(8);
        let text_y = y.saturating_add(9);
        graphics::draw_string(
            text_x,
            text_y,
            text,
            if field.text.is_empty() {
                0x8393a3ff
            } else {
                0xffffffff
            },
        )?;
        if field.focused {
            let caret_x = text_x.saturating_add(field.text.len().saturating_mul(8));
            if caret_x < x.saturating_add(field.width).saturating_sub(4) {
                graphics::draw_rectangle(caret_x, y.saturating_add(6), 2, 14, 0xffffffff)?;
            }
        }
        Ok(())
    }

    fn render_text_viewer(&self, window: &Window) -> Result<(), &'static str> {
        let x = window.x.saturating_add(16);
        let top = window.y.saturating_add(TITLE_BAR_HEIGHT + 16);
        graphics::draw_string(x, top, &window.viewer_path, 0x7ec8f8ff)?;

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
        graphics::draw_string(x, y, "System Control", 0x7ec8f8ff)?;
        graphics::draw_string(x, y.saturating_add(24), "Kernel: Horizon OS", 0xd7e4efff)?;
        graphics::draw_string(x, y.saturating_add(44), "Input: PS/2 mouse", 0xd7e4efff)?;
        graphics::draw_string(
            x,
            y.saturating_add(64),
            "Display: linear framebuffer",
            0xd7e4efff,
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

    fn toggle_start_menu(&mut self) {
        let menu = &mut self.windows[START_MENU_INDEX];
        menu.visible = !menu.visible;
        if menu.visible {
            self.raise_window(START_MENU_INDEX);
        }
    }

    fn spawn_file_browser(&mut self) -> Result<(), &'static str> {
        if let Some(index) = self.find_window(WindowKind::FileBrowser) {
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
        let index = self.insert_window(Window::new(
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
            0x202c3aff,
            0x26374aff,
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
            0x202c3aff,
            0x26374aff,
            &SYSTEM_CONTROL_CONTENT,
            SYSTEM_CONTROL_CONTENT_WIDTH,
            SYSTEM_CONTROL_CONTENT_HEIGHT,
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
        // Closed/minimized slots are reused before the vector grows so stable
        // indices for the desktop, taskbar, launcher, and primary window survive.
        if let Some(index) = self.find_window(WindowKind::Empty) {
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
    graphics::draw_string(x, y, text, 0xd7e4efff)
}
