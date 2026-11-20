//! Which windows had the keyboard, across every monitor, so the Alt+Tab stack
//! can list windows from several workspaces most recent first.

use crate::utils::id::WindowId;

/// Every window that had the keyboard, oldest first, each once.
#[derive(Debug, Default)]
pub struct FocusHistory(Vec<WindowId>);

impl FocusHistory {
    pub fn touch(&mut self, window: WindowId) {
        self.forget(window);
        self.0.push(window);
    }

    pub fn forget(&mut self, window: WindowId) {
        self.0.retain(|known| *known != window);
    }

    /// The windows of `workspaces`, each given in its own most-recent-first
    /// order: `current` first, then the ones this history knows, newest first,
    /// then the rest in workspace order.
    pub fn most_recent_first(
        &self,
        current: Option<WindowId>,
        workspaces: &[Vec<WindowId>],
    ) -> Vec<WindowId> {
        let listed: Vec<WindowId> = workspaces.iter().flatten().copied().collect();
        let mut ranked = Vec::with_capacity(listed.len());
        let candidates = current
            .into_iter()
            .chain(self.0.iter().rev().copied())
            .chain(listed.iter().copied());
        for window in candidates {
            if listed.contains(&window) && !ranked.contains(&window) {
                ranked.push(window);
            }
        }
        ranked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows<const COUNT: usize>() -> [WindowId; COUNT] {
        std::array::from_fn(|_| WindowId::next())
    }

    fn history(touched: &[WindowId]) -> FocusHistory {
        let mut history = FocusHistory::default();
        for window in touched {
            history.touch(*window);
        }
        history
    }

    #[test]
    fn the_newest_touch_comes_first_across_workspaces() {
        let [a, b, c, d] = windows();
        let history = history(&[a, c, b, d]);

        assert_eq!(
            history.most_recent_first(None, &[vec![a, b], vec![c, d]]),
            [d, b, c, a]
        );
    }

    #[test]
    fn windows_on_other_workspaces_are_left_out() {
        let [a, b, elsewhere] = windows();
        let history = history(&[a, elsewhere, b]);

        assert_eq!(history.most_recent_first(None, &[vec![a, b]]), [b, a]);
    }

    #[test]
    fn windows_that_never_had_the_keyboard_follow_in_workspace_order() {
        let [a, b, c, d] = windows();
        let history = history(&[c]);

        assert_eq!(
            history.most_recent_first(None, &[vec![a, b], vec![c, d]]),
            [c, a, b, d]
        );
    }

    #[test]
    fn the_current_window_leads_whatever_the_history_says() {
        let [a, b, c] = windows();
        let history = history(&[a, b, c]);

        assert_eq!(
            history.most_recent_first(Some(a), &[vec![a], vec![b, c]]),
            [a, c, b]
        );
    }

    #[test]
    fn a_forgotten_window_loses_its_rank() {
        let [a, b] = windows();
        let mut history = history(&[a, b]);
        history.forget(b);

        assert_eq!(history.most_recent_first(None, &[vec![b, a]]), [a, b]);
    }
}
