use std::time::{Duration, Instant};

use super::same_app;

/// How long after a launch the application's first window still counts as
/// the one launched: long enough for a cold start of a large application.
const LAUNCH_WINDOW: Duration = Duration::from_secs(15);

/// Applications launched from their icon whose first window has not mapped,
/// each with what the compositor kept of where it was launched from.
#[derive(Debug)]
pub struct Launches<T>(Vec<(String, Instant, T)>);

impl<T> Default for Launches<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> Launches<T> {
    pub fn arm(&mut self, app_id: String, from: T) {
        self.expire();
        self.0.retain(|(armed, _, _)| *armed != app_id);
        self.0.push((app_id, Instant::now(), from));
    }

    /// Where the application `window` belongs to was launched from, if it
    /// was moments ago. Consumes the launch, so only its first window has
    /// one.
    pub fn take(&mut self, window: &str) -> Option<T> {
        self.expire();
        let index = self
            .0
            .iter()
            .position(|(icon, _, _)| same_app(icon, window))?;
        Some(self.0.swap_remove(index).2)
    }

    fn expire(&mut self) {
        self.0.retain(|(_, at, _)| at.elapsed() < LAUNCH_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_is_taken_once() {
        let mut launches = Launches::default();
        launches.arm("org.gnome.Nautilus".into(), 7);
        assert_eq!(launches.take("nautilus"), Some(7));
        assert_eq!(launches.take("nautilus"), None);
    }

    #[test]
    fn rearming_does_not_duplicate() {
        let mut launches = Launches::default();
        launches.arm("kitty".into(), 1);
        launches.arm("kitty".into(), 2);
        assert_eq!(launches.take("kitty"), Some(2));
        assert_eq!(launches.take("kitty"), None);
    }
}
