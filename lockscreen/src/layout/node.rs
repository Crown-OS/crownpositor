use crownplugin_view::{Align, Justify, Length, Node, Radius, Style as Class, Tone};
use lockscene::{AssetId, Placeholder};
use taffy::{
    AlignItems, AlignSelf, Dimension, Display, FlexDirection, JustifyContent, LengthPercentage,
    Line, Size, Style,
    prelude::{auto, fr, length, line, percent},
};

use super::{Content, Context, FontFamily, TextRun, block::Block, live};
use crate::palette::Rgba;

const BASE_TEXT: f32 = 16.0;
const LINE_HEIGHT: f32 = 1.3;
const LARGEST_INTRINSIC_IMAGE: f32 = 512.0;
const FALLBACK_ICON: &str = "circle";
const CARD_PADDING: f32 = 16.0;
const CARD_GAP: f32 = 8.0;
const FULL_RADIUS: f32 = 9999.0;

/// Text styling that flows from a container into the text inside it.
#[derive(Debug, Clone)]
pub(super) struct Inherited {
    pub color: Rgba,
    pub size: f32,
    pub weight: u16,
    pub mono: bool,
    pub centered: bool,
    pub single_line: bool,
    pub opacity: f32,
}

impl Inherited {
    pub fn root(cx: &Context<'_>, centered: bool) -> Self {
        Self {
            color: cx.palette.color("text.primary"),
            size: BASE_TEXT * cx.text_scale,
            weight: 400,
            mono: false,
            centered,
            single_line: false,
            opacity: 1.0,
        }
    }

    pub fn run(&self, cx: &Context<'_>, text: &str) -> TextRun {
        let [r, g, b, a] = self.color;
        TextRun {
            text: text.to_owned(),
            size: self.size,
            line_height: (self.size * LINE_HEIGHT).ceil(),
            weight: self.weight,
            family: if self.mono {
                FontFamily::Mono(cx.mono.to_owned())
            } else {
                FontFamily::Sans(cx.family.to_owned())
            },
            color: [r, g, b, a * self.opacity],
            centered: self.centered,
            single_line: self.single_line,
        }
    }

    pub fn tint(&self, color: Rgba) -> Rgba {
        let [r, g, b, a] = color;
        [r, g, b, a * self.opacity]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Axis {
    Row,
    Column,
}

/// A theme's layout tree, filling the output.
pub(super) fn layout_root(cx: &Context<'_>, root: &Node) -> Block {
    let mut block = convert(cx, root, &Inherited::root(cx, true), Axis::Column)
        .unwrap_or_else(|| Block::new(Style::default()));
    block.style.size = Size {
        width: percent(1.0),
        height: percent(1.0),
    };
    block
}

pub(super) fn convert(
    cx: &Context<'_>,
    node: &Node,
    inherited: &Inherited,
    parent: Axis,
) -> Option<Block> {
    let mut text = inherited.clone();
    let mut panel = Panel::default();
    let mut style = Style::default();
    if let Some(classes) = classes(node) {
        if classes.contains(&Class::Hidden) {
            return None;
        }
        apply(cx, classes, &mut style, &mut text, &mut panel);
    }
    match node {
        Node::VStack { children, .. } => {
            container(cx, style, FlexDirection::Column, children, &text, panel)
        }
        Node::HStack { children, .. } => {
            container(cx, style, FlexDirection::Row, children, &text, panel)
        }
        Node::Card { children, .. } => {
            if style.padding == taffy::Rect::zero() {
                style.padding = uniform(CARD_PADDING);
            }
            style.gap = Size {
                width: length(CARD_GAP),
                height: length(CARD_GAP),
            };
            panel
                .fill
                .get_or_insert_with(|| cx.palette.color("surface.raised"));
            panel.radius.get_or_insert(cx.scene.theme.radii.lg);
            container(cx, style, FlexDirection::Column, children, &text, panel)
        }
        Node::ZStack { children, .. } => {
            style.display = Display::Grid;
            style.grid_template_rows = vec![fr(1.0)];
            style.grid_template_columns = vec![fr(1.0)];
            let children = children
                .iter()
                .filter_map(|child| convert(cx, child, &text, Axis::Column))
                .map(|mut child| {
                    child.style.grid_row = Line {
                        start: line(1),
                        end: auto(),
                    };
                    child.style.grid_column = Line {
                        start: line(1),
                        end: auto(),
                    };
                    child
                })
                .collect();
            Some(
                panel
                    .paint(&text, Block::new(style))
                    .with_children(children),
            )
        }
        Node::Text { text: words, .. } => {
            Some(Block::new(style).with_content(Content::Text(text.run(cx, words))))
        }
        Node::Icon { name, .. } => {
            let body = crownplugin_view::lucide(name)
                .or_else(|| crownplugin_view::lucide(FALLBACK_ICON))?;
            default_size(&mut style, text.size, text.size);
            Some(Block::new(style).with_content(Content::Icon {
                body,
                color: text.tint(text.color),
            }))
        }
        Node::Image { src, .. } => {
            let asset = AssetId::from_src(src)?;
            let [width, height] = cx
                .images
                .get(&asset)
                .map(|image| intrinsic(image.width() as f32, image.height() as f32))
                .unwrap_or([0.0; 2]);
            default_size(&mut style, width, height);
            Some(Block::new(style).with_content(Content::Image {
                asset,
                radius: panel.radius.unwrap_or_default(),
            }))
        }
        Node::Spacer => {
            style.flex_grow = 1.0;
            Some(Block::new(style))
        }
        Node::Divider => {
            let color = text.tint(cx.palette.color("surface.hairline"));
            let style = match parent {
                Axis::Column => Style {
                    size: Size {
                        width: auto(),
                        height: length(1.0),
                    },
                    align_self: Some(AlignSelf::STRETCH),
                    flex_shrink: 0.0,
                    ..Style::default()
                },
                Axis::Row => Style {
                    size: Size {
                        width: length(1.0),
                        height: auto(),
                    },
                    align_self: Some(AlignSelf::STRETCH),
                    flex_shrink: 0.0,
                    ..Style::default()
                },
            };
            Some(Block::new(style).with_content(Content::Divider { color }))
        }
        Node::Badge { text: words, tone } => Some(badge(cx, words, *tone, &text)),
        Node::Progress { value, .. } => {
            if style.size.height == Dimension::auto() {
                style.size.height = length(6.0);
            }
            style.min_size.width = length(60.0);
            style.flex_grow = 1.0;
            style.align_self = Some(AlignSelf::STRETCH);
            Some(Block::new(style).with_content(Content::Progress {
                value: value.clamp(0.0, 1.0),
                track: text.tint(cx.palette.color("control.track")),
                fill: text.tint(cx.palette.color("accent.end")),
            }))
        }
        Node::Placeholder { name } => {
            let part = Placeholder::parse(name)?;
            let mut block = live::block(cx, part, &text)?;
            block.layer.get_or_insert(Context::placeholder_layer(name));
            Some(block)
        }
        Node::Button { .. } | Node::Toggle { .. } | Node::Slider { .. } | Node::Select { .. } => {
            None
        }
    }
}

fn container(
    cx: &Context<'_>,
    mut style: Style,
    direction: FlexDirection,
    children: &[Node],
    text: &Inherited,
    panel: Panel,
) -> Option<Block> {
    style.display = Display::Flex;
    style.flex_direction = direction;
    let axis = match direction {
        FlexDirection::Row | FlexDirection::RowReverse => Axis::Row,
        FlexDirection::Column | FlexDirection::ColumnReverse => Axis::Column,
    };
    let children = children
        .iter()
        .filter_map(|child| convert(cx, child, text, axis))
        .collect();
    Some(panel.paint(text, Block::new(style)).with_children(children))
}

fn badge(cx: &Context<'_>, words: &str, tone: Tone, inherited: &Inherited) -> Block {
    let (fill, ink) = match tone {
        Tone::Neutral => ("surface.raised", "text.body"),
        Tone::Accent => ("accent.end", "text.on_accent"),
        Tone::Success => ("status.success", "text.on_accent"),
        Tone::Warning => ("status.warning", "text.on_accent"),
        Tone::Danger => ("status.danger_bg", "status.on_danger"),
    };
    let text = Inherited {
        color: cx.palette.color(ink),
        size: 12.0 * cx.text_scale,
        weight: 600,
        single_line: true,
        ..inherited.clone()
    };
    let style = Style {
        padding: taffy::Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(2.0),
            bottom: length(2.0),
        },
        flex_shrink: 0.0,
        ..Style::default()
    };
    Block::new(style)
        .with_content(Content::Panel {
            fill: Some(inherited.tint(cx.palette.color(fill))),
            border: None,
            radius: FULL_RADIUS,
        })
        .with_children(vec![
            Block::new(Style::default()).with_content(Content::Text(text.run(cx, words))),
        ])
}

#[derive(Debug, Default)]
struct Panel {
    fill: Option<Rgba>,
    border: Option<(f32, Option<Rgba>)>,
    radius: Option<f32>,
}

impl Panel {
    fn paint(self, inherited: &Inherited, block: Block) -> Block {
        if self.fill.is_none() && self.border.is_none() {
            return block;
        }
        let border = self.border.map(|(width, color)| {
            (
                width,
                inherited.tint(color.unwrap_or([1.0, 1.0, 1.0, 0.15])),
            )
        });
        block.with_content(Content::Panel {
            fill: self.fill.map(|fill| inherited.tint(fill)),
            border,
            radius: self.radius.unwrap_or_default(),
        })
    }
}

fn classes(node: &Node) -> Option<&[Class]> {
    match node {
        Node::VStack { style, .. }
        | Node::HStack { style, .. }
        | Node::ZStack { style, .. }
        | Node::Card { style, .. }
        | Node::Text { style, .. }
        | Node::Icon { style, .. }
        | Node::Image { style, .. }
        | Node::Progress { style, .. } => Some(style),
        _ => None,
    }
}

fn apply(
    cx: &Context<'_>,
    classes: &[Class],
    style: &mut Style,
    text: &mut Inherited,
    panel: &mut Panel,
) {
    let radii = cx.scene.theme.radii;
    for class in classes {
        match class {
            Class::P(value) => style.padding = uniform(*value),
            Class::Px(value) => {
                style.padding.left = length(*value);
                style.padding.right = length(*value);
            }
            Class::Py(value) => {
                style.padding.top = length(*value);
                style.padding.bottom = length(*value);
            }
            Class::Pt(value) => style.padding.top = length(*value),
            Class::Pb(value) => style.padding.bottom = length(*value),
            Class::Pl(value) => style.padding.left = length(*value),
            Class::Pr(value) => style.padding.right = length(*value),
            Class::Gap(value) => {
                style.gap = Size {
                    width: length(*value),
                    height: length(*value),
                }
            }
            Class::W(value) => style.size.width = dimension(*value),
            Class::H(value) => style.size.height = dimension(*value),
            Class::MinW(value) => style.min_size.width = length(*value),
            Class::MaxW(value) => style.max_size.width = length(*value),
            Class::Rounded(radius) => {
                panel.radius = Some(match radius {
                    Radius::None => 0.0,
                    Radius::Sm => radii.sm,
                    Radius::Md => radii.md,
                    Radius::Lg => radii.lg,
                    Radius::Xl => radii.xl,
                    Radius::Full => FULL_RADIUS,
                })
            }
            Class::Bg(token) => panel.fill = Some(cx.palette.color(token.as_str())),
            Class::BorderColor(token) => {
                let width = panel.border.map_or(1.0, |(width, _)| width);
                panel.border = Some((width, Some(cx.palette.color(token.as_str()))));
            }
            Class::Border(width) => {
                let color = panel.border.and_then(|(_, color)| color);
                panel.border = Some((*width, color));
            }
            Class::TextColor(token) => text.color = cx.palette.color(token.as_str()),
            Class::TextSize(size) => text.size = size.px() * cx.text_scale,
            Class::Weight(weight) => text.weight = weight.value(),
            Class::Mono => text.mono = true,
            Class::TextCenter => text.centered = true,
            Class::Opacity(value) => text.opacity *= value,
            Class::Items(align) => style.align_items = Some(align_items(*align)),
            Class::Justify(justify) => style.justify_content = Some(justify_content(*justify)),
            Class::Grow => style.flex_grow = 1.0,
            Class::Truncate => text.single_line = true,
            Class::Hidden => {}
        }
    }
}

pub(super) fn uniform(value: f32) -> taffy::Rect<LengthPercentage> {
    taffy::Rect {
        left: length(value),
        right: length(value),
        top: length(value),
        bottom: length(value),
    }
}

fn dimension(value: Length) -> Dimension {
    match value {
        Length::Px(value) => length(value),
        Length::Full => percent(1.0),
        Length::Fit => auto(),
    }
}

pub(super) fn align_items(align: Align) -> AlignItems {
    match align {
        Align::Start => AlignItems::START,
        Align::Center => AlignItems::CENTER,
        Align::End => AlignItems::END,
        Align::Stretch => AlignItems::STRETCH,
    }
}

fn justify_content(justify: Justify) -> JustifyContent {
    match justify {
        Justify::Start => JustifyContent::START,
        Justify::Center => JustifyContent::CENTER,
        Justify::End => JustifyContent::END,
        Justify::Between => JustifyContent::SPACE_BETWEEN,
    }
}

fn default_size(style: &mut Style, width: f32, height: f32) {
    if style.size.width == Dimension::auto() {
        style.size.width = length(width);
    }
    if style.size.height == Dimension::auto() {
        style.size.height = length(height);
    }
    style.flex_shrink = 0.0;
}

fn intrinsic(width: f32, height: f32) -> [f32; 2] {
    let shrink = (LARGEST_INTRINSIC_IMAGE / width.max(height)).min(1.0);
    [width * shrink, height * shrink]
}
