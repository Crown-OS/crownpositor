//! How many windows each application has and whether one has focus: what a
//! launcher marks its icons with.

use super::same_app;

/// One application's windows, as launchers are told about them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppWindows {
    pub app_id: String,
    pub count: u32,
    pub focused: bool,
}

impl AppWindows {
    /// Counts one window of `app_id` into `apps`, which keeps one entry per
    /// application in order of first appearance.
    pub fn count_into(apps: &mut Vec<Self>, app_id: &str, focused: bool) {
        match apps.iter_mut().find(|app| same_app(&app.app_id, app_id)) {
            Some(app) => {
                app.count += 1;
                app.focused |= focused;
            }
            None => apps.push(Self {
                app_id: app_id.to_owned(),
                count: 1,
                focused,
            }),
        }
    }

    fn gone(app_id: &str) -> Self {
        Self {
            app_id: app_id.to_owned(),
            count: 0,
            focused: false,
        }
    }
}

/// The applications as launchers last heard of them.
#[derive(Debug, Default)]
pub(super) struct Published(Vec<AppWindows>);

impl Published {
    pub(super) fn all(&self) -> &[AppWindows] {
        &self.0
    }

    /// Takes `current` as what launchers know, returning what they have to be
    /// told: every application whose windows changed, and a count of 0 for
    /// each one that has none left.
    pub(super) fn update(&mut self, current: Vec<AppWindows>) -> Vec<AppWindows> {
        if current == self.0 {
            return Vec::new();
        }
        let mut changes: Vec<AppWindows> = self
            .0
            .iter()
            .filter(|old| !current.iter().any(|app| same_app(&app.app_id, &old.app_id)))
            .map(|old| AppWindows::gone(&old.app_id))
            .collect();
        changes.extend(current.iter().filter(|app| !self.0.contains(app)).cloned());
        self.0 = current;
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(app_id: &str, count: u32, focused: bool) -> AppWindows {
        AppWindows {
            app_id: app_id.into(),
            count,
            focused,
        }
    }

    #[test]
    fn windows_of_one_application_are_counted_together_whatever_the_case() {
        let mut apps = Vec::new();
        for (app_id, focused) in [("kitty", false), ("firefox", true), ("Kitty", false)] {
            AppWindows::count_into(&mut apps, app_id, focused);
        }
        assert_eq!(apps, [app("kitty", 2, false), app("firefox", 1, true)]);
    }

    #[test]
    fn only_what_changed_is_published() {
        let mut published = Published::default();
        assert_eq!(
            published.update(vec![app("kitty", 1, true)]),
            [app("kitty", 1, true)]
        );
        assert!(published.update(vec![app("kitty", 1, true)]).is_empty());
        assert_eq!(
            published.update(vec![app("kitty", 1, false), app("firefox", 1, true)]),
            [app("kitty", 1, false), app("firefox", 1, true)]
        );
    }

    #[test]
    fn an_application_without_windows_is_published_with_a_count_of_zero() {
        let mut published = Published::default();
        published.update(vec![app("kitty", 2, false), app("firefox", 1, true)]);
        assert_eq!(
            published.update(vec![app("firefox", 1, true)]),
            [app("kitty", 0, false)]
        );
        assert_eq!(published.all(), [app("firefox", 1, true)]);
    }
}
