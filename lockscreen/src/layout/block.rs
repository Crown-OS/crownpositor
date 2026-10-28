use taffy::{AvailableSpace, NodeId, Size, Style, TaffyTree, compute_leaf_layout, prelude::length};

use super::{Content, Layer, Part, Rect, Shaper, Sprite};

/// One box of the layout before flexbox has placed it.
#[derive(Debug, Default)]
pub(crate) struct Block {
    pub style: Style,
    pub content: Option<Content>,
    pub part: Option<Part>,
    /// Set by placeholders; everything else takes its layer from them.
    pub layer: Option<Layer>,
    /// Moves the box and everything in it after layout, in logical pixels.
    pub shift: [f32; 2],
    pub children: Vec<Block>,
}

impl Block {
    pub fn new(style: Style) -> Self {
        Self {
            style,
            ..Self::default()
        }
    }

    pub fn with_children(mut self, children: Vec<Block>) -> Self {
        self.children = children;
        self
    }

    pub fn with_content(mut self, content: Content) -> Self {
        self.content = Some(content);
        self
    }

    pub fn with_margin_top(mut self, margin: f32) -> Self {
        self.style.margin.top = length(margin);
        self
    }

    pub fn on(mut self, layer: Layer) -> Self {
        self.layer = Some(layer);
        self
    }

    /// The layer a box without its own belongs to: the prompt only when every
    /// placeholder inside it is part of the prompt.
    fn settle_layers(&mut self, inherited: Layer) {
        let own = self.layer.unwrap_or_else(|| {
            let mut found = (false, false);
            self.explicit_layers(&mut found);
            match found {
                (true, false) => Layer::Prompt,
                (false, true) => Layer::Clock,
                _ => inherited,
            }
        });
        self.layer = Some(own);
        for child in &mut self.children {
            child.settle_layers(own);
        }
    }

    fn explicit_layers(&self, found: &mut (bool, bool)) {
        match self.layer {
            Some(Layer::Prompt) => found.0 = true,
            Some(Layer::Clock) => found.1 = true,
            None => self
                .children
                .iter()
                .for_each(|child| child.explicit_layers(found)),
        }
    }
}

struct Placed {
    node: NodeId,
    id: u64,
    content: Option<Content>,
    part: Option<Part>,
    layer: Layer,
    shift: [f32; 2],
    children: Vec<usize>,
}

/// Lays `root` out over `size` and cuts it into sprites, back to front.
pub(crate) fn place(shaper: &mut Shaper, mut root: Block, size: [f32; 2]) -> Vec<Sprite> {
    root.settle_layers(Layer::Clock);
    let mut tree: TaffyTree<usize> = TaffyTree::new();
    let mut placed = Vec::new();
    let Ok(root_index) = build(&mut tree, &mut placed, root, ROOT_ID) else {
        return Vec::new();
    };
    let available = Size {
        width: AvailableSpace::Definite(size[0]),
        height: AvailableSpace::Definite(size[1]),
    };
    let laid_out = tree.compute_layout_with_measure(
        placed[root_index].node,
        available,
        |inputs, _, context, style| {
            compute_leaf_layout(
                inputs,
                style,
                |_, _| 0.0,
                |known, available| {
                    let [width, height] =
                        match context.and_then(|index| placed[*index].content.as_ref()) {
                            Some(Content::Text(run)) => {
                                let width = known.width.or(match available.width {
                                    AvailableSpace::Definite(width) => Some(width),
                                    AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
                                });
                                shaper.measure(run, width)
                            }
                            Some(Content::Glyph(run)) => shaper
                                .glyph(run)
                                .map_or([0.0; 2], |shape| shape.size(run.stretch)),
                            _ => return Size::ZERO,
                        };
                    Size {
                        width: known.width.unwrap_or(width),
                        height: known.height.unwrap_or(height),
                    }
                },
            )
        },
    );
    if laid_out.is_err() {
        return Vec::new();
    }
    let mut sprites = Vec::with_capacity(placed.len());
    collect(&tree, &placed, root_index, [0.0; 2], &mut sprites);
    sprites
}

const ROOT_ID: u64 = 0xcbf2_9ce4_8422_2325;

fn child_id(parent: u64, index: usize) -> u64 {
    (parent ^ index as u64).wrapping_mul(0x0100_0000_01b3)
}

fn build(
    tree: &mut TaffyTree<usize>,
    placed: &mut Vec<Placed>,
    block: Block,
    id: u64,
) -> taffy::TaffyResult<usize> {
    let index = placed.len();
    let measured = matches!(block.content, Some(Content::Text(_) | Content::Glyph(_)))
        && block.children.is_empty();
    let node = if measured {
        tree.new_leaf_with_context(block.style, index)?
    } else {
        tree.new_leaf(block.style)?
    };
    placed.push(Placed {
        node,
        id,
        content: block.content,
        part: block.part,
        layer: block.layer.unwrap_or(Layer::Clock),
        shift: block.shift,
        children: Vec::with_capacity(block.children.len()),
    });
    for (position, child) in block.children.into_iter().enumerate() {
        let child_index = build(tree, placed, child, child_id(id, position))?;
        tree.add_child(node, placed[child_index].node)?;
        placed[index].children.push(child_index);
    }
    Ok(index)
}

fn collect(
    tree: &TaffyTree<usize>,
    placed: &[Placed],
    index: usize,
    origin: [f32; 2],
    sprites: &mut Vec<Sprite>,
) {
    let block = &placed[index];
    let Ok(layout) = tree.layout(block.node) else {
        return;
    };
    let x = origin[0] + layout.location.x + block.shift[0];
    let y = origin[1] + layout.location.y + block.shift[1];
    if let Some(content) = &block.content
        && layout.size.width > 0.0
        && layout.size.height > 0.0
    {
        sprites.push(Sprite {
            id: block.id,
            layer: block.layer,
            part: block.part,
            rect: Rect {
                x,
                y,
                width: layout.size.width,
                height: layout.size.height,
            },
            content: content.clone(),
        });
    }
    for &child in &block.children {
        collect(tree, placed, child, [x, y], sprites);
    }
}

/// A fixed `width` × `height` box.
pub(crate) fn sized(width: f32, height: f32) -> Style {
    Style {
        size: Size {
            width: length(width),
            height: length(height),
        },
        flex_shrink: 0.0,
        ..Style::default()
    }
}
