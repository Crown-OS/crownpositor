use std::time::{Duration, Instant};

use super::same_app;

/// How long after a launch the application's first window still counts as
/// the one launched: long enough for a cold start of a large application.
const LAUNCH_WINDOW: Duration = Duration::from_secs(15);

/// Applications launched from their icon whose first window has not mapped.
#[derive(Debug, Default)]
pub(super) struct Launches(Vec<(String, Instant)>);

impl Launches {
    pub(super) fn arm(&mut self, app_id: String) {
        self.expire();
        self.0.retain(|(armed, _)| *armed != app_id);
        self.0.push((app_id, Instant::now()));
    }

    pub(super) fn take(&mut self, window: &str) -> bool {
        self.expire();
        let Some(index) = self.0.iter().position(|(icon, _)| same_app(icon, window)) else {
            return false;
        };
        self.0.swap_remove(index);
        true
    }

    fn expire(&mut self) {
        self.0.retain(|(_, at)| at.elapsed() < LAUNCH_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_is_taken_once() {
        let mut launches = Launches::default();
        launches.arm("org.gnome.Nautilus".into());
        assert!(launches.take("nautilus"));
        assert!(!launches.take("nautilus"));
    }

    #[test]
    fn rearming_does_not_duplicate() {
        let mut launches = Launches::default();
        launches.arm("kitty".into());
        launches.arm("kitty".into());
        assert!(launches.take("kitty"));
        assert!(!launches.take("kitty"));
    }
}
