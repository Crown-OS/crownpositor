mod config;
mod tree;

use std::{collections::HashSet, ops::Deref};

use crownplugin_view::Tree;

use crate::{Limits, LockScene, SceneError};

/// A scene that passed [`validate`]; the only kind a host should draw.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ValidScene(LockScene);

impl ValidScene {
    pub fn into_inner(self) -> LockScene {
        self.0
    }

    /// Swaps one slot's tree for a newer one, keeping its placement.
    pub fn replace_slot(&mut self, id: &str, tree: Tree) -> Result<(), SceneError> {
        let slot = self
            .0
            .slots
            .iter_mut()
            .find(|slot| slot.id == id)
            .ok_or_else(|| SceneError::BadPlaceholder(id.to_owned()))?;
        slot.tree = validate_slot(tree)?;
        Ok(())
    }
}

impl Deref for ValidScene {
    type Target = LockScene;

    fn deref(&self) -> &LockScene {
        &self.0
    }
}

pub fn validate(scene: LockScene) -> Result<ValidScene, SceneError> {
    config::check(&scene.config, &scene.theme)?;
    if scene.slots.len() > Limits::SLOTS {
        return Err(SceneError::TooManySlots {
            limit: Limits::SLOTS,
        });
    }
    let mut ids = HashSet::with_capacity(scene.slots.len());
    for slot in &scene.slots {
        if !is_slot_id(&slot.id) {
            return Err(SceneError::BadSlotId(slot.id.clone()));
        }
        if !ids.insert(slot.id.as_str()) {
            return Err(SceneError::DuplicateSlot(slot.id.clone()));
        }
        config::check_offset(slot.offset)?;
        tree::check(&slot.tree.root, tree::Scope::Slot)?;
    }
    if let Some(decoration) = &scene.decoration {
        let secure_fields = tree::check(&decoration.root, tree::Scope::Layout { slots: &ids })?;
        if secure_fields != 1 {
            return Err(SceneError::SecureFieldCount(secure_fields));
        }
    }
    Ok(ValidScene(scene))
}

pub fn validate_slot(tree: Tree) -> Result<Tree, SceneError> {
    tree::check(&tree.root, tree::Scope::Slot)?;
    Ok(tree)
}

fn is_slot_id(id: &str) -> bool {
    let Some((plugin, widget)) = id
        .strip_prefix("plugin:")
        .and_then(|rest| rest.split_once('/'))
    else {
        return false;
    };
    let part = |text: &str, extra: char| {
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == extra)
    };
    id.len() <= Limits::ID_CHARS && part(plugin, '.') && part(widget, '-')
}

#[cfg(test)]
mod tests {
    use crownos_config::schema::{Lock, LockBackground, LockSlotPosition};
    use crownplugin_view::Node;

    use super::*;
    use crate::{AssetId, Slot};

    fn slot(id: &str, root: Node) -> Slot {
        Slot {
            id: id.into(),
            position: LockSlotPosition::BelowClock,
            offset: [0.0; 2],
            tree: Tree { rev: 0, root },
        }
    }

    fn text(text: &str) -> Node {
        Node::Text {
            style: Vec::new(),
            text: text.into(),
        }
    }

    fn placeholder(name: &str) -> Node {
        Node::Placeholder { name: name.into() }
    }

    #[test]
    fn the_default_scene_is_valid() {
        assert!(validate(LockScene::default()).is_ok());
    }

    #[test]
    fn slot_ids_are_checked() {
        for id in ["plugin:a.b/c", "plugin:dev.x-y/now-playing"] {
            assert!(is_slot_id(id), "{id}");
        }
        for id in [
            "a.b/c",
            "plugin:a.b",
            "plugin:A/b",
            "plugin:a/b/c",
            "plugin:/x",
        ] {
            assert!(!is_slot_id(id), "{id}");
        }
    }

    #[test]
    fn a_layout_needs_exactly_one_secure_field() {
        let layout = |children| LockScene {
            decoration: Some(Tree {
                rev: 0,
                root: Node::VStack {
                    style: Vec::new(),
                    children,
                },
            }),
            ..LockScene::default()
        };
        assert!(matches!(
            validate(layout(vec![placeholder("lock.clock")])),
            Err(SceneError::SecureFieldCount(0))
        ));
        assert!(matches!(
            validate(layout(vec![
                placeholder("lock.secure_field"),
                placeholder("lock.secure_field")
            ])),
            Err(SceneError::SecureFieldCount(2))
        ));
        assert!(validate(layout(vec![placeholder("lock.secure_field")])).is_ok());
    }

    #[test]
    fn slots_may_not_hold_placeholders_or_paths() {
        let scene = |root| LockScene {
            slots: vec![slot("plugin:a.b/c", root)],
            ..LockScene::default()
        };
        assert!(validate(scene(placeholder("lock.secure_field"))).is_err());
        let path = Node::Image {
            style: Vec::new(),
            src: "/etc/shadow".into(),
        };
        assert!(matches!(
            validate(scene(path)),
            Err(SceneError::NotAnAsset(_))
        ));
        let asset = Node::Image {
            style: Vec::new(),
            src: AssetId::of(b"x").to_src(),
        };
        assert!(validate(scene(asset)).is_ok());
    }

    #[test]
    fn a_layout_may_only_name_slots_the_scene_has() {
        let scene = LockScene {
            slots: vec![slot("plugin:a.b/c", text("hi"))],
            decoration: Some(Tree {
                rev: 0,
                root: Node::VStack {
                    style: Vec::new(),
                    children: vec![
                        placeholder("lock.secure_field"),
                        placeholder("lock.slot:plugin:a.b/c"),
                        placeholder("lock.slot:plugin:x.y/z"),
                    ],
                },
            }),
            ..LockScene::default()
        };
        assert!(matches!(
            validate(scene),
            Err(SceneError::BadPlaceholder(name)) if name.ends_with("x.y/z")
        ));
    }

    #[test]
    fn hostile_sizes_are_refused() {
        let deep = (0..Limits::DEPTH + 1).fold(text("x"), |inner, _| Node::VStack {
            style: Vec::new(),
            children: vec![inner],
        });
        assert!(matches!(
            validate_slot(Tree { rev: 0, root: deep }),
            Err(SceneError::TooDeep { .. })
        ));
        let wide = Node::HStack {
            style: Vec::new(),
            children: vec![Node::Spacer; Limits::NODES],
        };
        assert!(matches!(
            validate_slot(Tree { rev: 0, root: wide }),
            Err(SceneError::TooManyNodes { .. })
        ));
        let long = text(&"x".repeat(Limits::TEXT_CHARS + 1));
        assert!(matches!(
            validate_slot(Tree { rev: 0, root: long }),
            Err(SceneError::TextTooLong { .. })
        ));
    }

    #[test]
    fn config_values_are_bounded() {
        let scene = |config| LockScene {
            config,
            ..LockScene::default()
        };
        assert!(
            validate(scene(Lock {
                dim: f32::NAN,
                ..Lock::default()
            }))
            .is_err()
        );
        assert!(
            validate(scene(Lock {
                time_format: "%Q".into(),
                ..Lock::default()
            }))
            .is_err()
        );
        assert!(
            validate(scene(Lock {
                background: LockBackground::Image("relative.png".into()),
                ..Lock::default()
            }))
            .is_err()
        );
        assert!(
            validate(scene(Lock {
                background: LockBackground::Token("surface.bg".into()),
                ..Lock::default()
            }))
            .is_ok()
        );
    }
}
