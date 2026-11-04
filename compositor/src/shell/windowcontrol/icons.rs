//! The app icon over each card: rasterised once per app at the output's
//! scale, and uploaded once per window so no two cards share an element.

use std::collections::HashMap;

use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::{Buffer, Size, Transform},
};

use crate::{
    shell::workspace::Workspace,
    utils::{app_icon, id::WindowId, raster::Pixmap, surface},
};

/// Logical pixels square.
pub const ICON_SIZE: f64 = 36.0;

#[derive(Debug, Default)]
pub struct CardIcons {
    /// Physical pixels square.
    side: u32,
    by_app: HashMap<String, Option<Pixmap>>,
    by_window: HashMap<WindowId, Option<MemoryRenderBuffer>>,
}

impl CardIcons {
    /// Forgets which windows had which icons, keeping every app's pixels.
    pub fn forget_windows(&mut self) {
        self.by_window.clear();
    }

    /// Finds an icon for every window in `windows` that has none yet.
    pub fn update(
        &mut self,
        workspace: &Workspace,
        windows: impl Iterator<Item = WindowId>,
        scale: f64,
    ) {
        let side = (ICON_SIZE * scale).round() as u32;
        if side != self.side {
            self.side = side;
            self.by_app.clear();
            self.by_window.clear();
        }
        for window in windows {
            if self.by_window.contains_key(&window) {
                continue;
            }
            let Some(tile) = workspace.tile(window) else {
                continue;
            };
            let icon = surface::with_app_id(tile.surface(), |app_id| {
                if app_id.is_empty() {
                    return None;
                }
                self.by_app
                    .entry(app_id.to_owned())
                    .or_insert_with(|| app_icon::rasterise(app_id, side))
                    .as_ref()
                    .map(buffer_of)
            });
            self.by_window.insert(window, icon);
        }
    }

    pub fn get(&self, window: WindowId) -> Option<(&MemoryRenderBuffer, Size<i32, Buffer>)> {
        let side = self.side as i32;
        Some((self.by_window.get(&window)?.as_ref()?, (side, side).into()))
    }
}

/// tiny-skia's premultiplied R, G, B, A bytes are `Abgr8888` little-endian.
fn buffer_of(pixmap: &Pixmap) -> MemoryRenderBuffer {
    MemoryRenderBuffer::from_slice(
        pixmap.data(),
        Fourcc::Abgr8888,
        (pixmap.width() as i32, pixmap.height() as i32),
        1,
        Transform::Normal,
        None,
    )
}
