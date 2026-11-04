//! Builds the render element list for one output.
//!
//! Generic over the renderer and over what a backend can do to a window, so
//! adding a backend needs no changes here — it supplies its own renderer and a
//! [`TileDecorator`], and gets the same scene graph.
//!
//! Windows are collected from the shell model rather than through
//! `desktop::space::render_output`: that helper draws its custom elements in
//! front of the space, which is right for Overlay and Top and wrong for Bottom
//! and Background, so a wallpaper would cover the desktop.

pub mod blur;
pub mod color;
pub mod corners;
pub mod cursor;
pub mod decorate;
pub mod decoration;
pub mod element;
mod frame;
pub mod framebuffer;
pub mod fullscreen;
pub mod glazed;
pub mod gles;
pub mod lock;
mod lock_glyph;
pub mod lock_screen;
mod morph;
pub mod opaque;
pub mod overview;
mod painter;
pub mod popup;
pub mod rounded;
pub mod shaded_sprite;
pub mod snapshot;
pub mod sources;
pub mod surface_tree;
pub mod unstretched;
pub mod warp;
mod windowcontrol;

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            Element, Id, Kind, Wrap, memory::MemoryRenderBufferRenderElement,
            utils::CropRenderElement,
        },
        utils::CommitCounter,
    },
    desktop::layer_map_for_output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use config::{Appearance, MinimizeAnimation};

use crate::{
    rendering::{
        blur::{Glass, GlassKind},
        cursor::Cursor,
        decorate::{Backdrop, Glaze, Glazing, Outline, Shadow, TileDecorator},
        decoration::{Border, FramePalette, TextRenderer},
        element::CrownElement,
        frame::{Frame, client_area},
        unstretched::Unstretched,
        warp::{Motion, WarpRequest},
    },
    shell::{
        Shell,
        decoration::Control,
        gaming,
        monitor::Monitor,
        morph::Morph,
        tile::{Tile, WindowState},
        workspace::Workspace,
    },
    utils::id::WindowId,
};

/// Everything a frame needs that is not the window itself.
///
/// Built once per output per frame: the palettes and the corner radius are the
/// same for every window on screen, and rebuilding them per tile would resolve
/// the theme dozens of times a frame.
pub struct FrameStyle<'a> {
    /// Corner radius in physical pixels, the space the shaders work in.
    pub radius: f32,
    pub scale: f64,
    /// The output's global origin, so the menu geometry — which is global,
    /// because that is the space the pointer arrives in — can be brought back
    /// into the output-local space the damage tracker works in.
    pub origin: Point<i32, Logical>,
    pub dark: bool,
    focused_palette: FramePalette,
    unfocused_palette: FramePalette,
    pub text: &'a mut TextRenderer,
    pub focused: Option<WindowId>,
    /// Which control the pointer is over, and on which window.
    pub hovered: Option<(WindowId, Control)>,
    pub minimize: MinimizeAnimation,
}

impl<'a> FrameStyle<'a> {
    pub fn new(
        appearance: &Appearance,
        scale: f64,
        origin: Point<i32, Logical>,
        text: &'a mut TextRenderer,
        focused: Option<WindowId>,
        hovered: Option<(WindowId, Control)>,
    ) -> Self {
        Self {
            radius: (appearance.border_radius as f64 * scale) as f32,
            scale,
            origin,
            dark: appearance.dark_mode,
            focused_palette: FramePalette::new(appearance, true),
            unfocused_palette: FramePalette::new(appearance, false),
            text,
            focused,
            hovered,
            minimize: appearance.minimize_effect,
        }
    }

    fn palette(&self, focused: bool) -> FramePalette {
        if focused {
            self.focused_palette
        } else {
            self.unfocused_palette
        }
    }

    fn hover_on(&self, id: WindowId) -> Option<Control> {
        self.hovered
            .and_then(|(window, control)| (window == id).then_some(control))
    }
}

/// One output's scene graph, for a given renderer and decorator.
pub type Elements<R, D> = Vec<CrownElement<R, <D as TileDecorator<R>>::Element>>;

/// Front-to-back, the order `OutputDamageTracker::render_output` wants.
///
/// The strata, nearest the eye first — and the order the pointer is routed
/// in, so what answers a click is what is drawn under it:
///
/// 1. The cursor and the icon a drag carries.
/// 2. A lock screen, which hides everything below until it lifts.
/// 3. The compositor's open menu.
/// 4. Every window's popups, over every window and panel.
/// 5. The Overlay layer, a window lifting its icon out of a dock, the Top
///    layer, and the preview of where a dragged window will land.
/// 6. The windows: the overview or the Alt+Tab stack standing in for them,
///    or each visible workspace's stack, closing windows first.
/// 7. The Bottom and Background layers.
///
/// A settled fullscreen window short-circuits to its menu, its popups and
/// its own surface, so the buffer can go straight to a plane.
#[allow(clippy::too_many_arguments)]
pub fn output_elements<R, D>(
    shell: &Shell,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    cursor: &mut Cursor,
    pointer: Point<f64, Logical>,
    scale: Scale<f64>,
    style: &mut FrameStyle<'_>,
) -> Elements<R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let mut elements = Elements::<R, D>::new();

    // First in the list is nearest the eye: the cursor is over everything,
    // fullscreen windows included.
    cursor.render(
        &mut elements,
        renderer,
        decorator,
        monitor.geometry().loc,
        pointer,
        scale,
    );

    // A locked session draws the lock and nothing else, whatever mode the
    // desktop behind it was in — until an accepted password lets the desktop
    // show through as the lock lifts.
    if let Some(lock) = shell.lock_screen.active() {
        let reveal = lock_screen::lock_screen_elements::<R, D>(
            &mut elements,
            lock,
            monitor,
            renderer,
            decorator,
            scale,
        );
        if !reveal {
            return elements;
        }
    } else if shell.session_lock.is_active() {
        lock::lock_elements::<R, D>(&mut elements, &shell.session_lock, monitor, renderer, scale);
        return elements;
    }

    // A settled fullscreen window is all there is to see. Panels and
    // notifications stay hidden, so its buffer can go straight to a plane;
    // only a menu the user opened is drawn over it.
    if let Some(tile) = gaming::scanout_tile(monitor) {
        menu_elements(
            &mut elements,
            shell,
            monitor,
            renderer,
            decorator,
            scale,
            style,
        );
        fullscreen::fullscreen_elements::<R, D>(&mut elements, tile, renderer, decorator, scale);
        return elements;
    }

    // Nearest the eye after the cursor: an open menu, then every window's
    // popups. Both float over the panels — a menu was opened deliberately and
    // dismisses itself, and a popup is free to hang past the window that
    // opened it and past whatever panel lies below — and the pointer is
    // routed in the same order, so what answers a click is what is drawn.
    menu_elements(
        &mut elements,
        shell,
        monitor,
        renderer,
        decorator,
        scale,
        style,
    );
    let desktop = !monitor.spacecontrol().is_visible() && !monitor.window_control().is_visible();
    if desktop {
        for (workspace, offset) in monitor.visible_workspaces() {
            popup::workspace_popup_elements(
                &mut elements,
                workspace,
                renderer,
                decorator,
                scale,
                offset,
            );
        }
    }

    layer_elements(
        &mut elements,
        monitor,
        renderer,
        decorator,
        scale,
        &[Layer::Overlay],
    );
    // A window lifting its icon out of a dock is drawn over it, so the icon
    // leaves the dock instead of sliding under its glass.
    if desktop {
        for (workspace, offset) in monitor.visible_workspaces() {
            workspace_window_elements(
                &mut elements,
                shell,
                workspace,
                renderer,
                decorator,
                scale,
                offset,
                style,
                Stacking::Lifted,
            );
        }
    }
    layer_elements(
        &mut elements,
        monitor,
        renderer,
        decorator,
        scale,
        &[Layer::Top],
    );

    // Above every window, below the panels: the preview is a hint about where
    // the drag will land, and it has to be visible over whatever it will cover.
    snap_preview_elements(
        &mut elements,
        shell,
        monitor,
        renderer,
        decorator,
        scale,
        style,
    );

    // The overview stands in for the workspaces entirely: it draws the same
    // windows, on their way to or from their thumbnails, so drawing both would
    // be drawing every window twice.
    if monitor.spacecontrol().is_visible() {
        overview::overview_elements(&mut elements, monitor, renderer, decorator, scale, style);
        return elements;
    }

    // One workspace once the viewport has settled, two while it is sliding —
    // and the slide is nothing but the offset each tile is drawn at, so the GPU
    // recomposites textures it already holds instead of anyone touching pixels.
    // Layer surfaces sit outside the loop: a bar does not travel with the
    // workspace under it.
    // The Alt+Tab strip draws the active workspace itself, stepped back
    // behind it; the wallpaper and the panels below stay where they are.
    let mut covered = false;
    let workspaces = match monitor.window_control().is_visible() {
        true => {
            windowcontrol::window_control_elements(
                &mut elements,
                monitor,
                renderer,
                decorator,
                scale,
                style,
            );
            None
        }
        false => Some(monitor.visible_workspaces()),
    };
    for (workspace, offset) in workspaces.into_iter().flatten() {
        let fullscreen = workspace_window_elements(
            &mut elements,
            shell,
            workspace,
            renderer,
            decorator,
            scale,
            offset,
            style,
            Stacking::BelowPanels,
        );
        covered |= fullscreen && offset.x == 0.0;
    }

    if !covered {
        layer_elements(
            &mut elements,
            monitor,
            renderer,
            decorator,
            scale,
            &[Layer::Bottom, Layer::Background],
        );
    }

    elements
}

/// Which of a workspace's windows a pass over it draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stacking {
    /// Every window.
    All,
    /// Every window but those lifting an icon out of a launcher.
    BelowPanels,
    /// Only the windows lifting an icon out of a launcher, drawn over the
    /// panels it sits in.
    Lifted,
}

impl Stacking {
    fn draws(self, tile: &Tile, effect: MinimizeAnimation) -> bool {
        let lifted = effect == MinimizeAnimation::Liquid
            && tile
                .morph()
                .is_some_and(|morph| morph.origin().carrier.is_some());
        match self {
            Self::All => true,
            Self::BelowPanels => !lifted,
            Self::Lifted => lifted,
        }
    }
}

/// The warp a morphing window is drawn with, if the style bends it at all.
fn morph_motion<'a>(style: &FrameStyle<'_>, morph: &'a Morph) -> Option<Motion<'a>> {
    match style.minimize {
        MinimizeAnimation::Liquid => Some(Motion::Liquid(morph)),
        MinimizeAnimation::Genie => Some(Motion::Genie(morph)),
        MinimizeAnimation::Scale => None,
    }
}

/// One workspace's windows that `stacking` draws, closing ones included, at
/// `offset` from where the workspace normally sits. Returns whether a settled
/// fullscreen window covered the whole page, so nothing below it needs
/// drawing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn workspace_window_elements<R, D>(
    elements: &mut Elements<R, D>,
    shell: &Shell,
    workspace: &Workspace,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    offset: Point<f64, Logical>,
    style: &mut FrameStyle<'_>,
    stacking: Stacking,
) -> bool
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    // Frontmost: a window that just closed shrinks away over everything
    // that was behind it.
    let effect = style.minimize;
    for closing in workspace
        .closing()
        .iter()
        .filter(|closing| stacking.draws(closing.tile(), effect))
    {
        let tile = closing.tile();
        let radius = match tile.state() {
            WindowState::Fullscreen => 0.0,
            _ => style.radius,
        };
        if let Some(dust) = closing.dust() {
            let request = WarpRequest {
                shell,
                tile,
                content: morph::Content::Snapshot(closing.snapshot()),
                motion: Motion::Dust(dust),
                scale,
                offset,
                radius,
                style,
            };
            if let Some(element) = decorator.warp(renderer, request) {
                elements.push(CrownElement::Tile(Wrap::from(element)));
            }
        } else if let Some(morph) = tile.morph() {
            if style.minimize == MinimizeAnimation::Liquid {
                let request = WarpRequest {
                    shell,
                    tile,
                    content: morph::Content::Snapshot(closing.snapshot()),
                    motion: Motion::Liquid(morph),
                    scale,
                    offset,
                    radius,
                    style,
                };
                if let Some(element) = decorator.warp(renderer, request) {
                    elements.push(CrownElement::Tile(Wrap::from(element)));
                    continue;
                }
            }
            morph::morph_elements(
                elements,
                shell,
                tile,
                morph,
                morph::Content::Snapshot(closing.snapshot()),
                renderer,
                decorator,
                scale,
                offset,
                radius,
                style,
            );
        }
    }
    // A fullscreen window on its way out of or into its icon has the
    // desktop showing round it, so it is drawn like any other.
    let fullscreen = workspace.fullscreen().and_then(|id| workspace.tile(id));
    match fullscreen.filter(|tile| tile.morph().is_none() && stacking.draws(tile, effect)) {
        // A fullscreen window covers its page edge to edge, so rounding it
        // would just cut four notches out of the display — and while it is
        // the only page on screen there is nothing behind it to draw.
        Some(tile) => {
            // Square corners and no frame: a fullscreen window covers its
            // page edge to edge, so rounding it would cut four notches out
            // of the display.

            tile_elements(
                elements, shell, tile, renderer, decorator, scale, offset, 0.0, style,
            );
            true
        }
        None => {
            for tile in workspace.stacking_order() {
                if !stacking.draws(tile, effect) {
                    continue;
                }
                let radius = match tile.state() {
                    WindowState::Fullscreen => 0.0,
                    _ => style.radius,
                };
                tile_elements(
                    elements, shell, tile, renderer, decorator, scale, offset, radius, style,
                );
            }
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn tile_elements<R, D>(
    elements: &mut Elements<R, D>,
    shell: &Shell,
    tile: &Tile,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    offset: Point<f64, Logical>,
    radius: f32,
    style: &mut FrameStyle<'_>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    if let Some(morph) = tile.morph() {
        if let Some(motion) = morph_motion(style, morph) {
            let request = WarpRequest {
                shell,
                tile,
                content: morph::Content::Live,
                motion,
                scale,
                offset,
                radius,
                style,
            };
            if let Some(element) = decorator.warp(renderer, request) {
                elements.push(CrownElement::Tile(Wrap::from(element)));
                return;
            }
        }
        morph::morph_elements(
            elements,
            shell,
            tile,
            morph,
            morph::Content::Live,
            renderer,
            decorator,
            scale,
            offset,
            radius,
            style,
        );
        return;
    }

    let Placed {
        frame,
        inset,
        rim,
        clip,
        surface_origin,
    } = Placed::new(tile, offset, scale);
    let reveal = tile.titlebar_reveal();
    let alpha = tile.render_alpha();

    let framed = (inset > 0).then(|| Frame::desktop(tile, frame, inset, radius));
    if let Some(framed) = &framed {
        framed.overlay(elements, renderer, decorator, style, alpha * reveal);
        menu_row_elements::<R, D>(
            elements,
            shell,
            tile.id(),
            renderer,
            scale,
            style,
            alpha * reveal,
        );
    }

    // A client that set its own corner radius through
    // `crownos_background_effects` is asking to be clipped to it, so that wins
    // for its surface — and for the glass behind it, which has to be cut from
    // the same shape. The *frame's* radius is left alone: the titlebar is the
    // compositor's, and a client does not get to restyle it.
    let surface = blur::window_surface(tile.window());
    let client_radius = surface
        .as_ref()
        .and_then(|surface| blur::surface_corner_radius(surface, scale.x));

    let radii = match client_radius {
        Some(radius) => [radius; 4],
        None => content_radii(radius, rim as f32),
    };

    let surfaces = surface_tree::transformed_surface_elements(
        renderer,
        tile.surface(),
        surface_origin,
        scale,
        alpha,
        Kind::Unspecified,
    );

    for scaled in surfaces {
        // A client's buffer is whatever size it last committed — during a shrink
        // still the *old* size — so without the clip it bleeds over its
        // neighbour. `from_element` returns `None` when the element falls
        // entirely outside, which is exactly what should not be drawn.
        let Some(cropped) = CropRenderElement::from_element(scaled, scale, clip) else {
            continue;
        };
        if let Some(decorated) = decorator.decorate(renderer, cropped, clip, radii) {
            elements.push(CrownElement::Tile(Wrap::from(decorated)));
        }
    }

    // The blurred glass goes in *after* the window's surfaces — later in the
    // list is further from the eye, so it sits directly behind them. The
    // surface origin is the space the client expressed its blur region in, and
    // it is cut to the client's own shape. Inside a frame it is flat: the
    // frame's sheet lights the window's edge, and a second lit edge round the
    // client would read as a seam.
    if let Some(surface) = surface {
        let outline = Outline {
            rect: clip,
            radius: radii[0],
        };
        let glass = decorator.glass(scale.x, GlassKind::Window);
        let glass = match framed {
            Some(_) => glass.flat(),
            None => glass,
        };
        backdrop_elements(
            &mut |element| elements.push(CrownElement::Tile(Wrap::from(element))),
            renderer,
            decorator,
            &surface,
            surface_origin,
            scale,
            clip,
            outline,
            alpha,
            glass,
            1.0,
        );
    }

    // Last, so it is behind the client as well as behind the frame's own
    // overlay: the frame's material has to show through every part of the
    // window that is not the client — the titlebar, the corner notches the
    // client's rounding cuts away, and the ring.
    if let Some(framed) = framed {
        framed.backing(elements, renderer, decorator, style, alpha * reveal);
    }
}

/// Where a tile's pieces land on the output this frame.
///
/// The interpolated rect, not the target: this is what makes a window slide.
/// Output-local, because the damage tracker works in this output's own space.
/// `offset` slides the whole workspace the tile belongs to; rounding only at
/// the physical step keeps both motions sub-pixel smooth.
pub(crate) struct Placed {
    /// The whole window, frame included.
    pub frame: Rectangle<i32, Physical>,
    /// How much of the frame's top the titlebar takes.
    pub inset: i32,
    /// How much of its other three sides the rim takes.
    pub rim: i32,
    /// Where the client actually draws. Equal to the frame when undecorated,
    /// so the undecorated path costs nothing extra.
    pub clip: Rectangle<i32, Physical>,
    /// The client's own origin, which sits its window geometry's offset up and
    /// to the left of the clip: a client that draws its own shadows keeps them
    /// in its buffer, outside the rect it asked to be framed.
    pub surface_origin: Point<i32, Physical>,
}

impl Placed {
    pub(crate) fn new(tile: &Tile, offset: Point<f64, Logical>, scale: Scale<f64>) -> Self {
        let animated = tile.render_rect();
        let frame_loc: Point<i32, Physical> =
            (animated.loc + offset).to_physical_precise_round(scale);
        let frame = Rectangle::new(frame_loc, animated.size.to_physical_precise_round(scale));
        let inset = (tile.render_titlebar() * scale.y).round() as i32;
        let rim = (tile.render_rim() * scale.x).round() as i32;
        let clip = client_area(frame, inset, rim);
        let surface_origin = clip.loc
            - tile
                .window()
                .geometry()
                .loc
                .to_physical_precise_round(scale);
        Self {
            frame,
            inset,
            rim,
            clip,
            surface_origin,
        }
    }
}

/// The menu row's labels along a titlebar.
///
/// Pushed by [`tile_elements`] for the window that owns them, from the layout
/// [`State::layout_menus`](crate::state::State::layout_menus) recorded before
/// the frame — so what is drawn is exactly what a click will find.
fn menu_row_elements<R, D>(
    elements: &mut Elements<R, D>,
    shell: &Shell,
    window: WindowId,
    renderer: &mut R,
    scale: Scale<f64>,
    style: &mut FrameStyle<'_>,
    alpha: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let Some((owner, row)) = shell.menus.row() else {
        return;
    };
    if owner != window || row.is_empty() {
        return;
    }

    let Some(bar) = shell.menus.bar(window) else {
        return;
    };
    let palette = style.palette(true);
    let open = shell
        .menus
        .open()
        .and_then(|open| open.path.first().copied());

    for entry in row {
        let Some(item) = bar.items.iter().find(|item| item.id == entry.id) else {
            continue;
        };
        // The open menu's own label reads as pressed, which is the only cue
        // that says which of them is showing.
        let color = if Some(entry.id) == open {
            palette.label
        } else {
            [
                palette.label[0],
                palette.label[1],
                palette.label[2],
                palette.label[3] * 0.78,
            ]
        };

        let Some(label) = style.text.label(&item.label, style.scale, color, false) else {
            continue;
        };
        // `entry` is global; the output's origin brings it back into the space
        // the damage tracker works in.
        let location: Point<i32, Physical> = (entry.label - style.origin)
            .to_f64()
            .to_physical_precise_round(scale);
        let centred = Point::<i32, Physical>::from((
            location.x,
            location.y + ((entry.rect.size.h as f64 * scale.y) as i32 - label.size.h) / 2,
        ));

        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            centred.to_f64(),
            &label.buffer,
            Some(alpha),
            None,
            None,
            Kind::Unspecified,
        ) {
            elements.push(Unstretched::at_own_size(element, scale).into());
        }
    }
}

/// The open menu: its labels, its glass panel and its outline.
#[allow(clippy::too_many_arguments)]
fn menu_elements<R, D>(
    elements: &mut Elements<R, D>,
    shell: &Shell,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    style: &mut FrameStyle<'_>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let Some(popup) = shell.menus.popup() else {
        return;
    };
    if !monitor.geometry().overlaps(popup.frame) {
        return;
    }

    let palette = style.palette(true);
    let highlighted = shell.menus.open().and_then(|open| open.highlighted);
    let items = shell
        .menus
        .open_items()
        .map(|(_, items)| items)
        .unwrap_or_default();
    let commit = CommitCounter::default();
    let local = |rect: Rectangle<i32, Logical>| {
        Rectangle::new(
            (rect.loc - style.origin).to_physical_precise_round(scale),
            rect.size.to_physical_precise_round(scale),
        )
    };

    // Labels first: earlier in the list is nearer the eye.
    for row in &popup.rows {
        let Some(item) = items.iter().find(|item| item.id == row.id) else {
            continue;
        };
        if !row.actionable {
            continue;
        }

        let faded = if item.enabled { 1.0 } else { 0.45 };
        let color = [
            palette.label[0],
            palette.label[1],
            palette.label[2],
            palette.label[3] * faded,
        ];
        let Some(label) = style.text.label(&item.label, style.scale, color, false) else {
            continue;
        };

        let placed = local(row.rect);
        let location = Point::<i32, Physical>::from((
            placed.loc.x + ((popup.label_origin(row).x - row.rect.loc.x) as f64 * scale.x) as i32,
            placed.loc.y + (placed.size.h - label.size.h) / 2,
        ));

        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location.to_f64(),
            &label.buffer,
            Some(1.0),
            None,
            None,
            Kind::Unspecified,
        ) {
            elements.push(Unstretched::at_own_size(element, scale).into());
        }
    }

    // The highlight behind whichever row the pointer is on.
    if let Some(row) = popup
        .rows
        .iter()
        .find(|row| row.actionable && Some(row.id) == highlighted)
        && let Some(highlight) = decorator.border(
            renderer,
            Border {
                id: shell.menus.highlight_id().clone(),
                commit,
                window: local(row.rect),
                thickness: (row.rect.size.h as f64 * scale.y / 2.0) as f32,
                radius: style.radius * 0.5,
                color: palette.control_fill,
                alpha: 1.0,
            },
        )
    {
        elements.push(CrownElement::Tile(Wrap::from(highlight)));
    }

    let frame = local(popup.frame);
    if let Some(border) = decorator.border(
        renderer,
        Border {
            id: shell.menus.border_id().clone(),
            commit,
            window: frame,
            thickness: 1.0 * style.scale as f32,
            radius: style.radius,
            color: palette.border,
            alpha: 1.0,
        },
    ) {
        elements.push(CrownElement::Tile(Wrap::from(border)));
    }

    if decorator.blur_fingerprint().is_some()
        && let Some(glass) = decorator.backdrop(
            renderer,
            Backdrop {
                id: shell.menus.glass_id().clone(),
                commit,
                geometry: frame,
                mask: frame,
                radius: style.radius,
                glass: decorator.glass(style.scale, GlassKind::Menu),
                alpha: 1.0,
                strength: 1.0,
            },
        )
    {
        elements.push(CrownElement::Tile(Wrap::from(glass)));
    }
}

/// The translucent rectangle showing where a dragged window would land.
///
/// Built from the same two pieces a frame is — a sheet of blurred glass and a
/// hairline around it — so it needs no shader of its own and reads as the same
/// material as the titlebar the user is dragging.
#[allow(clippy::too_many_arguments)]
fn snap_preview_elements<R, D>(
    elements: &mut Elements<R, D>,
    shell: &Shell,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    style: &FrameStyle<'_>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    let Some(preview) = shell.snap_preview() else {
        return;
    };
    if preview.output() != monitor.id() || preview.alpha() <= 0.0 {
        return;
    }

    let rect = preview.render_rect();
    let geometry = Rectangle::new(
        rect.loc.to_physical_precise_round(scale),
        rect.size.to_physical_precise_round(scale),
    );
    if geometry.is_empty() {
        return;
    }

    let palette = style.palette(true);
    let alpha = preview.alpha();
    // The preview is regenerated from its own geometry every frame, and the
    // damage tracker follows the geometry, so the counter never has to move.
    let commit = CommitCounter::default();

    if let Some(border) = decorator.border(
        renderer,
        Border {
            id: preview.ids().border.clone(),
            commit,
            window: geometry,
            thickness: 2.0 * style.scale as f32,
            radius: style.radius,
            color: palette.border,
            alpha,
        },
    ) {
        elements.push(CrownElement::Tile(Wrap::from(border)));
    }

    if decorator.blur_fingerprint().is_some()
        && let Some(glass) = decorator.backdrop(
            renderer,
            Backdrop {
                id: preview.ids().glass.clone(),
                commit,
                geometry,
                mask: geometry,
                radius: style.radius,
                glass: decorator.glass(style.scale, GlassKind::Menu),
                alpha: alpha * 0.85,
                strength: 1.0,
            },
        )
    {
        elements.push(CrownElement::Tile(Wrap::from(glass)));
    }
}

/// The client's corners inside a frame `rim` wide: concentric with the
/// frame's own, so the band between them is even all the way round.
fn content_radii(radius: f32, rim: f32) -> [f32; 4] {
    [(radius - rim).max(0.0); 4]
}

/// Pushes everything a surface asked the compositor to draw for it: the glass
/// behind it and the shadow under it.
///
/// Two protocols land here. `crownos_background_effects` is the richer one —
/// parametric shapes, tint, vibrancy, a refractive rim and a shadow — and wins
/// where a surface has committed to it; `ext-background-effect-v1` is the
/// portable one, a `wl_region` of blurred rectangles in the compositor's own
/// material. A surface that uses neither costs the two lookups below.
///
/// `origin` is where the surface's own `(0, 0)` lands, and `clip` is the
/// rectangle effects are clipped to — a window's animated rect, or a layer
/// surface's geometry. For the portable protocol `outline` is the shape the
/// glass is cut from and lit as, which reaches past `clip` when the surface is
/// one part of a bigger shape.
/// Elements go to `out` rather than into a list, because the overview draws
/// the very same effects into a list of its own: a thumbnail is the window at
/// a different size, and the blur it stands on is the window's own.
#[allow(clippy::too_many_arguments)]
pub(crate) fn backdrop_elements<R, D>(
    out: &mut dyn FnMut(D::Element),
    renderer: &mut R,
    decorator: &mut D,
    surface: &WlSurface,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
    clip: Rectangle<i32, Physical>,
    outline: Outline,
    alpha: f32,
    glass: Glass,
    strength: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    // No backdrops to draw means nothing to work out: this is the path every
    // frame takes on a backend without a blur pipeline, or with blur off.
    let Some(fingerprint) = decorator.blur_fingerprint() else {
        return;
    };

    if let Some(effects) = blur::place_surface_effects(surface, origin, scale, clip, glass.shading)
    {
        surface_effect_elements(
            out,
            renderer,
            decorator,
            surface,
            fingerprint,
            effects,
            alpha,
            strength,
        );
        return;
    }

    let mut rects = Vec::new();
    let Some(generation) = blur::place_blur_region(surface, origin, scale, clip, &mut rects) else {
        return;
    };

    let (ids, commit) = blur::backdrop_slots(surface, rects.len(), fingerprint, generation);
    for (id, geometry) in std::iter::zip(ids, rects) {
        if let Some(backdrop) = decorator.backdrop(
            renderer,
            Backdrop {
                id,
                commit,
                geometry,
                mask: outline.rect,
                radius: outline.radius,
                glass,
                alpha,
                strength,
            },
        ) {
            out(backdrop);
        }
    }
}

/// The `crownos_background_effects` half of [`backdrop_elements`].
///
/// Shadows first and glass after, the reverse of how they stack: a shadow is
/// cut away under the glass that casts it, so drawing it in front changes
/// nothing on screen — and keeps it out of what that glass blurs, which would
/// otherwise darken its own edges.
fn surface_effect_elements<R, D>(
    out: &mut dyn FnMut(D::Element),
    renderer: &mut R,
    decorator: &mut D,
    surface: &WlSurface,
    fingerprint: u64,
    effects: blur::SurfaceEffects,
    alpha: f32,
    strength: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    shadow_elements(
        out,
        renderer,
        decorator,
        surface,
        fingerprint,
        &effects,
        alpha,
    );

    let (ids, commit) = blur::backdrop_slots(
        surface,
        effects.pieces.len(),
        fingerprint,
        effects.generation,
    );
    for (id, piece) in std::iter::zip(ids, &effects.pieces) {
        let backdrop = piece_backdrop(id, commit, piece, &effects, alpha, strength);
        if let Some(backdrop) = decorator.backdrop(renderer, backdrop) {
            out(backdrop);
        }
    }
}

/// The shadows a surface casts through `crownos_background_effects`.
fn shadow_elements<R, D>(
    out: &mut dyn FnMut(D::Element),
    renderer: &mut R,
    decorator: &mut D,
    surface: &WlSurface,
    fingerprint: u64,
    effects: &blur::SurfaceEffects,
    alpha: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    let (ids, commit) = blur::shadow_slots(
        surface,
        effects.shadows.len(),
        fingerprint,
        effects.generation,
    );
    for (id, &piece) in std::iter::zip(ids, &effects.shadows) {
        if let Some(shadow) = decorator.shadow(
            renderer,
            Shadow {
                id,
                commit,
                piece,
                alpha,
            },
        ) {
            out(shadow);
        }
    }
}

/// One piece of a surface's glass, as its backdrop draws it.
fn piece_backdrop(
    id: Id,
    commit: CommitCounter,
    piece: &blur::GlassPiece,
    effects: &blur::SurfaceEffects,
    alpha: f32,
    strength: f32,
) -> Backdrop {
    Backdrop {
        id,
        commit,
        geometry: piece.geometry,
        mask: piece.mask,
        radius: piece.radius,
        glass: effects.glass,
        alpha,
        strength: effects.strength * strength,
    }
}

/// The backdrop that lifts the glass a glazed surface is laid on: its one
/// piece, under the identity that piece's backdrop has always had. `None`
/// with no blur this frame, or nothing of the piece on screen.
fn glazed_backdrop<R, D>(
    decorator: &D,
    surface: &WlSurface,
    effects: &blur::SurfaceEffects,
    alpha: f32,
) -> Option<Backdrop>
where
    R: Renderer + ImportAll,
    D: TileDecorator<R>,
{
    let fingerprint = decorator.blur_fingerprint()?;
    let piece = effects.pieces.first()?;
    let (ids, commit) = blur::backdrop_slots(
        surface,
        effects.pieces.len(),
        fingerprint,
        effects.generation,
    );
    let id = ids.into_iter().next()?;
    Some(piece_backdrop(id, commit, piece, effects, alpha, 1.0))
}

pub(crate) fn layer_elements<R, D>(
    elements: &mut Elements<R, D>,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    layers: &[Layer],
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    // A second guard for the same output deadlocks, so keep the scope tight.
    let map = layer_map_for_output(monitor.output());

    for layer in layers {
        for surface in map.layers_on(*layer).rev() {
            let Some(geometry) = map.layer_geometry(surface) else {
                continue;
            };
            let location: Point<i32, Physical> = geometry.loc.to_physical_precise_round(scale);
            let clip = Rectangle::new(location, geometry.size.to_physical_precise_round(scale));
            let radius = blur::surface_corner_radius(surface.wl_surface(), scale.x);
            let popups = popup::layer_popup_elements(renderer, surface, location, scale);
            let tree = surface_tree::transformed_surface_elements(
                renderer,
                surface.wl_surface(),
                location,
                scale,
                1.0,
                Kind::Unspecified,
            );

            // A panel that speaks `composited_blur` painted its body square
            // for the compositor to cut: its own surface is laid on its glass
            // and cut with it, and the glass's backdrop only lifts.
            let shading = decorator.glass(scale.x, GlassKind::Panel).shading;
            let effects =
                blur::place_surface_effects(surface.wl_surface(), location, scale, clip, shading);
            let glazed = effects.as_ref().and_then(|effects| effects.glazed);
            let own = Id::from_wayland_resource(surface.wl_surface());
            let mut lift = None;

            for scaled in popups.into_iter().chain(tree) {
                // A panel that asked to be rounded is clipped to its own
                // radius. Nothing to round, nothing to crop: the surfaces go
                // into the frame as the client committed them, animations
                // applied, only sampled the way windows are.
                let (shape, corners) = match radius {
                    Some(radius) => (clip, [radius; 4]),
                    None => (scaled.geometry(scale), [0.0; 4]),
                };
                let Some(cropped) = CropRenderElement::from_element(scaled, scale, shape) else {
                    continue;
                };
                let glazing = match (glazed, &effects) {
                    (Some(outline), Some(effects)) if lift.is_none() && cropped.id() == &own => {
                        let backdrop =
                            glazed_backdrop(decorator, surface.wl_surface(), effects, 1.0);
                        let glaze = Glaze { outline, backdrop };
                        decorator.glaze(renderer, cropped, shape, corners, glaze)
                    }
                    _ => Glazing::Decorated(decorator.decorate(renderer, cropped, shape, corners)),
                };
                let tile = |element| CrownElement::Tile(Wrap::from(element));
                match glazing {
                    Glazing::Glazed {
                        surface,
                        lift: glass,
                    } => {
                        elements.push(tile(surface));
                        lift = Some(glass);
                    }
                    Glazing::Decorated(decorated) => elements.extend(decorated.map(tile)),
                }
            }

            let out = &mut |element| elements.push(CrownElement::Tile(Wrap::from(element)));
            match lift {
                // The shadows first and the glass's lift after, exactly where
                // the glass would have stood: the lift then sees the frame as
                // the glass always did, shadow-free.
                Some(lift) => {
                    if let (Some(effects), Some(fingerprint)) =
                        (&effects, decorator.blur_fingerprint())
                    {
                        let wl_surface = surface.wl_surface();
                        shadow_elements(
                            out,
                            renderer,
                            decorator,
                            wl_surface,
                            fingerprint,
                            effects,
                            1.0,
                        );
                    }
                    lift.into_iter().for_each(out);
                }
                // Panels and notifications are what actually wants glass, so
                // layer surfaces get the same treatment windows do.
                None => backdrop_elements(
                    out,
                    renderer,
                    decorator,
                    surface.wl_surface(),
                    location,
                    scale,
                    clip,
                    Outline {
                        rect: clip,
                        radius: radius.unwrap_or(0.0),
                    },
                    1.0,
                    decorator.glass(scale.x, GlassKind::Panel),
                    1.0,
                ),
            }
        }
    }
}
