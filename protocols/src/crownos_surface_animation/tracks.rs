//! Every animatable property of one surface, and the events their motion
//! produces.

use std::time::Duration;

use smithay::utils::Point;

use super::{
    pending::{PendingMotion, PropertyChange},
    property::{PROPERTIES, Property, index, is_rendered, resting_value},
    track::{Interruption, Track},
    transform::SurfaceTransform,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionEvent {
    Settled {
        property: Property,
        value: f32,
    },
    Interrupted {
        property: Property,
        value: f32,
        velocity: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tracks([Track; PROPERTIES.len()]);

impl Default for Tracks {
    fn default() -> Self {
        Self(PROPERTIES.map(|property| Track::resting(resting_value(property))))
    }
}

impl Tracks {
    /// Applies one commit's worth of requests. `now` stands in for a start time
    /// the client left at zero.
    pub fn apply(
        &mut self,
        pending: &PendingMotion,
        now: Duration,
        emit: &mut impl FnMut(MotionEvent),
    ) {
        if pending.is_reset() {
            *self = Self::default();
        }
        for (property, change) in pending.changes() {
            let track = &mut self.0[index(property)];
            match change {
                PropertyChange::Set(value) => track.set(value),
                PropertyChange::Animate { target, .. } if !is_rendered(property) => {
                    track.set(target);
                    emit(MotionEvent::Settled {
                        property,
                        value: target,
                    });
                }
                PropertyChange::Animate {
                    target,
                    spring,
                    start,
                } => {
                    if let Some(Interruption { value, velocity }) =
                        track.animate(target, spring, start.unwrap_or(now))
                    {
                        emit(MotionEvent::Interrupted {
                            property,
                            value,
                            velocity,
                        });
                    }
                }
            }
        }
    }

    pub fn advance(&mut self, now: Duration, emit: &mut impl FnMut(MotionEvent)) {
        for (property, track) in PROPERTIES.into_iter().zip(&mut self.0) {
            if let Some(value) = track.advance(now) {
                emit(MotionEvent::Settled { property, value });
            }
        }
    }

    pub fn is_moving(&self) -> bool {
        self.0.iter().any(Track::is_moving)
    }

    pub fn transform(&self) -> SurfaceTransform {
        let value = |property| f64::from(self.0[index(property)].value());
        SurfaceTransform {
            translation: Point::new(value(Property::TranslateX), value(Property::TranslateY)),
            scale: value(Property::Scale).max(0.0),
            opacity: self.0[index(Property::Opacity)].value().clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use motion::{Animation, Spring};

    use super::*;

    const START: Duration = Duration::from_secs(50);

    fn animate(property: Property, target: f32, start: Option<Duration>) -> PendingMotion {
        let mut pending = PendingMotion::default();
        pending.change(
            property,
            PropertyChange::Animate {
                target,
                spring: Spring::snappy(),
                start,
            },
        );
        pending
    }

    fn apply(tracks: &mut Tracks, pending: &PendingMotion, now: Duration) -> Vec<MotionEvent> {
        let mut events = Vec::new();
        tracks.apply(pending, now, &mut |event| events.push(event));
        events
    }

    fn frames(tracks: &mut Tracks, until: Duration) -> Vec<MotionEvent> {
        let mut events = Vec::new();
        let mut now = START;
        while now <= until {
            tracks.advance(now, &mut |event| events.push(event));
            now += Duration::from_micros(16_667);
        }
        events
    }

    #[test]
    fn nothing_applied_is_the_identity() {
        assert!(Tracks::default().transform().is_identity());
        assert!(!Tracks::default().is_moving());
    }

    #[test]
    fn an_animation_settles_exactly_once_at_its_target() {
        let mut tracks = Tracks::default();
        let applied = apply(
            &mut tracks,
            &animate(Property::TranslateX, 120.0, Some(START)),
            START,
        );
        assert!(applied.is_empty());
        assert!(tracks.is_moving());

        let events = frames(&mut tracks, START + Duration::from_secs(3));

        assert_eq!(
            events,
            vec![MotionEvent::Settled {
                property: Property::TranslateX,
                value: 120.0,
            }]
        );
        assert!(!tracks.is_moving());
        assert_eq!(tracks.transform().translation, Point::new(120.0, 0.0));
    }

    #[test]
    fn retargeting_reports_and_continues_from_the_handoff_state() {
        let mut tracks = Tracks::default();
        apply(
            &mut tracks,
            &animate(Property::Scale, 2.0, Some(START)),
            START,
        );
        let switch = START + Duration::from_millis(60);
        let expected =
            Animation::new(1.0_f32, 2.0, 0.0, START, Spring::snappy().into()).sample(switch);

        let events = apply(
            &mut tracks,
            &animate(Property::Scale, 0.5, Some(switch)),
            switch,
        );

        assert_eq!(
            events,
            vec![MotionEvent::Interrupted {
                property: Property::Scale,
                value: expected.value,
                velocity: expected.velocity,
            }]
        );
        tracks.advance(switch, &mut |_| {});
        assert_eq!(tracks.transform().scale, f64::from(expected.value));
    }

    #[test]
    fn a_zero_start_time_starts_at_the_commit() {
        let mut tracks = Tracks::default();
        let commit = START + Duration::from_millis(500);
        apply(
            &mut tracks,
            &animate(Property::TranslateY, 10.0, None),
            commit,
        );
        tracks.advance(commit, &mut |_| {});
        assert_eq!(tracks.transform().translation.y, 0.0);
    }

    #[test]
    fn rotation_is_accepted_but_settles_immediately() {
        let mut tracks = Tracks::default();
        let events = apply(
            &mut tracks,
            &animate(Property::Rotation, 90.0, Some(START)),
            START,
        );
        assert_eq!(
            events,
            vec![MotionEvent::Settled {
                property: Property::Rotation,
                value: 90.0,
            }]
        );
        assert!(!tracks.is_moving());
    }

    #[test]
    fn a_reset_returns_every_property_to_rest() {
        let mut tracks = Tracks::default();
        apply(
            &mut tracks,
            &animate(Property::Opacity, 0.0, Some(START)),
            START,
        );
        let mut reset = PendingMotion::default();
        reset.reset();
        apply(&mut tracks, &reset, START);
        assert_eq!(tracks, Tracks::default());
    }

    #[test]
    fn opacity_and_scale_are_clamped_for_the_renderer() {
        let mut tracks = Tracks::default();
        let mut pending = PendingMotion::default();
        pending.change(Property::Opacity, PropertyChange::Set(1.5));
        pending.change(Property::Scale, PropertyChange::Set(-0.2));
        apply(&mut tracks, &pending, START);
        let transform = tracks.transform();
        assert_eq!(transform.opacity, 1.0);
        assert_eq!(transform.scale, 0.0);
    }
}
