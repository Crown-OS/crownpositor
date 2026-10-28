//! Password checks, off the caller's thread.
//!
//! One worker thread owns every PAM conversation, so a slow module never
//! stalls the compositor and two checks never run at once. The account is
//! always the process's own: the lock screen cannot be talked into checking
//! anyone else's password.

mod pam;

use std::{
    ffi::CString,
    path::Path,
    sync::mpsc::{self, SyncSender, TrySendError},
    thread,
};

use thiserror::Error;

use crate::{secret::Password, user::User};

/// The PAM service CrownOS installs for unlocking.
pub const SERVICE: &str = "crownos-unlock";
/// Present on every distribution, for systems without [`SERVICE`] yet.
const FALLBACK_SERVICE: &str = "login";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthEvent {
    /// Text a module wants shown, such as a lockout notice.
    Message(String),
    Accepted,
    Rejected,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("the service or user name holds a NUL")]
    BadName,
    #[error("could not start the authentication thread: {0}")]
    Thread(#[from] std::io::Error),
}

pub struct Authenticator {
    requests: SyncSender<Password>,
}

impl Authenticator {
    /// Starts the worker for `user` on `service`. `events` runs on the
    /// worker thread, so it should only hand the event over.
    pub fn spawn(
        service: &str,
        user: &User,
        events: impl Fn(AuthEvent) + Send + 'static,
    ) -> Result<Self, AuthError> {
        let service = pam::c_string(service).ok_or(AuthError::BadName)?;
        let login = pam::c_string(&user.login).ok_or(AuthError::BadName)?;
        let (requests, incoming) = mpsc::sync_channel::<Password>(1);
        thread::Builder::new()
            .name("crown-lock-auth".into())
            .spawn(move || serve(&service, &login, &incoming, &events))?;
        Ok(Self { requests })
    }

    /// Queues `password` for checking. `false` when a check is already
    /// running; the password is then wiped and dropped.
    pub fn submit(&self, password: Password) -> bool {
        match self.requests.try_send(password) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        }
    }
}

fn serve(
    service: &CString,
    login: &CString,
    incoming: &mpsc::Receiver<Password>,
    events: &dyn Fn(AuthEvent),
) {
    while let Ok(password) = incoming.recv() {
        let verdict = pam::authenticate(service, login, &password, &mut |text| {
            events(AuthEvent::Message(text.to_owned()));
        });
        drop(password);
        events(match verdict {
            pam::Verdict::Accepted => AuthEvent::Accepted,
            pam::Verdict::Rejected => AuthEvent::Rejected,
        });
    }
}

/// [`SERVICE`] when the system has it configured, else a stock one.
pub fn default_service() -> &'static str {
    if Path::new("/etc/pam.d").join(SERVICE).is_file()
        || Path::new("/usr/lib/pam.d").join(SERVICE).is_file()
    {
        SERVICE
    } else {
        FALLBACK_SERVICE
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc::channel, time::Duration};

    use super::*;

    /// A service no system configures falls through to PAM's `other`, which
    /// denies: the real account's failure counter is never touched.
    const UNCONFIGURED: &str = "crownos-lockscreen-test-unconfigured";

    #[test]
    fn a_check_is_answered_off_thread() {
        let user = User::current().expect("an account");
        let (sender, events) = channel();
        let auth = Authenticator::spawn(UNCONFIGURED, &user, move |event| {
            let _ = sender.send(event);
        })
        .expect("spawns");
        let mut password = Password::new();
        for c in "certainly-not-the-password-\u{1f512}".chars() {
            password.push(c);
        }
        assert!(auth.submit(password));
        let verdict = events
            .iter()
            .find(|event| !matches!(event, AuthEvent::Message(_)))
            .expect("an answer");
        assert_eq!(verdict, AuthEvent::Rejected);
        assert!(events.recv_timeout(Duration::from_millis(10)).is_err());
    }

    #[test]
    fn names_with_nul_are_refused() {
        let user = User {
            login: "a\0b".into(),
            display_name: "x".into(),
        };
        assert!(matches!(
            Authenticator::spawn(SERVICE, &user, |_| {}),
            Err(AuthError::BadName)
        ));
    }
}
