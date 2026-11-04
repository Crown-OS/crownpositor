//! A decorated window's frame: its titlebar with the title and controls, the
//! hairline round it, and the glass they sit on.
//!
//! Laid out at the window's own size and drawn at whatever size the window is
//! drawn at, so the titlebar stays part of the window wherever it goes: whole
//! on the desktop, shrunk with it in the overview and the Alt+Tab stack.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            Kind, Wrap, memory::MemoryRenderBufferRenderElement, utils::RescaleRenderElement,
        },
        utils::CommitCounter,
    },
    utils::{Physical, Point, Rectangle, Scale, Size},
};

use crate::{
    rendering::{
        Elements, FrameStyle,
        blur::{Glass, GlassKind},
        decorate::{Backdrop, Outline, TileDecorator},
        decoration::{Border, FramePalette, TitleBarParams, window},
        element::CrownElement,
        painter::Backing,
        unstretched::Unstretched,
    },
    shell::{
        decoration::{Control, DecorationIds, Insets, TitleBarLayout},
        tile::Tile,
    },
};

/// One window's frame as placed this frame, output-local.
pub(crate) struct Frame<'a> {
    pub tile: &'a Tile,
    /// The window's own, or the second set a preview draws its copy with.
    pub ids: &'a DecorationIds,
    /// The whole window, titlebar included.
    pub rect: Rectangle<i32, Physical>,
    /// How much of the top the titlebar takes.
    pub inset: i32,
    pub radius: f32,
    /// The frame's size against the window's own.
    pub zoom: f64,
    pub backing: Backing,
    /// What a preview's card cuts it to.
    pub clip: Option<Rectangle<i32, Physical>>,
}

impl<'a> Frame<'a> {
    pub fn desktop(
        tile: &'a Tile,
        rect: Rectangle<i32, Physical>,
        inset: i32,
        radius: f32,
    ) -> Self {
        Self {
            tile,
            ids: tile.decoration_ids(),
            rect,
            inset,
            radius,
            zoom: 1.0,
            backing: Backing::Glass,
            clip: None,
        }
    }

    /// The title and the hairline, which go *over* the client. Neither can be
    /// cut, so a frame poking out of a preview's card goes without them.
    pub fn overlay<R, D>(
        &self,
        elements: &mut Elements<R, D>,
        renderer: &mut R,
        decorator: &mut D,
        style: &mut FrameStyle<'_>,
        alpha: f32,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
        D: TileDecorator<R>,
    {
        if self.clip.is_some_and(|clip| !clip.contains_rect(self.rect)) {
            return;
        }
        self.title::<R, D>(elements, renderer, style, alpha);

        let (focused, commit) = self.appearance(style);
        if let Some(border) = decorator.border(
            renderer,
            Border {
                id: self.ids.border.clone(),
                commit,
                window: self.rect,
                thickness: self.ring(style),
                radius: self.radius,
                color: style.palette(focused).border,
                alpha,
            },
        ) {
            elements.push(CrownElement::Tile(Wrap::from(border)));
        }
    }

    /// Rasterised once at the output's scale and shrunk with the frame, so a
    /// zoom costs no rasterising. Clipped rather than elided: a title too long
    /// for its window loses its tail.
    fn title<R, D>(
        &self,
        elements: &mut Elements<R, D>,
        renderer: &mut R,
        style: &mut FrameStyle<'_>,
        alpha: f32,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
        D: TileDecorator<R>,
    {
        let area = self.layout(style).label();
        let focused = style.focused == Some(self.tile.id());
        let color = style.palette(focused).label;
        let scale = style.scale;
        if area.size.w <= 0 {
            return;
        }
        let Some(label) = style.text.label(self.tile.title(), scale, color, true) else {
            return;
        };

        let natural = |value: i32| (f64::from(value) * scale) as i32;
        let bar = (f64::from(self.inset) / self.zoom).round() as i32;
        let visible = Size::from((
            label.size.w.min(natural(area.size.w)),
            label.size.h.min(bar),
        ));
        if visible.w <= 0 || visible.h <= 0 {
            return;
        }
        let location = Point::<i32, Physical>::from((
            self.rect.loc.x + natural(area.loc.x),
            self.rect.loc.y + (bar - visible.h) / 2,
        ));
        let crop = Rectangle::new(
            Point::from((0.0, f64::from((label.size.h - visible.h) / 2))),
            visible.to_f64(),
        );
        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location.to_f64(),
            &label.buffer,
            Some(alpha),
            Some(crop),
            Some(visible),
            Kind::Unspecified,
        ) {
            let exact = Unstretched::at_own_size(element, Scale::from(scale));
            elements
                .push(RescaleRenderElement::from_element(exact, self.rect.loc, self.zoom).into());
        }
    }

    /// The tinted glass and the titlebar's controls, which go *behind* the
    /// client so that its middle covers theirs.
    ///
    /// With no glass the panel carries the tint over the whole window, so the
    /// titlebar and the notches the client's corners cut away still read as
    /// one material.
    pub fn backing<R, D>(
        &self,
        elements: &mut Elements<R, D>,
        renderer: &mut R,
        decorator: &mut D,
        style: &FrameStyle<'_>,
        alpha: f32,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Clone + 'static,
        D: TileDecorator<R>,
    {
        let (focused, commit) = self.appearance(style);
        let palette = style.palette(focused);
        let palette = FramePalette {
            control_glyph: palette
                .control_glyph
                .map(|channel| channel * self.tile.anim().glyphs()),
            ..palette
        };
        let glass = match self.backing {
            Backing::Glass => decorator.blur_fingerprint().and_then(|_| {
                let sheet = self.sheet(style);
                let ring = (self.rect.loc.y - sheet.rect.loc.y) as f32;
                let glass = decorator.glass(style.scale * self.zoom, GlassKind::Window);
                decorator.backdrop(
                    renderer,
                    Backdrop {
                        id: self.ids.glass.clone(),
                        commit,
                        geometry: sheet.rect,
                        mask: sheet.rect,
                        radius: sheet.radius,
                        glass: Glass {
                            tint: palette.sheet,
                            tint_inset: ring,
                            ..glass
                        },
                        alpha,
                        strength: decorator.blur_strength_for(self.zoom),
                    },
                )
            }),
            Backing::Bare => None,
        };

        let (geometry, palette) = match glass {
            Some(_) => (
                Rectangle::new(self.rect.loc, Size::from((self.rect.size.w, self.inset))),
                palette.over_glass(),
            ),
            None => (self.rect, palette),
        };
        let geometry = self
            .clip
            .map_or(Some(geometry), |clip| geometry.intersection(clip));

        let layout = self.layout(style);
        let physical = |value: i32| (f64::from(value) * style.scale * self.zoom) as f32;
        let first = layout.first_control_centre();
        let panel = geometry.and_then(|geometry| {
            decorator.title_bar(
                renderer,
                TitleBarParams {
                    id: self.ids.panel.clone(),
                    commit,
                    geometry,
                    frame: self.rect,
                    radius: self.radius,
                    bar_height: self.inset as f32,
                    first_control: (
                        self.rect.loc.x as f32 + physical(first.x),
                        self.rect.loc.y as f32 + physical(first.y),
                    ),
                    control_pitch: physical(layout.control_pitch()),
                    control_radius: physical(layout.control_radius()),
                    hovered: style.hover_on(self.tile.id()),
                    palette,
                    alpha,
                },
            )
        });

        elements.extend(
            panel
                .into_iter()
                .chain(glass)
                .map(|element| CrownElement::Tile(Wrap::from(element))),
        );
    }

    /// The whole frame as one shape, out to the ring's outer edge where its
    /// glass stops: the same arithmetic the ring is drawn with, because a
    /// sheet that disagreed with it by a pixel would show as a seam.
    pub fn sheet(&self, style: &FrameStyle<'_>) -> Outline {
        window::outline(self.rect, self.ring(style), self.radius).map_or(
            Outline {
                rect: self.rect,
                radius: self.radius,
            },
            |(rect, _, radius)| Outline { rect, radius },
        )
    }

    /// One logical pixel, and never thinner than one physical one.
    fn ring(&self, style: &FrameStyle<'_>) -> f32 {
        (style.scale * self.zoom).max(1.0) as f32
    }

    /// Against the frame's *animated* size, scaled back to the window's own:
    /// controls placed from the settled width would drift outside a window
    /// that is mid-resize. Measured at the origin, so every rect is an offset
    /// into the frame.
    fn layout(&self, style: &FrameStyle<'_>) -> TitleBarLayout {
        let scale = style.scale * self.zoom;
        let logical = |value: i32| (f64::from(value) / scale).round() as i32;
        TitleBarLayout::new(
            Rectangle::from_size(Size::from((
                logical(self.rect.size.w),
                logical(self.rect.size.h),
            ))),
            Insets::top(logical(self.inset)),
        )
    }

    fn appearance(&self, style: &FrameStyle<'_>) -> (bool, CommitCounter) {
        let id = self.tile.id();
        let focused = style.focused == Some(id);
        let glyphs = (self.tile.anim().glyphs() * f32::from(u8::MAX)).round() as u8;
        let key = appearance_key(focused, style.hover_on(id), style.dark, glyphs);
        (focused, self.tile.decoration_commit(key))
    }
}

/// Where the client draws inside a frame: below its titlebar, `rim` in from
/// its other three sides. Never empty, so a frame squeezed smaller than its
/// own decoration still hands the renderer a pixel to clip to.
pub(crate) fn client_area(
    frame: Rectangle<i32, Physical>,
    inset: i32,
    rim: i32,
) -> Rectangle<i32, Physical> {
    Rectangle::new(
        frame.loc + Point::from((rim, inset)),
        Size::from((
            (frame.size.w - 2 * rim).max(1),
            (frame.size.h - inset - rim).max(1),
        )),
    )
}

/// What a frame's *pixels* depend on, packed so the commit counter can tell in
/// one comparison whether they changed. `glyphs` is how much the controls'
/// glyphs show, in 256ths, so their fade repaints the panel on every step.
fn appearance_key(focused: bool, hovered: Option<Control>, dark: bool, glyphs: u8) -> u64 {
    let hover = hovered.map_or(u64::from(u8::MAX), |control| control.index() as u64);
    (u64::from(glyphs) << 24) | (u64::from(focused) << 16) | (u64::from(dark) << 8) | hover
}
