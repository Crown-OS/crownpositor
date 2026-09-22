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
pub mod cursor;
pub mod decorate;
pub mod decoration;
pub mod element;
pub mod overview;
pub mod rounded;

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, Kind, Wrap,
            memory::MemoryRenderBufferRenderElement,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
        utils::CommitCounter,
    },
    desktop::layer_map_for_output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size},
    wayland::shell::wlr_layer::Layer,
};

use config::Appearance;

use crate::{
    rendering::{
        cursor::Cursor,
        decorate::{Backdrop, Shadow, TileDecorator},
        decoration::{Border, FramePalette, TextRenderer, TitleBarParams, window},
        element::CrownElement,
    },
    shell::{
        Shell,
        decoration::{Control, TitleBarLayout},
        monitor::Monitor,
        tile::Tile,
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
            radius: appearance.border_radius as f32,
            scale,
            origin,
            dark: appearance.dark_mode,
            focused_palette: FramePalette::new(appearance, true),
            unfocused_palette: FramePalette::new(appearance, false),
            text,
            focused,
            hovered,
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

/// A rasterised buffer's own size, retyped.
///
/// The label buffers are imported at scale 1, so their pixel size and the
/// logical size the element wants are the same number in two coordinate spaces.
pub(crate) fn logical(size: Size<i32, BufferCoords>) -> Size<i32, Logical> {
    Size::from((size.w, size.h))
}

/// What a frame's *pixels* depend on, packed so the commit counter can tell in
/// one comparison whether they changed.
fn appearance_key(focused: bool, hovered: Option<Control>, dark: bool) -> u64 {
    let hover = hovered.map_or(u64::from(u8::MAX), |control| control.index() as u64);
    (u64::from(focused) << 16) | (u64::from(dark) << 8) | hover
}

/// One output's scene graph, for a given renderer and decorator.
pub type Elements<R, D> = Vec<CrownElement<R, <D as TileDecorator<R>>::Element>>;

/// Front-to-back, the order `OutputDamageTracker::render_output` wants.
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
        monitor.geometry().loc,
        pointer,
        scale,
    );

    layer_elements(
        &mut elements,
        monitor,
        renderer,
        decorator,
        scale,
        &[Layer::Overlay, Layer::Top],
    );

    // An open menu is over everything on its output, panels included from the
    // user's point of view — it was opened deliberately and dismisses itself.
    menu_elements(
        &mut elements,
        shell,
        monitor,
        renderer,
        decorator,
        scale,
        style,
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
        overview::overview_elements(
            &mut elements,
            monitor,
            renderer,
            decorator,
            scale,
            style.radius,
        );
        overview::label_elements::<R, D>(&mut elements, monitor, renderer, scale, style);
        overview::background_elements(&mut elements, monitor, renderer, decorator, scale);
        return elements;
    }

    // One workspace once the viewport has settled, two while it is sliding —
    // and the slide is nothing but the offset each tile is drawn at, so the GPU
    // recomposites textures it already holds instead of anyone touching pixels.
    // Layer surfaces sit outside the loop: a bar does not travel with the
    // workspace under it.
    let mut covered = false;
    for (workspace, offset) in monitor.visible_workspaces() {
        match workspace.fullscreen().and_then(|id| workspace.tile(id)) {
            // A fullscreen window covers its page edge to edge, so rounding it
            // would just cut four notches out of the display — and while it is
            // the only page on screen there is nothing behind it to draw.
            Some(tile) => {
                covered |= offset.x == 0.0;
                // Square corners and no frame: a fullscreen window covers its
                // page edge to edge, so rounding it would cut four notches out
                // of the display.

                tile_elements(
                    &mut elements,
                    shell,
                    tile,
                    renderer,
                    decorator,
                    scale,
                    offset,
                    0.0,
                    style,
                );
            }
            None => {
                for tile in workspace.stacking_order() {
                    tile_elements(
                        &mut elements,
                        shell,
                        tile,
                        renderer,
                        decorator,
                        scale,
                        offset,
                        style.radius,
                        style,
                    );
                }
            }
        }
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

    let _ = shell;
    elements
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
    // The interpolated rect, not the target: this is what makes a window slide.
    // Output-local, because the damage tracker works in this output's own
    // space. `offset` slides the whole workspace the tile belongs to; rounding
    // only at the physical step keeps both motions sub-pixel smooth.
    let animated = tile.render_rect();
    let frame_loc: Point<i32, Physical> = (animated.loc + offset).to_physical_precise_round(scale);
    let frame = Rectangle::new(frame_loc, animated.size.to_physical_precise_round(scale));

    let inset: i32 = (tile.insets().top as f64 * scale.y).round() as i32;
    // Where the client actually draws. Equal to the frame when undecorated, so
    // the undecorated path costs nothing extra.
    let clip = Rectangle::new(
        frame_loc + Point::from((0, inset)),
        Size::from((frame.size.w, (frame.size.h - inset).max(1))),
    );
    let size = (clip.size.w as f32, clip.size.h as f32);
    let alpha = tile.render_alpha();

    if inset > 0 {
        frame_overlay(
            elements, shell, tile, renderer, decorator, frame, inset, radius, scale, style, alpha,
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

    // Square where the titlebar covers them, so the two do not each round the
    // same corner and leave a notch between them.
    let radii = match client_radius {
        Some(radius) => [radius; 4],
        None => corner_radii(radius, inset > 0),
    };

    // `Window::render_elements` walks the surface tree and its popups, so popups
    // need no separate pass.
    let surfaces: Vec<WaylandSurfaceRenderElement<R>> = tile
        .window()
        .render_elements(renderer, clip.loc, scale, alpha);

    for surface in surfaces {
        // A client's buffer is whatever size it last committed — during a shrink
        // still the *old* size — so without the clip it bleeds over its
        // neighbour. `from_element` returns `None` when the element falls
        // entirely outside, which is exactly what should not be drawn.
        // if shell.is_animating() {
        //     let Some(surface) =  else {
        //         continue;
        //     }
        // }
        // Drawn at its own size here; the overview passes a smaller factor
        // through the very same wrapper.
        let scaled = RescaleRenderElement::from_element(surface, clip.loc, 1.0);
        let Some(cropped) = CropRenderElement::from_element(scaled, scale, clip) else {
            continue;
        };
        if let Some(decorated) = decorator.decorate(renderer, cropped, size, radii) {
            elements.push(CrownElement::Tile(Wrap::from(decorated)));
        }
    }

    // The blurred glass goes in *after* the window's surfaces — later in the
    // list is further from the eye, so it sits directly behind them. `clip.loc`
    // is where the surface's own origin lands, which is the space the client
    // expressed its blur region in; `clip` is both the mask the corners are cut
    // from and the bound the region is clipped to.
    if let Some(surface) = surface {
        backdrop_elements(
            &mut |element| elements.push(CrownElement::Tile(Wrap::from(element))),
            renderer,
            decorator,
            &surface,
            clip.loc,
            scale,
            clip,
            client_radius.unwrap_or(radius),
            alpha,
        );
    }

    // Last, so it is behind the client as well as behind the frame's own
    // overlay: the frame's material has to show through every part of the
    // window that is not the client — the titlebar, the corner notches the
    // client's rounding cuts away, and the ring.
    if inset > 0 {
        frame_backing(
            elements, tile, renderer, decorator, frame, inset, radius, style, alpha,
        );
    }
}

/// What a decorated window draws *over* itself: its title, its menu row and
/// its outline.
///
/// The material all three sit on is [`frame_backing`]'s, pushed behind the
/// client — so these are only the marks laid on top of it.
#[allow(clippy::too_many_arguments)]
fn frame_overlay<R, D>(
    elements: &mut Elements<R, D>,
    shell: &Shell,
    tile: &Tile,
    renderer: &mut R,
    decorator: &mut D,
    frame: Rectangle<i32, Physical>,
    inset: i32,
    radius: f32,
    scale: Scale<f64>,
    style: &mut FrameStyle<'_>,
    alpha: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let focused = style.focused == Some(tile.id());
    let palette = style.palette(focused);
    let commit = tile.decoration_commit(appearance_key(
        focused,
        style.hover_on(tile.id()),
        style.dark,
    ));

    // Copied out, so the closure below does not hold a borrow on `style` —
    // which the menu row needs mutably to shape its labels.
    let output_scale = style.scale;
    let physical = |value: i32| (value as f64 * output_scale) as f32;
    let layout = frame_layout(tile, frame, output_scale);

    // The title. Rasterised at the output's scale and cached, so this is a hash
    // lookup on every frame but the one where the title changed.
    let label_area = layout.label();
    if label_area.size.w > 0
        && let Some(label) = style
            .text
            .label(tile.title(), style.scale, palette.label, true)
    {
        // Clipped rather than elided: a title too long for its window loses its
        // tail, which is what every titlebar has always done.
        let visible = Size::from((
            label.size.w.min(physical(label_area.size.w) as i32),
            label.size.h,
        ));
        let location = Point::<i32, Physical>::from((
            frame.loc.x + physical(label_area.loc.x) as i32,
            frame.loc.y + (inset - visible.h) / 2,
        ));

        // The buffer was rasterised at this output's scale and imported at
        // scale 1, so its logical size and its pixel size are the same number.
        if visible.w > 0
            && let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                location.to_f64(),
                &label.buffer,
                Some(alpha),
                Some(Rectangle::from_size(visible.to_f64())),
                Some(visible),
                Kind::Unspecified,
            )
        {
            elements.push(CrownElement::Memory(element));
        }
    }

    menu_row_elements::<R, D>(elements, shell, tile.id(), renderer, scale, style, alpha);

    // The hairline. Grown outward from the window, so it eats into the gap
    // rather than into the client area, and translucent — the glass behind it
    // is what it is lit by.
    if let Some(border) = decorator.border(
        renderer,
        Border {
            id: tile.decoration_ids().border.clone(),
            commit,
            window: frame,
            thickness: style.scale as f32,
            radius,
            color: palette.border,
            alpha,
        },
    ) {
        elements.push(CrownElement::Tile(Wrap::from(border)));
    }
}

/// Where a frame's pieces sit, laid out against its *animated* size.
///
/// The window is mid-resize on every frame of a transition, and controls
/// positioned from the settled width would drift outside it while it grows.
/// Measured at the origin, so every rect that comes back is already an offset
/// into the frame.
fn frame_layout(tile: &Tile, frame: Rectangle<i32, Physical>, scale: f64) -> TitleBarLayout {
    let logical = |value: i32| (value as f64 / scale).round() as i32;
    TitleBarLayout::new(
        Rectangle::from_size(Size::from((logical(frame.size.w), logical(frame.size.h)))),
        tile.insets(),
    )
}

/// What a decorated window sits on: a sheet of tint over a sheet of blurred
/// glass, both behind the client.
///
/// Two rects rather than one, because they reach different distances. The tint
/// covers the window and is masked by its rounded rect, so it fills the
/// titlebar *and* the notches the client's rounded corners cut away — those
/// were bare desktop before, and a titlebar whose material stops at its own
/// bottom edge reads as two materials. The glass reaches one step further, to
/// the border's outer edge, because the ring is translucent and a sheet that
/// stopped at the window would leave it the one unlit part of the frame.
///
/// Both are furthest from the eye of everything a window draws, which is what
/// lets the client cover their middles: one quad each behind the window, rather
/// than a strip per uncovered edge, which costs more draw calls than the
/// covered pixels save.
#[allow(clippy::too_many_arguments)]
fn frame_backing<R, D>(
    elements: &mut Elements<R, D>,
    tile: &Tile,
    renderer: &mut R,
    decorator: &mut D,
    frame: Rectangle<i32, Physical>,
    inset: i32,
    radius: f32,
    style: &FrameStyle<'_>,
    alpha: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    let focused = style.focused == Some(tile.id());
    let hovered = style.hover_on(tile.id());
    let ids = tile.decoration_ids();
    let commit = tile.decoration_commit(appearance_key(focused, hovered, style.dark));

    let layout = frame_layout(tile, frame, style.scale);
    let physical = |value: i32| (value as f64 * style.scale) as f32;
    let first = layout.first_control_centre();

    if let Some(panel) = decorator.title_bar(
        renderer,
        TitleBarParams {
            id: ids.panel.clone(),
            commit,
            geometry: frame,
            frame,
            radius,
            bar_height: inset as f32,
            first_control: (
                frame.loc.x as f32 + physical(first.x),
                frame.loc.y as f32 + physical(first.y),
            ),
            control_pitch: physical(layout.control_pitch()),
            control_radius: physical(layout.control_radius()),
            hovered,
            palette: style.palette(focused),
            alpha,
        },
    ) {
        elements.push(CrownElement::Tile(Wrap::from(panel)));
    }

    if decorator.blur_fingerprint().is_none() {
        return;
    }

    // The same growth the ring is drawn with, from the same arithmetic: a sheet
    // that disagreed with it by a pixel would show as a seam around the window.
    let (sheet, _, outer) = window::outline(frame, style.scale as f32, radius)
        // No ring to reach past, so the window's own rect is the whole of it.
        .unwrap_or((frame, 0.0, radius));

    if let Some(glass) = decorator.backdrop(
        renderer,
        Backdrop {
            id: ids.glass.clone(),
            commit,
            geometry: sheet,
            mask: sheet,
            radius: outer,
            glass: decorator.glass(style.scale),
            alpha,
            strength: 1.0,
        },
    ) {
        elements.push(CrownElement::Tile(Wrap::from(glass)));
    }
}

/// The menu row's labels along a titlebar.
///
/// Pushed by [`frame_elements`] for the window that owns them, from the layout
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
            Some(Rectangle::from_size(logical(label.size).to_f64())),
            Some(logical(label.size)),
            Kind::Unspecified,
        ) {
            elements.push(CrownElement::Memory(element));
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
            Some(Rectangle::from_size(logical(label.size).to_f64())),
            Some(logical(label.size)),
            Kind::Unspecified,
        ) {
            elements.push(CrownElement::Memory(element));
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
                glass: decorator.glass(style.scale),
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
                glass: decorator.glass(style.scale),
                alpha: alpha * 0.85,
                strength: 1.0,
            },
        )
    {
        elements.push(CrownElement::Tile(Wrap::from(glass)));
    }
}

/// Which corners of the client area round.
///
/// A decorated window's top pair are square: the titlebar sits over them and
/// rounds them itself, and two shapes each rounding the same corner leaves a
/// notch between them. The order is the one the shader reads: right-bottom,
/// right-top, left-bottom, left-top.
fn corner_radii(radius: f32, decorated: bool) -> [f32; 4] {
    let top = if decorated { 0.0 } else { radius };
    [radius, top, radius, top]
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
/// `origin` is where the surface's own `(0, 0)` lands, and `mask` is the
/// rectangle effects are clipped to and, for the portable protocol, the one the
/// corners are cut from — a window's animated rect, or a layer surface's
/// geometry.
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
    mask: Rectangle<i32, Physical>,
    radius: f32,
    alpha: f32,
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

    if let Some(effects) = blur::place_surface_effects(surface, origin, scale, mask) {
        surface_effect_elements(
            out,
            renderer,
            decorator,
            surface,
            fingerprint,
            effects,
            alpha,
        );
        return;
    }

    let mut rects = Vec::new();
    let Some(generation) = blur::place_blur_region(surface, origin, scale, mask, &mut rects) else {
        return;
    };

    let glass = decorator.glass(scale.x);
    let (ids, commit) = blur::backdrop_slots(surface, rects.len(), fingerprint, generation);
    for (id, geometry) in std::iter::zip(ids, rects) {
        if let Some(backdrop) = decorator.backdrop(
            renderer,
            Backdrop {
                id,
                commit,
                geometry,
                mask,
                radius,
                glass,
                alpha,
                strength: 1.0,
            },
        ) {
            out(backdrop);
        }
    }
}

/// The `crownos_background_effects` half of [`backdrop_elements`].
///
/// Glass first and shadows after: later in the list is further from the eye, so
/// this is the order that puts the silhouette underneath the material it is
/// cast by.
fn surface_effect_elements<R, D>(
    out: &mut dyn FnMut(D::Element),
    renderer: &mut R,
    decorator: &mut D,
    surface: &WlSurface,
    fingerprint: u64,
    effects: blur::SurfaceEffects,
    alpha: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    let (ids, commit) = blur::backdrop_slots(
        surface,
        effects.pieces.len(),
        fingerprint,
        effects.generation,
    );
    for (id, piece) in std::iter::zip(ids, &effects.pieces) {
        if let Some(backdrop) = decorator.backdrop(
            renderer,
            Backdrop {
                id,
                commit,
                geometry: piece.geometry,
                mask: piece.mask,
                radius: piece.radius,
                glass: effects.glass,
                alpha,
                strength: 1.0,
            },
        ) {
            out(backdrop);
        }
    }

    let (ids, commit) = blur::shadow_slots(
        surface,
        effects.shadows.len(),
        fingerprint,
        effects.generation,
    );
    for (id, piece) in std::iter::zip(ids, effects.shadows) {
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

fn layer_elements<R, D>(
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
            let layers: Vec<WaylandSurfaceRenderElement<R>> =
                surface.render_elements(renderer, location, scale, 1.0);

            match radius {
                // A panel that asked to be rounded is clipped to its own
                // radius, which costs it the decorator's wrapper per element.
                Some(radius) => {
                    let size = (clip.size.w as f32, clip.size.h as f32);
                    for layer in layers {
                        let scaled = RescaleRenderElement::from_element(layer, clip.loc, 1.0);
                        let Some(cropped) = CropRenderElement::from_element(scaled, scale, clip)
                        else {
                            continue;
                        };
                        if let Some(decorated) =
                            decorator.decorate(renderer, cropped, size, [radius; 4])
                        {
                            elements.push(CrownElement::Tile(Wrap::from(decorated)));
                        }
                    }
                }
                // Nothing to round, so nothing to wrap: the surfaces go into
                // the frame exactly as the client committed them.
                None => elements.extend(layers.into_iter().map(CrownElement::Surface)),
            }

            // Panels and notifications are what actually wants glass, so layer
            // surfaces get the same treatment windows do.
            backdrop_elements(
                &mut |element| elements.push(CrownElement::Tile(Wrap::from(element))),
                renderer,
                decorator,
                surface.wl_surface(),
                location,
                scale,
                clip,
                radius.unwrap_or(0.0),
                1.0,
            );
        }
    }
}
