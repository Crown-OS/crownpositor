//! The clock, one sprite per character, so a digit can change on its own,
//! the hours and minutes can move apart, and every glyph can stand taller
//! than its font draws it; with its date, the three laid out as one group.

use crownos_config::schema::{ClockFont, ClockFrame, ClockLayout, ClockLook};
use taffy::{
    AlignItems, Display, FlexDirection, Line, Size, Style,
    prelude::{auto, length, line},
};

use super::{
    ClockGroup, Content, Context, GlyphPaint, GlyphRun, Part, StrokedFont, block::Block,
    node::Inherited, stroked,
};

/// Of the output's height: the size a centred clock's digits are drawn at,
/// before the frame scales them, so that at the default stretch they stand
/// 0.37 of it tall, as ColorOS's stand 0.363 of a phone screen's height.
const DIGIT_SIZE: f32 = 0.147;
/// Of the centred size: how wide the Left and Stacked layouts draw their
/// digits, and how tall each line of them stands, as ColorOS's cards do.
const LEFT_WIDTH: f32 = 0.78;
const LEFT_HEIGHT: f32 = 1.0;
const STACKED_WIDTH: f32 = 0.87;
const STACKED_HEIGHT: f32 = 0.97;
/// Of the output's height: the date's text size, before the frame scales
/// it, so its capitals stand 0.0163 of it tall, as on the phone.
const DATE_SIZE: f32 = 0.0233;
const DATE_WEIGHT: u16 = 500;
/// The date's colour: the palette's body text, about 85% of its primary
/// white, as ColorOS's date is white at 85% opacity. A colour rather than
/// an opacity, because text is drawn from its glyphs' coverage alone and a
/// colour's alpha never reaches it.
const DATE_COLOR: &str = "text.body";
/// Of the digits' height: the room between the date and the digits it
/// stands above, the digits it stands below, and the stacked digits it
/// stands beside.
const DATE_GAP_ABOVE: f32 = 0.045;
const DATE_GAP_BELOW: f32 = 0.08;
const DATE_GAP_BESIDE: f32 = 0.27;
/// Of one line's height: the room between stacked hours and minutes.
const STACK_GAP: f32 = 0.04;
/// The darkest the shadow gets, at full strength: under the glyph's base,
/// before its blur thins it to about a tenth where it shows past the stem.
const SHADOW_ALPHA: f32 = 0.3;
const SEPARATOR: &str = ":";

pub(super) fn block(cx: &Context<'_>, inherited: &Inherited) -> Block {
    let look = &cx.scene.config.clock;
    let frame = &look.frame;
    let scale = frame
        .scale
        .clamp(ClockFrame::MIN_SCALE, ClockFrame::MAX_SCALE);
    let output_height = cx.output[1];
    let [width, height] = proportions(look.layout);
    let cells = Cells::of(look, output_height * DIGIT_SIZE * scale, width, height);
    let line = stroked::height(cells.template.size, cells.template.stretch);
    let date = date(
        cx,
        inherited,
        look.layout,
        output_height * DATE_SIZE * scale,
    );

    let (hours, minutes) = cx.live.face.groups();
    let hours = cells.group(hours, ClockGroup::Hours, frame.hours);
    let minutes =
        (!minutes.is_empty()).then(|| cells.group(minutes, ClockGroup::Minutes, frame.minutes));
    let mut clock = match look.layout {
        ClockLayout::Centered => flex(
            FlexDirection::Column,
            line * DATE_GAP_ABOVE,
            AlignItems::CENTER,
        )
        .with_children(
            date.into_iter()
                .chain([cells.row(hours, minutes)])
                .collect(),
        ),
        ClockLayout::Left => flex(
            FlexDirection::Column,
            line * DATE_GAP_BELOW,
            AlignItems::START,
        )
        .with_children(
            [cells.row(hours, minutes)]
                .into_iter()
                .chain(date)
                .collect(),
        ),
        ClockLayout::Stacked => {
            let digits = flex(FlexDirection::Column, line * STACK_GAP, AlignItems::START)
                .with_children([hours].into_iter().chain(minutes).collect());
            flex(
                FlexDirection::Row,
                line * DATE_GAP_BESIDE,
                AlignItems::START,
            )
            .with_children([digits].into_iter().chain(date).collect())
        }
    };
    clock.shift = frame.offset;
    clock
}

/// Of the centred clock's size: how wide `layout` draws its digits, and how
/// tall each line of them stands.
fn proportions(layout: ClockLayout) -> [f32; 2] {
    match layout {
        ClockLayout::Centered => [1.0, 1.0],
        ClockLayout::Left => [LEFT_WIDTH, LEFT_HEIGHT],
        ClockLayout::Stacked => [STACKED_WIDTH, STACKED_HEIGHT],
    }
}

fn font(font: ClockFont) -> StrokedFont {
    match font {
        ClockFont::Sans => StrokedFont::Sans,
        ClockFont::Grotesk => StrokedFont::Grotesk,
        ClockFont::Serif => StrokedFont::Serif,
    }
}

/// The date in the theme's font, `size` logical pixels, aligned as the
/// layout has it; `None` when the date reads nothing.
fn date(cx: &Context<'_>, inherited: &Inherited, layout: ClockLayout, size: f32) -> Option<Block> {
    let date = &cx.live.face.date;
    if date.is_empty() {
        return None;
    }
    let text = Inherited {
        size,
        weight: DATE_WEIGHT,
        color: cx.palette.color(DATE_COLOR),
        mono: false,
        centered: layout == ClockLayout::Centered,
        single_line: true,
        ..inherited.clone()
    };
    let mut block = Block::new(Style::default()).with_content(Content::Text(text.run(cx, date)));
    block.part = Some(Part::ClockDate);
    Some(block)
}

struct Cells {
    template: GlyphRun,
    shadow: f32,
    /// Whether hours and minutes side by side have the separator between.
    colon: bool,
}

impl Cells {
    /// Cells for digits `width` times as wide as a centred clock's `size`,
    /// standing `height` times as tall, their stems as wide as its.
    fn of(look: &ClockLook, size: f32, width: f32, height: f32) -> Self {
        Self {
            template: GlyphRun {
                ch: ' ',
                size: size * width,
                weight: stroked::weight_keeping_stroke(look.clamped_weight(), size, size * width),
                font: font(look.font),
                stretch: look.clamped_stretch() * height / width,
                paint: GlyphPaint::Glass {
                    whiteness: look.clamped_whiteness(),
                },
            },
            shadow: look.shadow.clamp(0.0, 1.0) * SHADOW_ALPHA,
            colon: look.colon,
        }
    }

    /// Hours, then the minutes when there are any, side by side on one
    /// baseline with the separator between them unless the colon is off.
    fn row(&self, hours: Block, minutes: Option<Block>) -> Block {
        let separator = (self.colon && minutes.is_some())
            .then(|| self.group(SEPARATOR, ClockGroup::Separator, [0.0; 2]));
        let rest = separator.into_iter().chain(minutes);
        flex(FlexDirection::Row, 0.0, AlignItems::END)
            .with_children([hours].into_iter().chain(rest).collect())
    }

    fn group(&self, text: &str, group: ClockGroup, shift: [f32; 2]) -> Block {
        let mut row = flex(FlexDirection::Row, 0.0, AlignItems::END)
            .with_children(text.chars().map(|ch| self.cell(ch, group)).collect());
        row.shift = shift;
        row
    }

    /// A character's face over its shadow, both sharing one box.
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
        let shadow = (self.shadow > 0.0 && !ch.is_whitespace())
            .then(|| glyph(GlyphPaint::Shadow { alpha: self.shadow }));
        Block::new(Style {
            display: Display::Grid,
            ..Style::default()
        })
        .with_children(
            shadow
                .into_iter()
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
