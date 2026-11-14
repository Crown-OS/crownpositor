//! One view of every mapped window, refreshed each loop iteration and fanned
//! out to whoever watches windows: the agent daemon over
//! `crownos_window_control_v1`, the context engine over `crownos_context_v1`
//! and crownplugind over `crownos_plugin_host_v1`. Each protocol diffs against
//! what it last sent, so an unchanged desktop costs a comparison and nothing
//! on the wire.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use calloop::timer::{TimeoutAction, Timer};
use crownos_config::schema::VisualContext;
use protocols::{
    crownos_context::{
        ContentType, ContextRecord, ContextWindowFlags, Sandbox, Sensitivity, SessionState,
    },
    crownos_window_control::{Placement, WindowFlags, WindowRecord},
};
use regex::Regex;
use smithay::{
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    utils::{Clock, Monotonic},
    wayland::compositor::with_states,
};

use crate::{
    handlers::ext_workspace::workspace_identifier,
    shell::tile::WindowState,
    state::{
        ClientState, State,
        activity::{ActivityCounters, FLUSH_INTERVAL},
    },
    utils::{
        id::{OutputId, WindowId},
        surface,
    },
};

/// Which windows must never be captured, and why, plus what each watcher was
/// last told.
pub struct WindowDirectory {
    excluded_apps: Vec<Regex>,
    private_titles: Vec<Regex>,
    /// Windows the engine flagged, until when.
    flagged: HashMap<WindowId, Instant>,
    sensitive: HashSet<WindowId>,
    plugin_known: HashMap<WindowId, (String, String)>,
    plugin_focus: Option<WindowId>,
    pub activity: ActivityCounters,
    clock: Clock<Monotonic>,
}

impl WindowDirectory {
    pub fn new(rules: &VisualContext) -> Self {
        Self {
            excluded_apps: glob_patterns(&rules.excluded_apps),
            private_titles: patterns(&rules.private_titles),
            flagged: HashMap::new(),
            sensitive: HashSet::new(),
            plugin_known: HashMap::new(),
            plugin_focus: None,
            activity: ActivityCounters::default(),
            clock: Clock::new(),
        }
    }

    pub fn set_rules(&mut self, rules: &VisualContext) {
        self.excluded_apps = glob_patterns(&rules.excluded_apps);
        self.private_titles = patterns(&rules.private_titles);
    }

    /// Treats `id` as sensitive for `duration`, on the engine's word.
    pub fn flag(&mut self, id: WindowId, duration: Duration) {
        let until = Instant::now() + duration;
        let entry = self.flagged.entry(id).or_insert(until);
        *entry = (*entry).max(until);
    }

    /// Whether captures of `id` must fail right now.
    pub fn is_sensitive(&self, id: WindowId) -> bool {
        self.sensitive.contains(&id)
    }

    fn sensitivity(
        &self,
        id: WindowId,
        app_id: &str,
        title: &str,
        app_excluded: bool,
    ) -> Sensitivity {
        let mut sensitivity = Sensitivity::empty();
        if self.excluded_apps.iter().any(|rule| rule.is_match(app_id)) {
            sensitivity |= Sensitivity::Excluded;
        }
        if self.private_titles.iter().any(|rule| rule.is_match(title)) {
            sensitivity |= Sensitivity::PrivateBrowsing;
        }
        if app_excluded {
            sensitivity |= Sensitivity::AppRequested;
        }
        if self.flagged.contains_key(&id) {
            sensitivity |= Sensitivity::EngineFlagged;
        }
        sensitivity
    }

    fn now_ms(&self) -> u32 {
        let elapsed: Duration = self.clock.now().into();
        u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX)
    }
}

/// A plugin's view of one window: enough to tell `window_opened` from
/// `window_changed`.
struct PluginWindow {
    id: WindowId,
    app_id: String,
    title: String,
    workspace: u32,
}

impl State {
    /// The output agents' windows live on, while it exists.
    pub fn agent_output_id(&self) -> Option<OutputId> {
        let output = self.virtual_outputs.compositor_owned()?;
        Some(self.shell.monitor(output)?.id())
    }

    pub fn is_on_agent_workspace(&self, id: WindowId) -> bool {
        let Some(agent) = self.agent_output_id() else {
            return false;
        };
        self.shell
            .location(id)
            .is_some_and(|location| location.output == agent)
    }

    /// Describes every window once and hands the description to each watcher.
    pub fn sync_window_directory(&mut self) {
        self.revoke_agents_if_locked();
        let now = Instant::now();
        self.windows.flagged.retain(|_, until| *until > now);
        let focus = self.shell.focused_window_id();
        let agent_output = self.agent_output_id();
        let mut control = Vec::new();
        let mut context = Vec::new();
        let mut plugin = Vec::new();
        let mut sensitive = HashSet::new();
        for tile in self.shell.tiles() {
            let id = tile.id();
            let Some(identifier) = self.wayland.foreign_toplevels.identifier(id) else {
                continue;
            };
            let Some(location) = self.shell.location(id) else {
                continue;
            };
            let Some(monitor) = self.shell.monitor_by_id(location.output) else {
                continue;
            };
            let surface = tile.surface();
            let app_id = surface::app_id(surface).unwrap_or_default();
            let title = tile.title().to_owned();
            let client = surface.client();
            let pid = client
                .as_ref()
                .and_then(|client| client.get_credentials(&self.common.display_handle).ok())
                .map(|credentials| credentials.pid)
                .unwrap_or(0);
            let sandbox = client
                .as_ref()
                .and_then(|client| client.get_data::<ClientState>())
                .and_then(|state| state.security_context.as_ref())
                .map(|context| Sandbox {
                    engine: context.sandbox_engine.clone().unwrap_or_default(),
                    app_id: context.app_id.clone().unwrap_or_default(),
                    instance_id: context.instance_id.clone().unwrap_or_default(),
                })
                .unwrap_or_default();
            let rect = tile.content_rect();
            let placement = Placement {
                x: rect.loc.x,
                y: rect.loc.y,
                width: rect.size.w,
                height: rect.size.h,
                output: monitor.output().name(),
                workspace: workspace_identifier(location.workspace),
            };
            let on_agent = Some(location.output) == agent_output;
            let activated = focus == Some(id);
            let minimized = tile.is_minimized();
            let state = tile.state();
            let mut flags = WindowFlags::empty();
            let mut context_flags = ContextWindowFlags::empty();
            if matches!(state, WindowState::Maximized) {
                flags |= WindowFlags::Maximized;
                context_flags |= ContextWindowFlags::Maximized;
            }
            if matches!(state, WindowState::Fullscreen) {
                flags |= WindowFlags::Fullscreen;
                context_flags |= ContextWindowFlags::Fullscreen;
            }
            if minimized {
                flags |= WindowFlags::Minimized;
                context_flags |= ContextWindowFlags::Minimized;
            }
            if state.is_floating() {
                flags |= WindowFlags::Floating;
            }
            if activated {
                flags |= WindowFlags::Activated;
                context_flags |= ContextWindowFlags::Activated;
            }
            if on_agent {
                flags |= WindowFlags::AgentWorkspace;
            }
            if !minimized && !on_agent && !monitor.is_hidden() {
                context_flags |= ContextWindowFlags::Visible;
            }
            let app_excluded = self.wayland.capture_privacy_state.is_excluded(surface);
            let sensitivity = self.windows.sensitivity(id, &app_id, &title, app_excluded);
            if !sensitivity.is_empty() {
                sensitive.insert(id);
            }
            control.push(WindowRecord {
                key: id.raw(),
                identifier: identifier.clone(),
                app_id: app_id.clone(),
                title: title.clone(),
                pid,
                placement: placement.clone(),
                flags,
            });
            context.push(ContextRecord {
                key: id.raw(),
                identifier,
                app_id: app_id.clone(),
                title: title.clone(),
                pid,
                sandbox,
                placement,
                flags: context_flags,
                content_type: content_type_of(surface),
                sensitivity,
            });
            plugin.push(PluginWindow {
                id,
                app_id,
                title,
                workspace: monitor
                    .index_of(location.workspace)
                    .and_then(|index| u32::try_from(index).ok())
                    .unwrap_or(0),
            });
        }
        self.windows.sensitive = sensitive;
        let display = self.common.display_handle.clone();
        let focus_key = focus.map(WindowId::raw);
        self.wayland
            .window_control_state
            .sync::<State>(&display, &control, focus_key);
        let session = if self.shell.is_locked() {
            SessionState::Locked
        } else {
            SessionState::Active
        };
        self.wayland.context_state.set_session(session);
        let time_ms = self.windows.now_ms();
        self.wayland
            .context_state
            .sync::<State>(&display, &context, focus_key, time_ms);
        self.sync_plugin_windows(plugin, focus);
    }

    fn sync_plugin_windows(&mut self, windows: Vec<PluginWindow>, focus: Option<WindowId>) {
        let host = &self.wayland.plugin_host_state;
        let known = &mut self.windows.plugin_known;
        known.retain(|id, _| {
            let alive = windows.iter().any(|window| window.id == *id);
            if !alive {
                host.window_closed(id.raw());
            }
            alive
        });
        for window in windows {
            match known.get(&window.id) {
                Some((app_id, title)) if *app_id == window.app_id && *title == window.title => {}
                Some(_) => host.window_changed(window.id.raw(), &window.app_id, &window.title),
                None => host.window_opened(
                    window.id.raw(),
                    &window.app_id,
                    &window.title,
                    window.workspace,
                ),
            }
            known.insert(window.id, (window.app_id, window.title));
        }
        if self.windows.plugin_focus != focus {
            self.windows.plugin_focus = focus;
            host.window_focused(focus.map_or(0, WindowId::raw));
        }
    }

    /// Hands over input buckets on a fixed beat, so a quiet desktop sends
    /// nothing and a busy one sends one event per interval.
    pub fn arm_activity_flush(&mut self) {
        let armed = self.common.event_loop_handle.insert_source(
            Timer::from_duration(FLUSH_INTERVAL),
            |_, _, state| {
                state.flush_input_activity();
                TimeoutAction::ToDuration(FLUSH_INTERVAL)
            },
        );
        if let Err(error) = armed {
            tracing::warn!(%error, "input activity will not reach the context engine");
        }
    }

    /// Hands the last interval's input counts to the context engine, with text
    /// counts blanked while the focused window is sensitive.
    pub fn flush_input_activity(&mut self) {
        let Some(mut activity) = self.windows.activity.take() else {
            return;
        };
        let focus_sensitive = self
            .shell
            .focused_window_id()
            .is_some_and(|id| self.windows.is_sensitive(id));
        if focus_sensitive {
            activity.text = 0;
            activity.edit = 0;
            activity.nav = 0;
            activity.chords = 0;
        }
        self.wayland.context_state.input_activity(activity);
    }
}

fn content_type_of(surface: &WlSurface) -> ContentType {
    use smithay::reexports::wayland_protocols::wp::content_type::v1::server::wp_content_type_v1::Type;
    use smithay::wayland::content_type::ContentTypeSurfaceCachedState;

    with_states(surface, |states| {
        match *states
            .cached_state
            .get::<ContentTypeSurfaceCachedState>()
            .current()
            .content_type()
        {
            Type::Photo => ContentType::Photo,
            Type::Video => ContentType::Video,
            Type::Game => ContentType::Game,
            _ => ContentType::None,
        }
    })
}

fn patterns(sources: &[String]) -> Vec<Regex> {
    sources
        .iter()
        .filter_map(|source| match Regex::new(source) {
            Ok(regex) => Some(regex),
            Err(error) => {
                tracing::warn!(source, %error, "ignoring a malformed visual_context rule");
                None
            }
        })
        .collect()
}

/// `*` and `?` globs over app ids, matched whole and without regard to case.
fn glob_patterns(globs: &[String]) -> Vec<Regex> {
    let sources: Vec<String> = globs.iter().map(|glob| glob_to_regex(glob)).collect();
    patterns(&sources)
}

fn glob_to_regex(glob: &str) -> String {
    let mut source = String::with_capacity(glob.len() + 8);
    source.push_str("(?i)^");
    for character in glob.chars() {
        match character {
            '*' => source.push_str(".*"),
            '?' => source.push('.'),
            other => source.push_str(&regex::escape(&other.to_string())),
        }
    }
    source.push('$');
    source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_whole_app_ids_case_insensitively() {
        let rules = glob_patterns(&["org.keepassxc.*".to_owned(), "*Bank*".to_owned()]);
        assert!(
            rules
                .iter()
                .any(|rule| rule.is_match("org.keepassxc.KeePassXC"))
        );
        assert!(rules.iter().any(|rule| rule.is_match("com.mybank.app")));
        assert!(!rules.iter().any(|rule| rule.is_match("org.keepassxc")));
        assert!(!rules.iter().any(|rule| rule.is_match("kitty")));
    }

    #[test]
    fn malformed_rules_are_dropped_not_fatal() {
        assert_eq!(patterns(&["(".to_owned(), "ok".to_owned()]).len(), 1);
    }
}
