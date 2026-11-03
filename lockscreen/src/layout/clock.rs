//! The clock, one sprite per character: so a digit can change on its own,
//! the hours and minutes can move apart, and every glyph can stand taller
//! than its font draws it.

use crownos_config::schema::{
    ClockArrangement, ClockFont, ClockFrame, ClockLighting, ClockLook, ClockStyle,
};
use taffy::{
    AlignItems, Display, FlexDirection, Line, Size, Style,
    prelude::{auto, length, line},
};

use super::{
    ClockGroup, Content, Context, FontFamily, GlyphPaint, GlyphRun, Part, StrokedFont,
    block::Block, node::Inherited,
};
use crate::{clock::GLASS_TINT, palette::Rgba};

/// Of the size: the space between stacked hours and minutes.
const STACK_GAP: f32 = 0.08;
/// The darkest the shadow gets, at full strength.
const SHADOW_ALPHA: f32 = 0.55;
/// How strongly the halo of glowing digits shows.
const GLOW_ALPHA: f32 = 0.75;
/// How much of a sheened digit's own colour shows away from the light, and
/// how far its light runs toward white.
const SHEEN_SHADE: f32 = 0.55;
const SHEEN_WHITENESS: f32 = 0.7;
const SEPARATOR: char = ':';

/// Size and weight of each clock style.
fn metrics(style: ClockStyle) -> (f32, u16) {
    match style {
        ClockStyle::Classic => (96.0, 300),
        ClockStyle::Large => (152.0, 600),
        ClockStyle::Minimal => (60.0, 400),
    }
}

pub(super) fn block(cx: &Context<'_>, inherited: &Inherited) -> Block {
    let config = &cx.scene.config;
    let look = &config.clock;
    let frame = &look.frame;
    let (size, weight) = metrics(config.clock_style);
    let cells = Cells {
        template: GlyphRun {
            ch: SEPARATOR,
            size: size
                * cx.text_scale
                * frame
                    .scale
                    .clamp(ClockFrame::MIN_SCALE, ClockFrame::MAX_SCALE),
            weight: look
                .weight
                .unwrap_or(weight)
                .clamp(ClockLook::MIN_WEIGHT, ClockLook::MAX_WEIGHT),
            family: family(cx, look.font),
            stretch: look
                .stretch
                .clamp(ClockLook::MIN_STRETCH, ClockLook::MAX_STRETCH),
            paint: face(cx, inherited, look),
        },
        shadow: look.shadow.clamp(0.0, 1.0) * SHADOW_ALPHA,
        glow: (look.lighting == ClockLighting::Glow).then(|| {
            let [r, g, b, a] = inherited.tint(cx.palette.color("text.primary"));
            [r, g, b, a * GLOW_ALPHA]
        }),
    };

    let (hours, minutes) = cx.live.face.groups();
    let mut groups = vec![cells.group(hours, ClockGroup::Hours, frame.hours)];
    let stacked = look.arrangement == ClockArrangement::Stacked;
    if !minutes.is_empty() {
        if !stacked {
            groups.push(cells.group(":", ClockGroup::Separator, [0.0; 2]));
        }
        groups.push(cells.group(minutes, ClockGroup::Minutes, frame.minutes));
    }
    let (direction, gap) = match look.arrangement {
        ClockArrangement::Row => (FlexDirection::Row, 0.0),
        ClockArrangement::Stacked => (FlexDirection::Column, cells.template.size * STACK_GAP),
    };
    let align = if inherited.centered {
        AlignItems::CENTER
    } else {
        AlignItems::START
    };
    let mut clock = flex(direction, gap, align).with_children(groups);
    clock.shift = frame.offset;
    clock
}

fn family(cx: &Context<'_>, font: ClockFont) -> FontFamily {
    match font {
        ClockFont::Theme => FontFamily::Sans(cx.family.to_owned()),
        ClockFont::Tall => FontFamily::Stroked(StrokedFont::Tall),
        ClockFont::Narrow => FontFamily::Stroked(StrokedFont::Narrow),
        ClockFont::Serif => FontFamily::Stroked(StrokedFont::Serif),
    }
}

/// Glass digits preview as a white wash, clearer at the top; the host that
/// can blur the background through them draws the real thing.
fn face(cx: &Context<'_>, inherited: &Inherited, look: &ClockLook) -> GlyphPaint {
    let color = inherited.tint(cx.palette.color("text.primary"));
    if !look.glass {
        return match look.lighting {
            ClockLighting::Sheen => sheen(color),
            ClockLighting::None | ClockLighting::Glow => GlyphPaint::Face {
                top: color,
                bottom: color,
            },
        };
    }
    let [r, g, b, a] = color;
    let wash = |amount: f32| [r, g, b, a * amount];
    GlyphPaint::Face {
        top: wash(GLASS_TINT[0]),
        bottom: wash(GLASS_TINT[1]),
    }
}

fn sheen([r, g, b, a]: Rgba) -> GlyphPaint {
    let toward_white = |channel: f32| channel + (1.0 - channel) * SHEEN_WHITENESS;
    GlyphPaint::Sheen {
        base: [r, g, b, a * SHEEN_SHADE],
        light: [toward_white(r), toward_white(g), toward_white(b), a],
    }
}

struct Cells {
    template: GlyphRun,
    shadow: f32,
    glow: Option<Rgba>,
}

impl Cells {
    fn group(&self, text: &str, group: ClockGroup, shift: [f32; 2]) -> Block {
        let mut row = flex(FlexDirection::Row, 0.0, AlignItems::END)
            .with_children(text.chars().map(|ch| self.cell(ch, group)).collect());
        row.shift = shift;
        row
    }

    /// A character's face over its glow and shadow, all sharing one box.
    fn cell(&self, ch: char, group: ClockGroup) -> Block {
        let glyph = |paint| {
            let mut block = Block::new(Style {
                grid_row: Line {
                    start: line(1),
                    end: auto(),
                },
                grid_column: Line {
                    start: line(1),
                    end: auto(),
                },
                ..Style::default()
            })
            .with_content(Content::Glyph(GlyphRun {
                ch,
                paint,
                ..self.template.clone()
            }));
            block.part = Some(Part::ClockGlyph(group));
            block
        };
        let inked = !ch.is_whitespace();
        let shadow =
            (self.shadow > 0.0 && inked).then(|| glyph(GlyphPaint::Shadow { alpha: self.shadow }));
        let glow = self
            .glow
            .filter(|_| inked)
            .map(|color| glyph(GlyphPaint::Glow { color }));
        Block::new(Style {
            display: Display::Grid,
            ..Style::default()
        })
        .with_children(
            shadow
                .into_iter()
                .chain(glow)
                .chain([glyph(self.template.paint)])
                .collect(),
        )
    }
}

fn flex(direction: FlexDirection, gap: f32, align: AlignItems) -> Block {
    Block::new(Style {
        display: Display::Flex,
        flex_direction: direction,
        align_items: Some(align),
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        ..Style::default()
    })
}
