//! The built-in lock screen's life: going up, taking input, checking the
//! password off-thread, and coming down only on PAM's say-so.
//!
//! Everything a client could reach is closed off while it is up: keyboard and
//! pointer focus go nowhere, shortcuts other than VT switching and volume are
//! off, input capture and injection are refused, and window captures go
//! blank. If the compositor dies the session dies with it — there is no
//! unlocked state to fall back to.

mod idle;
mod keys;
mod logind;
mod scene;
mod visuals;
mod wake;

use std::{borrow::Cow, collections::HashMap, time::Instant};

use calloop::RegistrationToken;
use lockscreen::{
    auth::{AuthEvent, Authenticator},
    clock::ClockFace,
    flow::{AuthOutcome, LockFlow},
    lockscene::AssetId,
    paint::Painter,
    palette::Palette,
    secret::Password,
    tiny_skia::Pixmap,
    user::User,
};
use smithay::{
    backend::input::InputTime,
    input::pointer::{CursorIcon, CursorImageStatus},
    utils::SERIAL_COUNTER,
};

use crownos_config::schema::LockBackground;

use crate::{shell::lock_screen::Backdrop, state::State};

pub use idle::IdleLock;
pub use keys::key_input;
pub use logind::SleepGuard;
pub use scene::{AssetError, SceneSource};

/// Why the lock went up, which decides how it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockReason {
    /// A keybinding, `loginctl lock-session`, a plugin's action.
    Requested,
    Idle,
    /// On the way to suspend: no entrance animation, so nothing is still
    /// fading in when the machine sleeps.
    Suspend,
}

#[derive(Default)]
pub struct LockScreenState {
    pub scene: SceneSource,
    painter: Option<Painter>,
    authenticator: Option<Authenticator>,
    user: Option<User>,
    face: ClockFace,
    /// Decoded slot images, kept only while locked.
    images: DecodedImages,
    /// Bumped per lock, so a decode finishing after its lock ended is dropped.
    generation: u64,
    wake: Option<RegistrationToken>,
    pub idle: IdleLock,
    pub sleep: SleepGuard,
}

/// Decoded plugin images by asset, for the layout.
#[derive(Default)]
struct DecodedImages(HashMap<AssetId, Pixmap>);

impl lockscreen::layout::Images for DecodedImages {
    fn get(&self, id: &AssetId) -> Option<&Pixmap> {
        self.0.get(id)
    }
}

impl State {
    /// Loads fonts off-thread and starts the password checker. Called once,
    /// after the event loop exists.
    pub fn init_lock_screen(&mut self) {
        self.lock
            .scene
            .rebuild(&self.config.current.lock, &self.config.current.theme);
        match User::current() {
            Ok(user) => {
                let tasks = self.common.tasks.clone();
                match Authenticator::spawn(&auth_service(), &user, move |event| {
                    tasks.deliver(move |state| state.on_auth_event(event));
                }) {
                    Ok(authenticator) => self.lock.authenticator = Some(authenticator),
                    Err(err) => tracing::error!(%err, "the lock screen cannot check passwords"),
                }
                self.lock.user = Some(user);
            }
            Err(err) => tracing::error!(%err, "the lock screen does not know its user"),
        }
        self.common.tasks.run(
            async {
                tokio::task::spawn_blocking(|| {
                    Painter::new(lockscreen::cosmic_text::FontSystem::new())
                })
                .await
                .ok()
            },
            |state, painter| {
                state.lock.painter = painter;
                state.mark_lock_dirty();
            },
        );
        self.start_logind_watch();
        self.schedule_idle_lock();
    }

    /// Puts the lock up. Does nothing if it already is, unless it is on its
    /// way down: then it starts over.
    pub fn lock_screen(&mut self, reason: LockReason) {
        if self
            .shell
            .lock_screen
            .active()
            .is_some_and(|lock| !lock.flow.is_unlocking())
        {
            return;
        }
        let now = Instant::now();
        let config = &self.lock.scene.current().config;
        let flow = match reason {
            LockReason::Suspend => LockFlow::settled(config, now),
            LockReason::Requested | LockReason::Idle => LockFlow::new(config, now),
        };
        let backdrop = match &config.background {
            LockBackground::Desktop => Backdrop::Wallpaper,
            LockBackground::Image(_) => Backdrop::Picture,
            LockBackground::Token(token) => {
                Backdrop::Color(Palette::new(&self.lock.scene.current().theme).color(token))
            }
        };
        let (dim, blur) = (config.dim, config.blur);

        self.dismiss_menu();
        for monitor in self.shell.monitors_mut() {
            monitor.spacecontrol_mut().close();
            monitor.window_control_mut().dismiss();
        }
        self.shell.lock_screen.begin(flow, backdrop, dim, blur);
        self.release_seat_for_lock();

        self.lock.generation = self.lock.generation.wrapping_add(1);
        self.lock.face = ClockFace::now(&self.lock.scene.current().config);
        self.load_lock_pictures();
        self.schedule_lock_wake();
        self.announce_lock_state();
        tracing::info!(?reason, "screen locked");
        self.queue_redraw();
    }

    /// Takes the lock down once its unlock has finished animating. The only
    /// way down is a [`AuthOutcome::Accepted`] from the password checker.
    fn finish_unlock(&mut self) {
        self.shell.lock_screen.end();
        self.lock.images = DecodedImages::default();
        self.lock.generation = self.lock.generation.wrapping_add(1);
        if let Some(token) = self.lock.wake.take() {
            self.common.event_loop_handle.remove(token);
        }
        self.update_keyboard_focus();
        self.update_pointer_focus();
        self.announce_lock_state();
        self.schedule_idle_lock();
        tracing::info!("screen unlocked");
        self.queue_redraw();
    }

    /// No grab, no client focus, the compositor's own cursor.
    fn release_seat_for_lock(&mut self) {
        if let Some(pointer) = self.wayland.seat.get_pointer() {
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), InputTime::now());
        }
        if let Some(keyboard) = self.wayland.seat.get_keyboard() {
            keyboard.unset_grab(self);
        }
        self.update_keyboard_focus();
        self.update_pointer_focus();
        self.input.cursor.status = CursorImageStatus::Named(CursorIcon::Default);
        self.queue_pointer_redraw();
    }

    fn submit_password(&mut self, password: Password) {
        let accepted = self
            .lock
            .authenticator
            .as_ref()
            .is_some_and(|authenticator| authenticator.submit(password));
        if !accepted {
            self.on_auth_event(AuthEvent::Rejected);
        }
    }

    pub fn on_auth_event(&mut self, event: AuthEvent) {
        let Some(lock) = self.shell.lock_screen.active_mut() else {
            return;
        };
        let now = Instant::now();
        match event {
            AuthEvent::Message(text) => lock.flow.service_message(&text),
            AuthEvent::Accepted if lock.flow.is_checking() => {
                lock.flow.finish(AuthOutcome::Accepted, now)
            }
            AuthEvent::Accepted | AuthEvent::Rejected => {
                lock.flow.finish(AuthOutcome::Rejected, now)
            }
        }
        lock.dirty = true;
        self.schedule_lock_wake();
        self.queue_redraw();
    }

    /// Once per loop turn: ends a finished unlock, applies timeouts and brings
    /// every output's sprites up to date.
    pub fn refresh_lock_screen(&mut self) {
        let now = Instant::now();
        let Some(lock) = self.shell.lock_screen.active_mut() else {
            return;
        };
        if lock.flow.is_finished(now) {
            self.finish_unlock();
            return;
        }
        for visuals in lock.visuals.values_mut() {
            visuals.settle_morphs(now);
        }
        if lock.flow.poll(now) {
            lock.dirty = true;
            self.schedule_lock_wake();
            self.queue_redraw();
        }
        self.prepare_lock_visuals();
        self.release_sleep_once_locked();
    }

    pub fn mark_lock_dirty(&mut self) {
        if let Some(lock) = self.shell.lock_screen.active_mut() {
            lock.dirty = true;
            self.queue_redraw();
        }
    }

    /// The config or a plugin changed the scene.
    pub fn lock_scene_changed(&mut self) {
        self.lock
            .scene
            .rebuild(&self.config.current.lock, &self.config.current.theme);
        if let Some(lock) = self.shell.lock_screen.active_mut() {
            lock.flow.configure(&self.lock.scene.current().config);
        }
        self.mark_lock_dirty();
        self.schedule_idle_lock();
        self.refresh_sleep_inhibitor();
    }

    /// `crownplugind` changed the layout, a slot or its images.
    pub fn lock_content_changed(&mut self) {
        self.lock
            .scene
            .rebuild(&self.config.current.lock, &self.config.current.theme);
        self.mark_lock_dirty();
    }

    /// Tells the plugin daemon and logind whether the session is locked, by
    /// either lock.
    pub fn announce_lock_state(&mut self) {
        let locked = self.shell.is_locked();
        self.wayland.plugin_host_state.set_locked(locked);
        self.set_logind_locked_hint(locked);
    }
}

/// The unlock service; a debug build may name another, such as one that
/// accepts anything, for hands-free nested tests.
fn auth_service() -> Cow<'static, str> {
    #[cfg(debug_assertions)]
    if let Ok(service) = std::env::var("CROWN_PAM_SERVICE") {
        return Cow::Owned(service);
    }
    Cow::Borrowed(lockscreen::auth::default_service())
}
