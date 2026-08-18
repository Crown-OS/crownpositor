use std::collections::HashMap;

use smithay::{
    desktop::{Space, Window},
    utils::{Logical, Point},
};

type WindowId = u64;

#[derive(Clone, Copy)]
pub struct WindowState {}

impl WindowState {
    pub fn new() -> Self {
        WindowState {}
    }
}

#[derive(Clone, Copy)]
pub enum LayoutMode {
    Floating,
    Tiling,
}

#[derive(Clone)]
pub struct LayoutManager {
    next_window_id: WindowId,
    layout_mode: LayoutMode,
    windows: HashMap<WindowId, Window>,
}

impl LayoutManager {
    pub fn new(layout_mode: LayoutMode) -> Self {
        LayoutManager {
            next_window_id: 0,
            layout_mode,
            windows: HashMap::new(),
        }
    }

    pub fn toggle_mode(&mut self) {
        self.layout_mode = match self.layout_mode {
            LayoutMode::Floating => LayoutMode::Tiling,
            LayoutMode::Tiling => LayoutMode::Floating,
        }
    }

    pub fn add_window(&mut self, window: Window) -> Point<i32, Logical> {
        let id = self.next_window_id;
        self.next_window_id += 1;

        self.windows.insert(id, window);
        let position = Point::from((100, 100));
        position
    }

    pub fn remove_window(&mut self, window_id: WindowId) {
        self.windows.remove(&window_id);
    }
}
