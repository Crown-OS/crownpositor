//! Delegation glue for `crownos_plugin_host_v1`: the protocol lives in
//! [`protocols::crownos_plugin_host`], the lock scene in
//! [`crate::state::lock_screen`].

use crownos_protocols::plugin_host::v1::server::crownos_plugin_host_v1::CrownosPluginHostV1;
use lockscreen::lockscene::{AssetId, LockScene};
use protocols::crownos_plugin_host::{
    ActionOutcome, PluginHostGlobalData, PluginHostHandler, PluginHostState, RequestError,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{
    input::shortcuts::Action,
    state::{State, lock_screen::AssetError},
};

impl PluginHostHandler for State {
    fn plugin_host_state(&mut self) -> &mut PluginHostState {
        &mut self.wayland.plugin_host_state
    }

    /// Nothing runs while locked, and nothing that leaves the session or
    /// starts a program runs ever.
    fn run_plugin_action(&mut self, action: &str) -> ActionOutcome {
        if self.shell.is_locked() {
            return ActionOutcome::Refused;
        }
        let Ok(action) = action.parse::<Action>() else {
            return ActionOutcome::Invalid;
        };
        if matches!(
            action,
            Action::Quit | Action::SwitchVt(_) | Action::Spawn(_)
        ) {
            return ActionOutcome::Refused;
        }
        self.handle_action(action);
        ActionOutcome::Done
    }

    fn set_lock_scene(&mut self, bytes: Vec<u8>) -> Result<(), RequestError> {
        let scene = LockScene::decode(&bytes).map_err(invalid)?;
        self.lock.scene.set_plugin_scene(scene).map_err(invalid)?;
        self.lock_content_changed();
        Ok(())
    }

    fn set_lock_slot(&mut self, id: &str, bytes: Vec<u8>) -> Result<(), RequestError> {
        let tree = LockScene::decode_slot(&bytes).map_err(invalid)?;
        if self.lock.scene.set_plugin_slot(id, tree).map_err(invalid)? {
            self.lock_content_changed();
        }
        Ok(())
    }

    fn put_lock_asset(&mut self, id: &str, bytes: Vec<u8>) -> Result<(), RequestError> {
        let id = AssetId::parse(id)
            .ok_or_else(|| RequestError::Invalid(format!("{id:?} is not an asset id")))?;
        self.lock
            .scene
            .put_asset(id, bytes)
            .map_err(|err| match err {
                AssetError::TooLarge => RequestError::TooLarge,
                AssetError::Mismatch => invalid(err),
            })
    }

    fn plugin_host_gone(&mut self) {
        self.lock.scene.clear_plugin();
        self.lock_content_changed();
    }
}

fn invalid(error: impl std::fmt::Display) -> RequestError {
    RequestError::Invalid(error.to_string())
}

delegate_global_dispatch!(State: [CrownosPluginHostV1: PluginHostGlobalData] => PluginHostState);
delegate_dispatch!(State: [CrownosPluginHostV1: ()] => PluginHostState);
