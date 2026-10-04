//! What one report says, and what of it a client has not been told yet.

pub use crownos_protocols::surface_visibility::v1::server::crownos_surface_visibility_v1::State as VisibilityState;

/// Sub-unit steps of `wl_fixed`, which is 24.8.
const FIXED_STEPS: f64 = 256.0;

/// A draw scale quantized to `wl_fixed`, so two scales the wire cannot tell
/// apart never count as a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailScale(i32);

impl ThumbnailScale {
    pub const NORMAL: Self = Self(FIXED_STEPS as i32);

    pub fn new(scale: f64) -> Self {
        Self((scale.clamp(0.0, 1.0) * FIXED_STEPS).round() as i32)
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / FIXED_STEPS
    }
}

/// How a surface is shown right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    pub state: VisibilityState,
    pub thumbnail_scale: ThumbnailScale,
}

impl Visibility {
    pub const VISIBLE: Self = Self::shown(VisibilityState::Visible);
    pub const OCCLUDED: Self = Self::shown(VisibilityState::Occluded);
    pub const HIDDEN: Self = Self::shown(VisibilityState::Hidden);

    const fn shown(state: VisibilityState) -> Self {
        Self {
            state,
            thumbnail_scale: ThumbnailScale::NORMAL,
        }
    }

    pub fn thumbnail(scale: f64) -> Self {
        Self {
            state: VisibilityState::Visible,
            thumbnail_scale: ThumbnailScale::new(scale),
        }
    }

    /// The events a client last told `reported` needs; `None` when nothing
    /// changed.
    pub(super) fn changes_since(self, reported: Option<Self>) -> Option<VisibilityChanges> {
        let Some(reported) = reported else {
            return Some(VisibilityChanges {
                state: Some(self.state),
                thumbnail_scale: Some(self.thumbnail_scale),
            });
        };
        let changes = VisibilityChanges {
            state: (reported.state != self.state).then_some(self.state),
            thumbnail_scale: (reported.thumbnail_scale != self.thumbnail_scale)
                .then_some(self.thumbnail_scale),
        };
        (changes.state.is_some() || changes.thumbnail_scale.is_some()).then_some(changes)
    }
}

/// One group of events, ended by `done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VisibilityChanges {
    pub(super) state: Option<VisibilityState>,
    pub(super) thumbnail_scale: Option<ThumbnailScale>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_report_carries_everything() {
        assert_eq!(
            Visibility::VISIBLE.changes_since(None),
            Some(VisibilityChanges {
                state: Some(VisibilityState::Visible),
                thumbnail_scale: Some(ThumbnailScale::NORMAL),
            })
        );
    }

    #[test]
    fn an_unchanged_surface_sends_nothing() {
        assert_eq!(
            Visibility::HIDDEN.changes_since(Some(Visibility::HIDDEN)),
            None
        );
    }

    #[test]
    fn only_the_changed_value_is_resent() {
        assert_eq!(
            Visibility::thumbnail(0.5).changes_since(Some(Visibility::VISIBLE)),
            Some(VisibilityChanges {
                state: None,
                thumbnail_scale: Some(ThumbnailScale::new(0.5)),
            })
        );
        assert_eq!(
            Visibility::OCCLUDED.changes_since(Some(Visibility::VISIBLE)),
            Some(VisibilityChanges {
                state: Some(VisibilityState::Occluded),
                thumbnail_scale: None,
            })
        );
    }

    #[test]
    fn scales_the_wire_cannot_tell_apart_are_not_a_change() {
        let settled = Visibility::thumbnail(0.5);
        assert_eq!(
            Visibility::thumbnail(0.5 + 1.0 / 1024.0).changes_since(Some(settled)),
            None
        );
    }

    #[test]
    fn a_thumbnail_never_reports_growing_past_normal_size() {
        assert_eq!(ThumbnailScale::new(1.3), ThumbnailScale::NORMAL);
        assert_eq!(ThumbnailScale::new(0.25).as_f64(), 0.25);
    }
}
