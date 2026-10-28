//! The layout drawn when no theme supplies one, built from the settings.

use crownos_config::schema::{Anchor, ClockStyle, LockLayout, LockSlotPosition};
use lockscene::Placeholder;
use taffy::{
    AlignItems, Display, JustifyContent, Line, Size, Style,
    prelude::{auto, fr, length, line, percent},
};

use super::{
    Context, Layer,
    block::Block,
    live::{self, column},
    node::{Axis, Inherited, convert},
};

const CLOCK_GAP: f32 = 6.0;
const PROMPT_GAP: f32 = 14.0;
const SLOT_GAP: f32 = 12.0;
const EDGE_MARGIN: f32 = 48.0;
const BOTTOM_MARGIN: f32 = 32.0;
/// Of the output's height, above a centred clock.
const CLOCK_TOP: f32 = 0.12;
/// Of the output's width, left of a left-aligned clock.
const CLOCK_LEFT: f32 = 0.08;
const BOTTOM_CLOCK_INSET: [f32; 2] = [64.0, 88.0];

pub(super) fn layout(cx: &Context<'_>, size: [f32; 2]) -> Block {
    let mut root = Block::new(Style {
        display: Display::Grid,
        grid_template_rows: vec![fr(1.0)],
        grid_template_columns: vec![fr(1.0)],
        size: Size {
            width: percent(1.0),
            height: percent(1.0),
        },
        ..Style::default()
    });
    root.children = [clock_group(cx, size), prompt_group(cx), bottom_group(cx)]
        .into_iter()
        .map(|mut group| {
            group.style.grid_row = Line {
                start: line(1),
                end: auto(),
            };
            group.style.grid_column = Line {
                start: line(1),
                end: auto(),
            };
            group
        })
        .collect();
    root
}

fn clock_group(cx: &Context<'_>, size: [f32; 2]) -> Block {
    let config = &cx.scene.config;
    let centred = config.layout == LockLayout::Center;
    let inherited = Inherited::root(cx, centred);
    let (align, justify, padding) = match config.layout {
        LockLayout::Center => (
            AlignItems::CENTER,
            JustifyContent::START,
            edges(0.0, size[1] * CLOCK_TOP, 0.0, 0.0),
        ),
        LockLayout::Left => (
            AlignItems::START,
            JustifyContent::CENTER,
            edges(size[0] * CLOCK_LEFT, 0.0, 0.0, 0.0),
        ),
        LockLayout::Bottom => (
            AlignItems::START,
            JustifyContent::END,
            edges(BOTTOM_CLOCK_INSET[0], 0.0, 0.0, BOTTOM_CLOCK_INSET[1]),
        ),
    };

    let mut children = slots(cx, LockSlotPosition::AboveClock, &inherited);
    let replacement = slots(cx, LockSlotPosition::Clock, &inherited);
    if replacement.is_empty() {
        children.extend(live::block(cx, Placeholder::Clock, &inherited));
        if config.clock_style != ClockStyle::Minimal {
            children.extend(live::block(cx, Placeholder::Date, &inherited));
        }
    } else {
        children.extend(replacement);
    }
    let below = slots(cx, LockSlotPosition::BelowClock, &inherited);
    if !below.is_empty() {
        children.push(
            column(SLOT_GAP, align)
                .with_children(below)
                .with_margin_top(SLOT_GAP),
        );
    }

    let mut group = column(CLOCK_GAP, align)
        .with_children(children)
        .on(Layer::Clock);
    group.style.justify_content = Some(justify);
    group.style.padding = padding;
    group
}

fn prompt_group(cx: &Context<'_>) -> Block {
    let inherited = Inherited::root(cx, true);
    let placement = cx.scene.config.auth_field;
    let parts = [
        Placeholder::User,
        Placeholder::SecureField,
        Placeholder::Caps,
        Placeholder::AuthPrompt,
    ];
    let prompt = column(PROMPT_GAP, AlignItems::CENTER)
        .with_children(
            parts
                .into_iter()
                .filter_map(|part| live::block(cx, part, &inherited))
                .collect(),
        )
        .on(Layer::Prompt);

    let (justify, align) = anchor(placement.anchor);
    let mut group = column(0.0, align)
        .with_children(vec![prompt])
        .on(Layer::Prompt);
    group.style.justify_content = Some(justify);
    group.style.padding = edges(EDGE_MARGIN, EDGE_MARGIN, EDGE_MARGIN, EDGE_MARGIN);
    group.shift = placement.offset;
    group
}

fn bottom_group(cx: &Context<'_>) -> Block {
    let inherited = Inherited::root(cx, true);
    let mut group = column(SLOT_GAP, AlignItems::CENTER)
        .with_children(slots(cx, LockSlotPosition::Bottom, &inherited))
        .on(Layer::Clock);
    group.style.justify_content = Some(JustifyContent::END);
    group.style.padding = edges(0.0, 0.0, 0.0, BOTTOM_MARGIN);
    group
}

fn slots(cx: &Context<'_>, position: LockSlotPosition, inherited: &Inherited) -> Vec<Block> {
    cx.scene
        .slots
        .iter()
        .filter(|slot| slot.position == position)
        .filter_map(|slot| {
            let mut block = convert(cx, &slot.tree.root, inherited, Axis::Column)?;
            block.shift = slot.offset;
            Some(block.on(Layer::Clock))
        })
        .collect()
}

/// Main-axis then cross-axis alignment for a column anchored at `anchor`.
fn anchor(anchor: Anchor) -> (JustifyContent, AlignItems) {
    let vertical = match anchor {
        Anchor::TopLeft | Anchor::Top | Anchor::TopRight => JustifyContent::START,
        Anchor::Left | Anchor::Center | Anchor::Right => JustifyContent::CENTER,
        Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => JustifyContent::END,
    };
    let horizontal = match anchor {
        Anchor::TopLeft | Anchor::Left | Anchor::BottomLeft => AlignItems::START,
        Anchor::Top | Anchor::Center | Anchor::Bottom => AlignItems::CENTER,
        Anchor::TopRight | Anchor::Right | Anchor::BottomRight => AlignItems::END,
    };
    (vertical, horizontal)
}

fn edges(left: f32, top: f32, right: f32, bottom: f32) -> taffy::Rect<taffy::LengthPercentage> {
    taffy::Rect {
        left: length(left),
        right: length(right),
        top: length(top),
        bottom: length(bottom),
    }
}
