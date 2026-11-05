use std::time::{Duration, Instant};

/// How long a placeholder waits for its app: the protocol's 120 seconds,
/// long enough for a cold Android boot.
pub const PLACEHOLDER_LIFETIME: Duration = Duration::from_secs(120);

/// Splash windows standing in for Android apps that are still starting, each
/// with the app_id of the window it waits for and when it was marked.
#[derive(Debug)]
pub struct Placeholders<T>(Vec<(String, Instant, T)>);

impl<T> Default for Placeholders<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> Placeholders<T> {
    /// Marks `placeholder` as waiting for `app_id`, replacing an older mark
    /// for the same app.
    pub fn mark(&mut self, app_id: String, placeholder: T, now: Instant) {
        self.expire(now);
        self.0.retain(|(waiting_for, _, _)| *waiting_for != app_id);
        self.0.push((app_id, now, placeholder));
    }

    /// The placeholder waiting for `app_id`, if it was marked less than
    /// [`PLACEHOLDER_LIFETIME`] ago. Consumes the mark, so only the app's
    /// first window takes its place.
    pub fn take(&mut self, app_id: &str, now: Instant) -> Option<T> {
        self.expire(now);
        let index = self
            .0
            .iter()
            .position(|(waiting_for, _, _)| waiting_for == app_id)?;
        Some(self.0.swap_remove(index).2)
    }

    fn expire(&mut self, now: Instant) {
        self.0
            .retain(|(_, marked, _)| now.saturating_duration_since(*marked) < PLACEHOLDER_LIFETIME);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "waydroid.org.example.notes";

    #[test]
    fn the_first_window_with_the_same_app_id_takes_the_place() {
        let now = Instant::now();
        let mut placeholders = Placeholders::default();
        placeholders.mark(APP.into(), 1, now);
        assert_eq!(placeholders.take("waydroid.org.example", now), None);
        assert_eq!(placeholders.take(APP, now), Some(1));
        assert_eq!(placeholders.take(APP, now), None, "taken once");
    }

    #[test]
    fn app_ids_match_exactly() {
        let now = Instant::now();
        let mut placeholders = Placeholders::default();
        placeholders.mark(APP.into(), 1, now);
        assert_eq!(placeholders.take("notes", now), None);
        assert_eq!(placeholders.take("WAYDROID.ORG.EXAMPLE.NOTES", now), None);
    }

    #[test]
    fn a_mark_expires_after_its_lifetime() {
        let marked = Instant::now();
        let mut placeholders = Placeholders::default();
        placeholders.mark(APP.into(), 1, marked);
        let almost = marked + PLACEHOLDER_LIFETIME - Duration::from_millis(1);
        assert_eq!(placeholders.take(APP, almost), Some(1));

        placeholders.mark(APP.into(), 2, marked);
        assert_eq!(placeholders.take(APP, marked + PLACEHOLDER_LIFETIME), None);
    }

    #[test]
    fn marking_again_replaces_the_older_placeholder() {
        let now = Instant::now();
        let mut placeholders = Placeholders::default();
        placeholders.mark(APP.into(), 1, now);
        placeholders.mark(APP.into(), 2, now);
        assert_eq!(placeholders.take(APP, now), Some(2));
        assert_eq!(placeholders.take(APP, now), None);
    }
}
