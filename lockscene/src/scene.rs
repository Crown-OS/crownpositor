use std::collections::BTreeSet;

use crownos_config::schema::{Lock, LockBackground, LockSlotPosition, Theme};
use crownplugin_view::{Node, Tree};
use serde::{Deserialize, Serialize};

use crate::{AssetId, Limits, SceneError};

/// One plugin's content on the lock screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// `plugin:<id>/<widget>`.
    pub id: String,
    pub position: LockSlotPosition,
    pub offset: [f32; 2],
    pub tree: Tree,
}

/// Everything the lock screen draws besides the live parts — clock, field,
/// messages — which the compositor fills in itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LockScene {
    pub config: Lock,
    pub theme: Theme,
    pub slots: Vec<Slot>,
    /// A theme's whole layout, built around placeholders. `None` lays the
    /// screen out from `config` alone.
    pub decoration: Option<Tree>,
}

impl LockScene {
    pub fn without_slots(&self) -> Self {
        Self {
            config: self.config.clone(),
            theme: self.theme.clone(),
            slots: Vec::new(),
            decoration: self.decoration.clone(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_stdvec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, SceneError> {
        if bytes.len() > Limits::SCENE_BYTES {
            return Err(SceneError::TooLarge {
                got: bytes.len(),
                limit: Limits::SCENE_BYTES,
            });
        }
        Ok(postcard::from_bytes(bytes)?)
    }

    /// A slot's tree as `set_lock_slot` carries it.
    pub fn decode_slot(bytes: &[u8]) -> Result<Tree, SceneError> {
        if bytes.len() > Limits::SLOT_BYTES {
            return Err(SceneError::TooLarge {
                got: bytes.len(),
                limit: Limits::SLOT_BYTES,
            });
        }
        Ok(postcard::from_bytes(bytes)?)
    }

    pub fn slot(&self, id: &str) -> Option<&Slot> {
        self.slots.iter().find(|slot| slot.id == id)
    }

    pub fn referenced_assets(&self) -> BTreeSet<AssetId> {
        let mut assets = BTreeSet::new();
        if let LockBackground::Image(src) = &self.config.background
            && let Some(id) = AssetId::from_src(src)
        {
            assets.insert(id);
        }
        let trees = self
            .decoration
            .iter()
            .chain(self.slots.iter().map(|slot| &slot.tree));
        for tree in trees {
            collect_assets(&tree.root, &mut assets);
        }
        assets
    }
}

fn collect_assets(node: &Node, assets: &mut BTreeSet<AssetId>) {
    if let Node::Image { src, .. } = node
        && let Some(id) = AssetId::from_src(src)
    {
        assets.insert(id);
    }
    for child in node.children() {
        collect_assets(child, assets);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_survives_the_wire() {
        let scene = LockScene {
            slots: vec![Slot {
                id: "plugin:a.b/c".into(),
                position: LockSlotPosition::Bottom,
                offset: [1.0, 2.0],
                tree: Tree {
                    rev: 3,
                    root: Node::Image {
                        style: Vec::new(),
                        src: AssetId::of(b"png").to_src(),
                    },
                },
            }],
            ..LockScene::default()
        };
        let bytes = scene.encode().expect("encodes");
        let back = LockScene::decode(&bytes).expect("decodes");
        assert_eq!(back, scene);
        assert_eq!(back.referenced_assets().len(), 1);
        assert!(back.without_slots().referenced_assets().is_empty());
    }

    #[test]
    fn oversized_payloads_are_refused_before_decoding() {
        let bytes = vec![0; Limits::SCENE_BYTES + 1];
        assert!(matches!(
            LockScene::decode(&bytes),
            Err(SceneError::TooLarge { .. })
        ));
    }
}
