//! What the lock screen shows: the settings, plus whatever `crownplugind`
//! handed over — a theme's layout, plugin slots and their images.
//!
//! The settings always come from the compositor's own live config, never
//! from the plugin daemon's copy, so a stale or hostile scene cannot change
//! how the lock behaves — only what its slots look like.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use crownos_config::schema::{Lock, LockSlotPosition, Theme};
use crownplugin_view::Tree;
use lockscreen::lockscene::{
    AssetId, Limits, LockScene, SceneError, ValidScene, compose, validate,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("the asset does not hash to its id")]
    Mismatch,
    #[error("the asset is larger than allowed")]
    TooLarge,
}

#[derive(Default)]
pub struct SceneSource {
    current: ValidScene,
    /// The last scene `crownplugind` sent, already validated.
    plugin: Option<ValidScene>,
    assets: HashMap<AssetId, Arc<[u8]>>,
    asset_bytes: usize,
    slot_updates: HashMap<String, (Instant, u32)>,
}

impl SceneSource {
    pub fn current(&self) -> &ValidScene {
        &self.current
    }

    pub fn asset(&self, id: &AssetId) -> Option<Arc<[u8]>> {
        self.assets.get(id).cloned()
    }

    /// Recomposes from the live settings. Settings that fail validation —
    /// a hand-edited file — fall back to the defaults rather than leaving the
    /// lock without a scene.
    pub fn rebuild(&mut self, config: &Lock, theme: &Theme) {
        let (decoration, slots) = self.plugin.as_ref().map_or((None, Vec::new()), |plugin| {
            let slots = plugin
                .slots
                .iter()
                .map(|slot| (slot.id.clone(), slot.position, slot.tree.clone()))
                .collect::<Vec<(String, LockSlotPosition, Tree)>>();
            (plugin.decoration.clone(), slots)
        });
        let scene = compose(config, theme, decoration.clone(), slots.clone());
        self.current = validate(scene).unwrap_or_else(|err| {
            tracing::warn!(%err, "lock settings refused; drawing the stock lock screen");
            validate(compose(
                &Lock::default(),
                &Theme::default(),
                decoration,
                slots,
            ))
            .unwrap_or_default()
        });
    }

    pub fn set_plugin_scene(&mut self, scene: LockScene) -> Result<(), SceneError> {
        let scene = validate(scene)?;
        let referenced = scene.referenced_assets();
        self.assets.retain(|id, _| referenced.contains(id));
        self.asset_bytes = self.assets.values().map(|bytes| bytes.len()).sum();
        self.slot_updates.clear();
        self.plugin = Some(scene);
        Ok(())
    }

    /// Swaps one slot's tree. `Ok(false)` when it came faster than the
    /// protocol allows and was dropped.
    pub fn set_plugin_slot(&mut self, id: &str, tree: Tree) -> Result<bool, SceneError> {
        let now = Instant::now();
        let (window, count) = self.slot_updates.entry(id.to_owned()).or_insert((now, 0));
        if now.duration_since(*window) >= Duration::from_secs(1) {
            *window = now;
            *count = 0;
        }
        if *count >= Limits::SLOT_UPDATES_PER_SECOND {
            return Ok(false);
        }
        *count += 1;
        let plugin = self
            .plugin
            .as_mut()
            .ok_or_else(|| SceneError::BadPlaceholder(id.to_owned()))?;
        plugin.replace_slot(id, tree)?;
        Ok(true)
    }

    pub fn put_asset(&mut self, id: AssetId, bytes: Vec<u8>) -> Result<(), AssetError> {
        if bytes.len() > Limits::ASSET_BYTES {
            return Err(AssetError::TooLarge);
        }
        if !id.matches(&bytes) {
            return Err(AssetError::Mismatch);
        }
        if self.assets.contains_key(&id) {
            return Ok(());
        }
        if self.asset_bytes + bytes.len() > Limits::ASSETS_TOTAL_BYTES {
            return Err(AssetError::TooLarge);
        }
        self.asset_bytes += bytes.len();
        self.assets.insert(id, bytes.into());
        Ok(())
    }

    /// Forgets everything the plugin daemon sent, when it disconnects.
    pub fn clear_plugin(&mut self) {
        self.plugin = None;
        self.assets.clear();
        self.asset_bytes = 0;
        self.slot_updates.clear();
    }
}

#[cfg(test)]
mod tests {
    use crownplugin_view::Node;
    use lockscreen::lockscene::Slot;

    use super::*;

    fn text_tree(text: &str) -> Tree {
        Tree {
            rev: 0,
            root: Node::Text {
                style: Vec::new(),
                text: text.into(),
            },
        }
    }

    fn plugin_scene() -> LockScene {
        LockScene {
            config: Lock {
                dim: 0.9,
                ..Lock::default()
            },
            slots: vec![Slot {
                id: "plugin:a.b/c".into(),
                position: LockSlotPosition::Bottom,
                offset: [0.0; 2],
                tree: text_tree("hi"),
            }],
            ..LockScene::default()
        }
    }

    #[test]
    fn the_plugin_scene_brings_slots_but_not_settings() {
        let mut source = SceneSource::default();
        source.set_plugin_scene(plugin_scene()).expect("valid");
        source.rebuild(&Lock::default(), &Theme::default());
        assert_eq!(source.current().config.dim, Lock::default().dim);
        assert_eq!(source.current().slots.len(), 1);
    }

    #[test]
    fn broken_settings_fall_back_to_the_defaults() {
        let mut source = SceneSource::default();
        let broken = Lock {
            dim: 7.0,
            ..Lock::default()
        };
        source.rebuild(&broken, &Theme::default());
        assert_eq!(source.current().config, Lock::default());
    }

    #[test]
    fn assets_must_hash_to_their_id_and_fit() {
        let mut source = SceneSource::default();
        let id = AssetId::of(b"png");
        assert!(matches!(
            source.put_asset(id.clone(), b"jpg".to_vec()),
            Err(AssetError::Mismatch)
        ));
        source.put_asset(id.clone(), b"png".to_vec()).expect("fits");
        assert!(source.asset(&id).is_some());
        source.set_plugin_scene(plugin_scene()).expect("valid");
        assert!(
            source.asset(&id).is_none(),
            "unreferenced assets are dropped"
        );
    }

    #[test]
    fn slot_updates_are_rate_limited() {
        let mut source = SceneSource::default();
        source.set_plugin_scene(plugin_scene()).expect("valid");
        let accepted = (0..Limits::SLOT_UPDATES_PER_SECOND + 5)
            .filter(|_| {
                source
                    .set_plugin_slot("plugin:a.b/c", text_tree("x"))
                    .expect("known slot")
            })
            .count();
        assert_eq!(accepted, Limits::SLOT_UPDATES_PER_SECOND as usize);
        assert!(
            source
                .set_plugin_slot("plugin:x.y/z", text_tree("x"))
                .is_err()
        );
    }
}
