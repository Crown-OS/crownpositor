use std::collections::HashSet;

use crownplugin_view::{Length, Node, Style};

use crate::{AssetId, Limits, Placeholder, SceneError};

pub(super) enum Scope<'a> {
    Slot,
    /// A theme layout, which may name these slots.
    Layout {
        slots: &'a HashSet<&'a str>,
    },
}

/// Checks every node, returning how many secure fields the tree holds.
pub(super) fn check(root: &Node, scope: Scope<'_>) -> Result<usize, SceneError> {
    let mut walk = Walk {
        scope,
        nodes: 0,
        secure_fields: 0,
    };
    walk.node(root, 1)?;
    Ok(walk.secure_fields)
}

struct Walk<'a> {
    scope: Scope<'a>,
    nodes: usize,
    secure_fields: usize,
}

impl Walk<'_> {
    fn node(&mut self, node: &Node, depth: usize) -> Result<(), SceneError> {
        self.nodes += 1;
        if self.nodes > Limits::NODES {
            return Err(SceneError::TooManyNodes {
                limit: Limits::NODES,
            });
        }
        if depth > Limits::DEPTH {
            return Err(SceneError::TooDeep {
                limit: Limits::DEPTH,
            });
        }
        if node.is_interactive() {
            return Err(SceneError::Interactive);
        }
        match node {
            Node::VStack { style, .. }
            | Node::HStack { style, .. }
            | Node::ZStack { style, .. }
            | Node::Card { style, .. }
            | Node::Icon { style, .. }
            | Node::Progress { style, .. } => styles(style)?,
            Node::Text { style, text } => {
                styles(style)?;
                short(text)?;
            }
            Node::Badge { text, .. } => short(text)?,
            Node::Image { style, src } => {
                styles(style)?;
                if AssetId::from_src(src).is_none() {
                    return Err(SceneError::NotAnAsset(src.clone()));
                }
            }
            Node::Placeholder { name } => self.placeholder(name)?,
            _ => {}
        }
        if let Node::Icon { name, .. } = node {
            short(name)?;
        }
        if let Node::Progress { value, .. } = node
            && !value.is_finite()
        {
            return Err(SceneError::BadStyle);
        }
        node.children()
            .iter()
            .try_for_each(|child| self.node(child, depth + 1))
    }

    fn placeholder(&mut self, name: &str) -> Result<(), SceneError> {
        let Scope::Layout { slots } = &self.scope else {
            return Err(SceneError::BadPlaceholder(name.to_owned()));
        };
        match Placeholder::parse(name) {
            Some(Placeholder::SecureField) => self.secure_fields += 1,
            Some(Placeholder::Slot(id)) if !slots.contains(id) => {
                return Err(SceneError::BadPlaceholder(name.to_owned()));
            }
            Some(_) => {}
            None => return Err(SceneError::BadPlaceholder(name.to_owned())),
        }
        Ok(())
    }
}

fn short(text: &str) -> Result<(), SceneError> {
    if text.chars().count() > Limits::TEXT_CHARS {
        Err(SceneError::TextTooLong {
            limit: Limits::TEXT_CHARS,
        })
    } else {
        Ok(())
    }
}

fn styles(styles: &[Style]) -> Result<(), SceneError> {
    let spacing = |value: f32| (0.0..=Limits::LENGTH).contains(&value);
    let length = |length: Length| match length {
        Length::Px(value) => spacing(value),
        Length::Full | Length::Fit => true,
    };
    let sane = styles.len() <= 64
        && styles.iter().all(|style| match *style {
            Style::P(value)
            | Style::Px(value)
            | Style::Py(value)
            | Style::Pt(value)
            | Style::Pb(value)
            | Style::Pl(value)
            | Style::Pr(value)
            | Style::Gap(value)
            | Style::MinW(value)
            | Style::MaxW(value) => spacing(value),
            Style::W(value) | Style::H(value) => length(value),
            Style::Border(width) => (0.0..=16.0).contains(&width),
            Style::Opacity(value) => (0.0..=1.0).contains(&value),
            _ => true,
        });
    if sane {
        Ok(())
    } else {
        Err(SceneError::BadStyle)
    }
}
