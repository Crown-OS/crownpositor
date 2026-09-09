use smithay::utils::{Logical, Rectangle};

use crate::{
    layout::{
        Direction, Gaps, LayoutInput, LayoutOp, LayoutOutput, TileInfo, TilingLayout,
        WorkspaceMode, placement,
    },
    shell::tile::{Tile, WindowState},
    utils::id::{OutputId, WindowId, WorkspaceId},
};

/// How an action names a workspace. Resolving it is the workspace list's job,
/// not the parser's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRef {
    /// Zero-based index on the active output, clamped to the list.
    Index(usize),
    /// Offset from the active workspace. Does not wrap.
    Relative(i32),
    Previous,
}

/// What changed since the last refresh.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Dirty {
    /// Membership, order or window state changed — rerun the layout.
    pub layout: bool,
    /// Only focus moved — restamp activation, no relayout.
    pub focus: bool,
    /// The usable area moved — rearrange layers, then relayout.
    pub area: bool,
}

impl Dirty {
    pub fn any(self) -> bool {
        self.layout || self.focus || self.area
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

pub struct Workspace {
    id: WorkspaceId,
    output: OutputId,

    /// One `Vec` for both kinds. The `Tiled` subsequence is layout order; the
    /// floating subsequence is back-to-front z-order. They never interleave
    /// semantically, so a single list means one place to look a window up, one
    /// to iterate for rendering, and one to keep the indices honest.
    tiles: Vec<Tile>,

    focus: Option<WindowId>,
    /// Most-recently-used, so closing a dialog returns you to what you were
    /// doing rather than to whatever tile shifted into the slot.
    focus_stack: Vec<WindowId>,

    /// This workspace's own regime. Nothing outside it has a say: the config
    /// value seeds it at construction and never again, so switching one
    /// workspace to floating leaves every other one alone.
    mode: WorkspaceMode,
    /// Kept across a trip through floating, so returning to tiling restores the
    /// master ratio the user had set.
    tiling: TilingLayout,

    /// Workspace-local, exclusive zones and the outer gap already subtracted.
    area: Rectangle<i32, Logical>,
    /// The full output, workspace-local. Fullscreen uses this instead.
    output_area: Rectangle<i32, Logical>,
    gaps: Gaps,

    pub dirty: Dirty,

    scratch_in: Vec<TileInfo>,
    scratch_out: LayoutOutput,
}

impl Workspace {
    pub fn new(output: OutputId, mode: WorkspaceMode, gaps: Gaps) -> Self {
        Self {
            id: WorkspaceId::next(),
            output,
            tiles: Vec::new(),
            focus: None,
            focus_stack: Vec::new(),
            mode,
            tiling: TilingLayout::default(),
            area: Rectangle::default(),
            output_area: Rectangle::default(),
            gaps,
            dirty: Dirty::default(),
            scratch_in: Vec::new(),
            scratch_out: LayoutOutput::default(),
        }
    }

    pub fn id(&self) -> WorkspaceId {
        self.id
    }

    pub fn output(&self) -> OutputId {
        self.output
    }

    pub fn set_output(&mut self, output: OutputId) {
        self.output = output;
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    /// Front-to-back: floating above tiled, topmost first.
    ///
    /// Rendering and hit-testing both go through this, so they cannot disagree
    /// about what is on top.
    pub fn stacking_order(&self) -> impl DoubleEndedIterator<Item = &Tile> {
        let floating = self
            .tiles
            .iter()
            .rev()
            .filter(|tile| tile.state().floats_above());
        let tiled = self
            .tiles
            .iter()
            .rev()
            .filter(|tile| !tile.state().floats_above());
        floating.chain(tiled)
    }

    pub fn tiles_mut(&mut self) -> &mut [Tile] {
        &mut self.tiles
    }

    pub fn tile(&self, id: WindowId) -> Option<&Tile> {
        self.tiles.iter().find(|tile| tile.id() == id)
    }

    pub fn tile_mut(&mut self, id: WindowId) -> Option<&mut Tile> {
        self.tiles.iter_mut().find(|tile| tile.id() == id)
    }

    pub fn contains(&self, id: WindowId) -> bool {
        self.tiles.iter().any(|tile| tile.id() == id)
    }

    pub fn focus(&self) -> Option<WindowId> {
        self.focus
    }

    pub fn area(&self) -> Rectangle<i32, Logical> {
        self.area
    }

    pub fn output_area(&self) -> Rectangle<i32, Logical> {
        self.output_area
    }

    pub fn gaps(&self) -> Gaps {
        self.gaps
    }

    /// At most one, enforced by `set_state` going through here.
    pub fn fullscreen(&self) -> Option<WindowId> {
        self.tiles
            .iter()
            .find(|tile| tile.state() == WindowState::Fullscreen)
            .map(Tile::id)
    }

    pub fn mode(&self) -> WorkspaceMode {
        self.mode
    }

    /// Switches the regime, converting every window that was following the old
    /// one.
    ///
    /// The conversion only retargets each tile, so the springs in `TileAnim`
    /// carry the windows to their new places — the transition needs no
    /// animation machinery of its own.
    pub fn set_mode(&mut self, mode: WorkspaceMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;

        let area = self.area;
        match mode {
            WorkspaceMode::Floating => {
                for (cascade, tile) in self.tiles.iter_mut().enumerate() {
                    if tile.state().is_tiled() {
                        tile.float_into(area, cascade);
                    }
                }
            }
            WorkspaceMode::Tiling => {
                for tile in &mut self.tiles {
                    if tile.state().floats_above() {
                        tile.set_state(WindowState::Tiled);
                    }
                }
            }
        }

        self.dirty.layout = true;
        true
    }

    pub fn set_area(
        &mut self,
        area: Rectangle<i32, Logical>,
        output_area: Rectangle<i32, Logical>,
    ) {
        if self.area == area && self.output_area == output_area {
            return;
        }
        self.area = area;
        self.output_area = output_area;
        self.dirty.layout = true;
    }

    pub fn set_gaps(&mut self, gaps: Gaps) {
        if self.gaps != gaps {
            self.gaps = gaps;
            self.dirty.layout = true;
        }
    }

    // ---- membership: `pub(super)` so only `shell/mod.rs` can move a tile ----

    pub(super) fn push_tile(&mut self, mut tile: Tile) {
        let id = tile.id();

        // A window opening on a floating workspace floats, whatever it arrived
        // as. Without this a new window would be laid out by the tiling layout
        // on a workspace that has none — the single place a tile joins a
        // workspace is the only place that cannot be forgotten.
        if self.mode.is_floating() && tile.state().is_tiled() {
            tile.float_into(self.area, self.tiles.len());
        }

        // Floating windows go to the front of their subsequence (topmost);
        // tiled windows append in layout order.
        if tile.state().floats_above() {
            self.tiles.push(tile);
        } else {
            let insert_at = self
                .tiles
                .iter()
                .position(|existing| existing.state().floats_above())
                .unwrap_or(self.tiles.len());
            self.tiles.insert(insert_at, tile);
        }
        self.focus_stack.push(id);
        self.dirty.layout = true;
    }

    pub(super) fn take_tile(&mut self, id: WindowId) -> Option<Tile> {
        let index = self.tiles.iter().position(|tile| tile.id() == id)?;
        let tile = self.tiles.remove(index);

        // Doing it here rather than in `Shell::remove_tile` means it cannot be
        // forgotten by a new call site.
        self.tiling.forget(id);
        self.focus_stack.retain(|existing| *existing != id);
        if self.focus == Some(id) {
            self.focus = self.focus_stack.last().copied().or_else(|| {
                // Nothing in the MRU stack, so fall back to whatever now
                // occupies the slot the window left.
                self.tiles
                    .get(index)
                    .or_else(|| self.tiles.last())
                    .map(Tile::id)
            });
            self.dirty.focus = true;
        }

        self.dirty.layout = true;
        Some(tile)
    }

    pub fn focus_window(&mut self, id: WindowId) -> bool {
        if !self.contains(id) || self.focus == Some(id) {
            return false;
        }
        self.focus = Some(id);
        self.focus_stack.retain(|existing| *existing != id);
        self.focus_stack.push(id);
        self.dirty.focus = true;

        // Floating windows raise within their subsequence when focused.
        if self
            .tile(id)
            .is_some_and(|tile| tile.state().floats_above())
            && let Some(index) = self.tiles.iter().position(|tile| tile.id() == id)
        {
            let tile = self.tiles.remove(index);
            self.tiles.push(tile);
        }
        true
    }

    /// Sends a window to the back of its own subsequence.
    pub fn lower(&mut self, id: WindowId) -> bool {
        let Some(index) = self.tiles.iter().position(|tile| tile.id() == id) else {
            return false;
        };
        let tile = self.tiles.remove(index);
        let insert_at = if tile.state().floats_above() {
            // Behind the other floating windows, but still above the tiled ones.
            self.tiles
                .iter()
                .position(|other| other.state().floats_above())
                .unwrap_or(self.tiles.len())
        } else {
            0
        };
        self.tiles.insert(insert_at, tile);

        self.focus_stack.retain(|existing| *existing != id);
        self.focus_stack.insert(0, id);
        self.dirty.layout = true;
        true
    }

    /// The tiling layout's own answer, falling back to list order.
    ///
    /// The fallback is what makes vertical movement work, where the layout only
    /// has an opinion about crossing the master/stack split.
    pub fn neighbour(&self, from: WindowId, dir: Direction) -> Option<WindowId> {
        let tiles: Vec<TileInfo> = self
            .tiles
            .iter()
            .filter(|tile| tile.state().is_tiled())
            .map(Tile::info)
            .collect();

        if let Some(id) = self.tiling.neighbour(&tiles, from, dir) {
            return Some(id);
        }

        let index = tiles.iter().position(|tile| tile.id == from)?;
        let next = match dir {
            Direction::Left | Direction::Up => index.checked_sub(1)?,
            Direction::Right | Direction::Down => index + 1,
        };
        tiles.get(next).map(|tile| tile.id)
    }

    /// Swaps two windows' places in the layout order.
    pub fn swap(&mut self, a: WindowId, b: WindowId) -> bool {
        let (Some(x), Some(y)) = (
            self.tiles.iter().position(|tile| tile.id() == a),
            self.tiles.iter().position(|tile| tile.id() == b),
        ) else {
            return false;
        };
        if x == y {
            return false;
        }
        self.tiles.swap(x, y);
        self.dirty.layout = true;
        true
    }

    pub fn apply_layout_op(&mut self, op: LayoutOp) -> bool {
        self.scratch_in.clear();
        self.scratch_in.extend(
            self.tiles
                .iter()
                .filter(|tile| tile.state().is_tiled())
                .map(Tile::info),
        );

        let input = LayoutInput {
            area: self.area,
            gaps: self.gaps,
            focused: self.focus,
            tiles: &self.scratch_in,
        };

        let changed = self.tiling.apply(op, &input);
        self.dirty.layout |= changed;
        changed
    }

    /// Assigns every tile a target rect. Returns the ids whose size changed, so
    /// the caller knows exactly who needs a configure.
    pub(super) fn arrange(&mut self, resized: &mut Vec<WindowId>) {
        let fullscreen = self.fullscreen();
        let (area, output_area, gaps) = (self.area, self.output_area, self.gaps);

        self.scratch_in.clear();
        self.scratch_in.extend(
            self.tiles
                .iter()
                .filter(|tile| tile.state().is_tiled() && Some(tile.id()) != fullscreen)
                .map(Tile::info),
        );

        if self.scratch_in.is_empty() {
            self.scratch_out.clear();
        } else {
            let input = LayoutInput {
                area,
                gaps,
                focused: self.focus,
                tiles: &self.scratch_in,
            };
            self.tiling.arrange(&input, &mut self.scratch_out);
            debug_assert_eq!(
                self.scratch_out.rects.len(),
                self.scratch_in.len(),
                "the tiling layout must return exactly one rect per tile"
            );
        }

        let mut next = 0;
        for tile in &mut self.tiles {
            // Every state but `Floating` has the compositor dictating a size,
            // which is what takes the decision away from the client for good.
            if !tile.state().is_floating() {
                tile.claim_size();
            }

            let rect = match tile.state() {
                WindowState::Fullscreen => output_area,
                WindowState::Maximized => area,
                WindowState::Snapped(zone) => zone.rect(area, gaps),
                // Free to hang off any edge the drag took it over, but never
                // so far that there is nothing left to catch it by.
                WindowState::Floating => {
                    placement::keep_reachable(tile.floating_rect(), area, tile.insets().top)
                }
                WindowState::Tiled => {
                    // A fullscreen window is excluded from the layout, so it
                    // keeps whatever it had until it comes back.
                    if Some(tile.id()) == fullscreen {
                        tile.target()
                    } else {
                        let rect = self.scratch_out.rects.get(next).copied().unwrap_or(area);
                        next += 1;
                        rect
                    }
                }
            };

            if tile.set_target(rect) {
                resized.push(tile.id());
            }
        }
    }
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("id", &self.id)
            .field("mode", &self.mode)
            .field("tiles", &self.tiles.len())
            .field("focus", &self.focus)
            .finish_non_exhaustive()
    }
}
