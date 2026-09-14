//! Per-client head and mode objects, and the diff that keeps them current.
//!
//! Every bound manager gets its own `zwlr_output_head_v1` per head and its own
//! `zwlr_output_mode_v1` per mode, so all of this is per-instance bookkeeping.
//! The interesting part is [`apply_diff`], which turns "here is the new state"
//! into the minimum set of events: the protocol has no way to say "everything
//! changed", and re-sending unchanged properties makes well-written clients
//! rebuild their UI on every unrelated hotplug.

use smithay::utils::Transform;
use wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::ZwlrOutputManagerV1,
    zwlr_output_mode_v1::ZwlrOutputModeV1,
};
use wayland_server::{Dispatch, DisplayHandle, Resource};

use super::{AdaptiveSync, HeadId, HeadSnapshot, ModeSnapshot};

/// User data of a `zwlr_output_head_v1`.
#[derive(Debug)]
pub struct HeadData {
    pub id: HeadId,
}

/// User data of a `zwlr_output_mode_v1`.
///
/// Carries the mode itself so `set_mode` need not index back through a
/// snapshot that may already have moved on, and so a mode object that outlived
/// its head is recognisable rather than silently resolving to the wrong mode.
#[derive(Debug)]
pub struct ModeData {
    pub head: HeadId,
    pub index: usize,
    pub mode: ModeSnapshot,
}

pub struct ManagerInstance {
    pub obj: ZwlrOutputManagerV1,
    pub heads: Vec<HeadInstance>,
}

pub struct HeadInstance {
    pub obj: ZwlrOutputHeadV1,
    pub id: HeadId,
    pub modes: Vec<ZwlrOutputModeV1>,
}

/// Version gates, straight from the protocol XML.
mod since {
    /// `make`, `model`, `serial_number`.
    pub const IDENTITY: u32 = 2;
    /// `zwlr_output_head_v1.release` and `zwlr_output_mode_v1.release`.
    pub const RELEASE: u32 = 3;
    /// `adaptive_sync`.
    pub const ADAPTIVE_SYNC: u32 = 4;
}

/// The dispatch bounds every function here needs, named once.
pub trait HeadDispatch:
    Dispatch<ZwlrOutputHeadV1, HeadData> + Dispatch<ZwlrOutputModeV1, ModeData> + 'static
{
}

impl<D> HeadDispatch for D where
    D: Dispatch<ZwlrOutputHeadV1, HeadData> + Dispatch<ZwlrOutputModeV1, ModeData> + 'static
{
}

/// Creates every head object for a manager that has just been bound.
pub fn send_all<D: HeadDispatch>(
    dh: &DisplayHandle,
    instance: &mut ManagerInstance,
    heads: &[HeadSnapshot],
) {
    for snapshot in heads {
        if let Some(head) = create_head::<D>(dh, &instance.obj, snapshot) {
            instance.heads.push(head);
        }
    }
}

/// Brings every instance from `old` to `new`.
pub fn apply_diff<D: HeadDispatch>(
    dh: &DisplayHandle,
    instances: &mut [ManagerInstance],
    old: &[HeadSnapshot],
    new: &[HeadSnapshot],
) {
    for instance in instances {
        // A head that is gone is finished first, so its id is free before an
        // identically named head could be created for a replug.
        instance.heads.retain_mut(|head| {
            if new.iter().any(|snapshot| snapshot.id == head.id) {
                return true;
            }
            finish_head(head);
            // Below v3 the client cannot tell us it is done with the object,
            // so there is no release to wait for and the entry goes now.
            head.obj.version() >= since::RELEASE
        });

        for snapshot in new {
            match instance
                .heads
                .iter_mut()
                .find(|head| head.id == snapshot.id)
            {
                Some(head) => {
                    let previous = old.iter().find(|previous| previous.id == snapshot.id);
                    update_head::<D>(dh, head, previous, snapshot);
                }
                None => {
                    if let Some(head) = create_head::<D>(dh, &instance.obj, snapshot) {
                        instance.heads.push(head);
                    }
                }
            }
        }
    }
}

fn create_head<D: HeadDispatch>(
    dh: &DisplayHandle,
    manager: &ZwlrOutputManagerV1,
    snapshot: &HeadSnapshot,
) -> Option<HeadInstance> {
    let client = manager.client()?;
    let version = manager.version();
    let obj = client
        .create_resource::<ZwlrOutputHeadV1, _, D>(
            dh,
            version,
            HeadData {
                id: snapshot.id.clone(),
            },
        )
        .ok()?;
    manager.head(&obj);

    obj.name(snapshot.id.as_str().to_owned());
    obj.description(snapshot.description.clone());
    obj.physical_size(snapshot.physical_size_mm.w, snapshot.physical_size_mm.h);

    if version >= since::IDENTITY {
        // Only what the panel actually told us. A client has to be able to
        // distinguish "no EDID" from a monitor whose make really is "Unknown".
        if let Some(make) = &snapshot.make {
            obj.make(make.clone());
        }
        if let Some(model) = &snapshot.model {
            obj.model(model.clone());
        }
        if let Some(serial) = &snapshot.serial {
            obj.serial_number(serial.clone());
        }
    }

    let mut head = HeadInstance {
        obj,
        id: snapshot.id.clone(),
        modes: Vec::new(),
    };

    for (index, mode) in snapshot.modes.iter().enumerate() {
        if let Some(obj) = create_mode::<D>(dh, &head, snapshot, index, *mode) {
            head.modes.push(obj);
        }
    }

    send_state(&head, snapshot);
    send_adaptive_sync(&head, snapshot);
    Some(head)
}

fn create_mode<D: HeadDispatch>(
    dh: &DisplayHandle,
    head: &HeadInstance,
    snapshot: &HeadSnapshot,
    index: usize,
    mode: ModeSnapshot,
) -> Option<ZwlrOutputModeV1> {
    let client = head.obj.client()?;
    let obj = client
        .create_resource::<ZwlrOutputModeV1, _, D>(
            dh,
            head.obj.version(),
            ModeData {
                head: snapshot.id.clone(),
                index,
                mode,
            },
        )
        .ok()?;

    head.obj.mode(&obj);
    obj.size(mode.size.w, mode.size.h);
    obj.refresh(mode.refresh);
    if mode.preferred {
        obj.preferred();
    }
    Some(obj)
}

fn update_head<D: HeadDispatch>(
    dh: &DisplayHandle,
    head: &mut HeadInstance,
    previous: Option<&HeadSnapshot>,
    snapshot: &HeadSnapshot,
) {
    let previous = match previous {
        Some(previous) if previous == snapshot => return,
        other => other,
    };

    if previous.is_none_or(|previous| previous.description != snapshot.description) {
        head.obj.description(snapshot.description.clone());
    }

    let modes_changed =
        previous.is_none_or(|previous| previous.modes != snapshot.modes);
    if modes_changed {
        rebuild_modes::<D>(dh, head, snapshot);
    }

    let state_changed = previous.is_none_or(|previous| {
        previous.enabled != snapshot.enabled
            || previous.current_mode != snapshot.current_mode
            || previous.position != snapshot.position
            || previous.transform != snapshot.transform
            || previous.scale != snapshot.scale
    });
    // A rebuilt mode list invalidates the object `current_mode` last named, so
    // the state has to be re-sent even when the values themselves are equal.
    if state_changed || modes_changed {
        send_state(head, snapshot);
    }

    if previous.is_none_or(|previous| previous.adaptive_sync != snapshot.adaptive_sync) {
        send_adaptive_sync(head, snapshot);
    }
}

/// Reconciles the mode objects with the snapshot, in place.
///
/// Existing objects are updated rather than replaced wherever the list lines
/// up, because a client may be holding one inside a configuration it has not
/// applied yet; only the tail that genuinely disappeared is finished.
fn rebuild_modes<D: HeadDispatch>(
    dh: &DisplayHandle,
    head: &mut HeadInstance,
    snapshot: &HeadSnapshot,
) {
    for (obj, mode) in head.modes.iter().zip(&snapshot.modes) {
        obj.size(mode.size.w, mode.size.h);
        obj.refresh(mode.refresh);
        if mode.preferred {
            obj.preferred();
        }
    }

    for obj in head.modes.drain(snapshot.modes.len().min(head.modes.len())..) {
        obj.finished();
    }

    for index in head.modes.len()..snapshot.modes.len() {
        let Some(mode) = snapshot.modes.get(index).copied() else {
            break;
        };
        if let Some(obj) = create_mode::<D>(dh, head, snapshot, index, mode) {
            head.modes.push(obj);
        }
    }
}

/// The properties that are only meaningful while the head is on.
fn send_state(head: &HeadInstance, snapshot: &HeadSnapshot) {
    head.obj.enabled(snapshot.enabled as i32);
    if !snapshot.enabled {
        return;
    }

    if let Some(mode) = snapshot.current_mode.and_then(|index| head.modes.get(index)) {
        head.obj.current_mode(mode);
    }
    head.obj.position(snapshot.position.x, snapshot.position.y);
    head.obj.transform(Transform::into(snapshot.transform));
    head.obj.scale(snapshot.scale);
}

fn send_adaptive_sync(head: &HeadInstance, snapshot: &HeadSnapshot) {
    if head.obj.version() < since::ADAPTIVE_SYNC || !snapshot.enabled {
        return;
    }

    head.obj.adaptive_sync(match snapshot.adaptive_sync {
        AdaptiveSync::Enabled => zwlr_output_head_v1::AdaptiveSyncState::Enabled,
        AdaptiveSync::Disabled => zwlr_output_head_v1::AdaptiveSyncState::Disabled,
    });
}

/// Finishes a head and every mode on it, in that order.
fn finish_head(head: &mut HeadInstance) {
    for mode in head.modes.drain(..) {
        mode.finished();
    }
    head.obj.finished();
}
