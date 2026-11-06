//! The layout drawn when no theme supplies one, built from the settings.

use crownos_config::schema::{Anchor, ClockLayout, LockSlotPosition};
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
/// Of the output's height, above the clock: where ColorOS's date tops a
/// centred clock, so its digits start 0.147 of the way down.
const CLOCK_TOP: f32 = 0.1;
/// Of the output's width, left of a clock laid out at the left.
const CLOCK_LEFT: f32 = 0.08;

pub(super) fn layout(cx: &Context<'_>) -> Block {
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
    root.children = [clock_group(cx), prompt_group(cx), bottom_group(cx)]
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

/// The clock with its date, top-centre when centred and at the top left
/// otherwise, between the slots placed around it.
fn clock_group(cx: &Context<'_>) -> Block {
    let [width, height] = cx.output;
    let centred = cx.scene.config.clock.layout == ClockLayout::Centered;
    let inherited = Inherited::root(cx, centred);
    let (align, left) = if centred {
        (AlignItems::CENTER, 0.0)
    } else {
        (AlignItems::START, width * CLOCK_LEFT)
    };

    let mut children = slots(cx, LockSlotPosition::AboveClock, &inherited);
    let replacement = slots(cx, LockSlotPosition::Clock, &inherited);
    if replacement.is_empty() {
        children.extend(live::block(cx, Placeholder::Clock, &inherited));
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
    group.style.justify_content = Some(JustifyContent::START);
    group.style.padding = edges(left, height * CLOCK_TOP, 0.0, 0.0);
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
