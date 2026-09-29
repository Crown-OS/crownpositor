//! What the content of an output asks of the display: a variable refresh
//! rate, and whether frames may tear.

use config::{GamingOptions, Vrr};

use crate::{
    shell::{Shell, monitor::Monitor, tile::Tile},
    utils::surface::is_game_content,
};

use smithay::utils::{Logical, Point, Rectangle, Size};

/// The display behaviour one output's content wants this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayDemand {
    pub vrr: bool,
    pub tearing: bool,
}

/// The window that alone covers `monitor`, once nothing is moving: no
/// overview, no workspace slide, and its own enter animation finished.
pub fn scanout_tile(monitor: &Monitor) -> Option<&Tile> {
    if monitor.spacecontrol().is_visible() {
        return None;
    }
    let mut visible = monitor.visible_workspaces();
    let (workspace, offset) = visible.next()?;
    if visible.next().is_some() || offset != Point::default() {
        return None;
    }
    let tile = workspace.tile(workspace.fullscreen()?)?;
    covers(
        tile.render_rect(),
        tile.anim().alpha(),
        monitor.geometry().size,
    )
    .then_some(tile)
}

/// Whether a window drawn at `rect` with `alpha` hides the whole output.
fn covers(rect: Rectangle<f64, Logical>, alpha: f32, output: Size<i32, Logical>) -> bool {
    alpha >= 1.0 && rect.to_i32_round() == Rectangle::from_size(output)
}

impl Shell {
    /// `tearing_hint` says whether a surface asked for async presentation.
    pub fn display_demand(
        &self,
        monitor: &Monitor,
        gaming: &GamingOptions,
        tearing_hint: impl Fn(&Tile) -> bool,
    ) -> DisplayDemand {
        let locked = self.session_lock.is_active();
        let fullscreen = (!locked).then(|| scanout_tile(monitor)).flatten();
        let windowed_game = || {
            self.visible_windows(monitor)
                .any(|tile| tile.rules().vrr == Some(true) || is_game_content(tile.surface()))
        };
        let menu_open = self.menus.open().is_some();

        DisplayDemand {
            vrr: vrr_wanted(
                monitor.config().vrr,
                locked,
                fullscreen.map(|tile| tile.rules().vrr),
                windowed_game,
            ),
            tearing: fullscreen.is_some_and(|tile| {
                !menu_open
                    && tearing_wanted(gaming.allow_tearing, tile.rules().tearing, || {
                        tearing_hint(tile)
                    })
            }),
        }
    }
}

/// `fullscreen_rule` is `Some` when a window covers the output, carrying its
/// rule's say; a fullscreen window counts as a game unless a rule says not.
fn vrr_wanted(
    policy: Vrr,
    locked: bool,
    fullscreen_rule: Option<Option<bool>>,
    windowed_game: impl FnOnce() -> bool,
) -> bool {
    match policy {
        Vrr::Off => false,
        Vrr::On => true,
        Vrr::OnDemand if locked => false,
        Vrr::OnDemand => match fullscreen_rule {
            Some(rule) => rule.unwrap_or(true),
            None => windowed_game(),
        },
    }
}

/// A rule wins over what the client asked for, which is how X11 games,
/// which cannot ask, get to tear.
fn tearing_wanted(allowed: bool, rule: Option<bool>, asked: impl FnOnce() -> bool) -> bool {
    allowed && rule.unwrap_or_else(asked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> Size<i32, Logical> {
        Size::from((1920, 1200))
    }

    #[test]
    fn a_settled_fullscreen_window_covers_the_output() {
        let rect = Rectangle::from_size(Size::from((1920.0, 1200.0)));
        assert!(covers(rect, 1.0, output()));
    }

    #[test]
    fn a_window_still_growing_does_not() {
        let rect = Rectangle::new((12.0, 8.0).into(), (1800.0, 1100.0).into());
        assert!(!covers(rect, 1.0, output()));
    }

    #[test]
    fn a_window_still_fading_in_does_not() {
        let rect = Rectangle::from_size(Size::from((1920.0, 1200.0)));
        assert!(!covers(rect, 0.6, output()));
    }

    #[test]
    fn explicit_vrr_policies_ignore_the_content() {
        assert!(!vrr_wanted(Vrr::Off, false, Some(Some(true)), || true));
        assert!(vrr_wanted(Vrr::On, true, None, || false));
    }

    #[test]
    fn on_demand_follows_fullscreen_unless_a_rule_objects() {
        assert!(vrr_wanted(Vrr::OnDemand, false, Some(None), || false));
        assert!(!vrr_wanted(Vrr::OnDemand, false, Some(Some(false)), || {
            true
        }));
        assert!(!vrr_wanted(Vrr::OnDemand, false, None, || false));
        assert!(
            vrr_wanted(Vrr::OnDemand, false, None, || true),
            "a windowed game"
        );
    }

    #[test]
    fn on_demand_is_off_while_locked() {
        assert!(!vrr_wanted(Vrr::OnDemand, true, Some(None), || true));
    }

    #[test]
    fn tearing_needs_the_global_switch() {
        assert!(!tearing_wanted(false, Some(true), || true));
    }

    #[test]
    fn a_rule_overrides_the_client_hint() {
        assert!(tearing_wanted(true, Some(true), || false), "an X11 game");
        assert!(!tearing_wanted(true, Some(false), || true));
        assert!(tearing_wanted(true, None, || true));
        assert!(!tearing_wanted(true, None, || false));
    }
}
