//! The configuration transaction: what a client has asked for, before the
//! compositor is told about it.
//!
//! A configuration accumulates in the resources' own user data rather than in
//! [`OutputManagementState`](super::OutputManagementState), because several
//! configurations can be in flight at once and each carries the serial it was
//! built against. Nothing is handed to the compositor until `apply` or `test`,
//! and by then the whole thing has been checked against the current head list.

use std::sync::Mutex;

use smithay::utils::{Logical, Physical, Point, Size, Transform};
use wayland_protocols_wlr::output_management::v1::server::zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1;

use super::HeadId;

/// Which mode a head should end up in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeRequest {
    /// Index into the head's advertised mode list.
    Advertised(usize),
    /// A mode the client made up. `refresh` is millihertz, or `None` when the
    /// client did not care.
    Custom {
        size: Size<i32, Physical>,
        refresh: Option<i32>,
    },
}

/// What one head should look like once the configuration is applied.
///
/// Every field of `Enabled` is optional because the protocol lets a client
/// enable a head without saying anything else about it, which means "keep what
/// it has". Resolving that against the current state is the compositor's job,
/// not this module's.
#[derive(Debug, Clone, PartialEq)]
pub enum HeadConfig {
    Disabled,
    Enabled {
        mode: Option<ModeRequest>,
        position: Option<Point<i32, Logical>>,
        transform: Option<Transform>,
        scale: Option<f64>,
        adaptive_sync: Option<bool>,
    },
}

/// A whole configuration, as handed to the compositor.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputConfigRequest {
    pub heads: Vec<(HeadId, HeadConfig)>,
}

impl OutputConfigRequest {
    pub fn get(&self, id: &HeadId) -> Option<&HeadConfig> {
        self.heads
            .iter()
            .find_map(|(head, config)| (head == id).then_some(config))
    }

    /// Whether every head would end up off, which no compositor should do to
    /// a user by accident.
    pub fn is_all_disabled(&self) -> bool {
        !self.heads.is_empty()
            && self
                .heads
                .iter()
                .all(|(_, config)| *config == HeadConfig::Disabled)
    }
}

/// User data of a `zwlr_output_configuration_v1`.
pub type PendingConfiguration = Mutex<PendingConfigurationInner>;

#[derive(Debug)]
pub struct PendingConfigurationInner {
    /// The serial the client built this against. A mismatch at any point means
    /// it was describing monitors that have since changed.
    pub serial: u32,
    /// Set by the first `apply` or `test`; any later request is a protocol
    /// error rather than a second attempt.
    pub used: bool,
    /// `None` for a head the client disabled.
    pub heads: Vec<(HeadId, Option<ZwlrOutputConfigurationHeadV1>)>,
}

impl PendingConfigurationInner {
    pub fn new(serial: u32) -> Self {
        Self {
            serial,
            used: false,
            heads: Vec::new(),
        }
    }

    pub fn is_configured(&self, id: &HeadId) -> bool {
        self.heads.iter().any(|(head, _)| head == id)
    }
}

/// User data of a `zwlr_output_configuration_head_v1`.
pub type PendingHeadConfiguration = Mutex<PendingHeadConfigurationInner>;

#[derive(Debug)]
pub struct PendingHeadConfigurationInner {
    pub head: HeadId,
    pub mode: Option<ModeRequest>,
    pub position: Option<Point<i32, Logical>>,
    pub transform: Option<Transform>,
    pub scale: Option<f64>,
    pub adaptive_sync: Option<bool>,
}

impl PendingHeadConfigurationInner {
    pub fn new(head: HeadId) -> Self {
        Self {
            head,
            mode: None,
            position: None,
            transform: None,
            scale: None,
            adaptive_sync: None,
        }
    }

    pub fn to_config(&self) -> HeadConfig {
        HeadConfig::Enabled {
            mode: self.mode,
            position: self.position,
            transform: self.transform,
            scale: self.scale,
            adaptive_sync: self.adaptive_sync,
        }
    }
}

/// Locks a pending object's state, treating a poisoned mutex as usable.
///
/// A panic in one client's dispatch must not make every later request on that
/// object fail — the data behind the lock is plain values with no invariant a
/// partial write could break.
pub fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(configs: &[(&str, HeadConfig)]) -> OutputConfigRequest {
        OutputConfigRequest {
            heads: configs
                .iter()
                .map(|(name, config)| (HeadId::new(*name), config.clone()))
                .collect(),
        }
    }

    fn enabled() -> HeadConfig {
        HeadConfig::Enabled {
            mode: None,
            position: None,
            transform: None,
            scale: None,
            adaptive_sync: None,
        }
    }

    #[test]
    fn a_configuration_that_turns_everything_off_is_recognised() {
        assert!(request(&[("DP-1", HeadConfig::Disabled)]).is_all_disabled());
        assert!(
            !request(&[("DP-1", HeadConfig::Disabled), ("DP-2", enabled())]).is_all_disabled()
        );
    }

    #[test]
    fn an_empty_configuration_is_not_all_disabled() {
        assert!(!request(&[]).is_all_disabled());
    }

    #[test]
    fn heads_are_looked_up_by_id() {
        let request = request(&[("DP-1", enabled()), ("DP-2", HeadConfig::Disabled)]);
        assert_eq!(request.get(&HeadId::new("DP-2")), Some(&HeadConfig::Disabled));
        assert_eq!(request.get(&HeadId::new("HDMI-A-1")), None);
    }
}
