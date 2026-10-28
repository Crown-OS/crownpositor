use lockscene::Placeholder;
use taffy::{AlignItems, Display, FlexDirection, Size, Style, prelude::length};

use super::{
    Content, Context, Layer, Part,
    block::{Block, sized},
    clock,
    node::{Axis, Inherited, convert},
};

const FIELD: [f32; 2] = [280.0, 44.0];
const AVATAR: f32 = 76.0;
const CAPS_ICON: &str = "arrow-big-up-dash";

/// The box a placeholder stands for, or `None` when it shows nothing now.
pub(super) fn block(
    cx: &Context<'_>,
    part: Placeholder<'_>,
    inherited: &Inherited,
) -> Option<Block> {
    let palette = cx.palette;
    let scale = cx.text_scale;
    let live = cx.live;
    let text = |size: f32, weight: u16, color: &str| Inherited {
        size: size * scale,
        weight,
        color: palette.color(color),
        ..inherited.clone()
    };
    match part {
        Placeholder::Clock => Some(clock::block(cx, inherited)),
        Placeholder::Date => {
            let run = text(22.0, 500, "text.body").run(cx, &live.face.date);
            Some(leaf(Content::Text(run)))
        }
        Placeholder::SecureField => {
            let mut hint = text(15.0, 400, "text.muted").run(cx, "Password");
            hint.centered = true;
            hint.single_line = true;
            let mut field = Block::new(sized(FIELD[0], FIELD[1])).with_content(Content::Field {
                dots: live.password_chars,
                busy: live.checking,
                hint,
                fill: palette.color("surface.hover"),
                border: palette.color("surface.border"),
                dot: palette.color("text.primary"),
            });
            field.part = Some(Part::SecureField);
            Some(field.on(Layer::Prompt))
        }
        Placeholder::AuthPrompt => {
            let (words, error) = live.notice.unwrap_or(("", false));
            let color = if error { "status.danger" } else { "text.muted" };
            let mut run = text(14.0, 500, color).run(cx, words);
            run.centered = true;
            let style = Style {
                max_size: Size {
                    width: length(FIELD[0] * 1.6),
                    height: taffy::prelude::auto(),
                },
                ..Style::default()
            };
            Some(
                Block::new(style)
                    .with_content(Content::Text(run))
                    .on(Layer::Prompt),
            )
        }
        Placeholder::User => {
            let user = live.user?;
            let mut initial =
                text(32.0, 600, "text.on_accent").run(cx, &user.initial().to_string());
            initial.centered = true;
            let avatar = Block::new(sized(AVATAR, AVATAR)).with_content(Content::Avatar {
                initial,
                fill: [palette.color("accent.start"), palette.color("accent.end")],
            });
            let mut name = text(18.0, 600, "text.primary").run(cx, &user.display_name);
            name.centered = true;
            name.single_line = true;
            Some(
                column(10.0, AlignItems::CENTER)
                    .with_children(vec![avatar, leaf(Content::Text(name))])
                    .on(Layer::Prompt),
            )
        }
        Placeholder::Caps if live.caps_lock => {
            let icon_size = 15.0 * scale;
            let icon = crownplugin_view::lucide(CAPS_ICON)?;
            let color = palette.color("status.warning");
            let style = Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::CENTER),
                gap: Size {
                    width: length(6.0),
                    height: length(6.0),
                },
                ..Style::default()
            };
            Some(
                Block::new(style)
                    .with_children(vec![
                        Block::new(sized(icon_size, icon_size))
                            .with_content(Content::Icon { body: icon, color }),
                        leaf(Content::Text(
                            text(13.0, 500, "status.warning").run(cx, "Caps Lock is on"),
                        )),
                    ])
                    .on(Layer::Prompt),
            )
        }
        Placeholder::Caps | Placeholder::Power => None,
        Placeholder::Slot(id) => {
            let slot = cx.scene.slot(id)?;
            let mut block = convert(cx, &slot.tree.root, inherited, Axis::Column)?;
            block.shift = slot.offset;
            Some(block.on(Layer::Clock))
        }
    }
}

fn leaf(content: Content) -> Block {
    Block::new(Style::default()).with_content(content)
}

pub(super) fn column(gap: f32, align: AlignItems) -> Block {
    Block::new(Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        align_items: Some(align),
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        ..Style::default()
    })
}
