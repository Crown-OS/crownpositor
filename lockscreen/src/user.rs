use std::ffi::CStr;

use thiserror::Error;

#[derive(Debug, Error)]
#[error("no passwd entry for uid {0}")]
pub struct UnknownUser(u32);

/// Who the session belongs to: the only account the lock screen unlocks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub login: String,
    /// The GECOS full name, or the login when there is none.
    pub display_name: String,
}

impl User {
    /// The account of this process's real uid.
    pub fn current() -> Result<Self, UnknownUser> {
        // SAFETY: getuid cannot fail.
        let uid = unsafe { libc::getuid() };
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut found = std::ptr::null_mut();
        let mut scratch = vec![0 as libc::c_char; 16 * 1024];
        // SAFETY: every pointer is valid for the call; `found` is either null
        // or points at `entry`, whose strings live in `scratch`.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                scratch.as_mut_ptr(),
                scratch.len(),
                &mut found,
            )
        };
        if status != 0 || found.is_null() {
            return Err(UnknownUser(uid));
        }
        // SAFETY: getpwuid_r succeeded, so `entry` is initialised and its
        // string fields are NUL-terminated inside `scratch`.
        let (login, gecos) = unsafe {
            let entry = entry.assume_init();
            (
                CStr::from_ptr(entry.pw_name).to_string_lossy().into_owned(),
                (!entry.pw_gecos.is_null()).then(|| {
                    CStr::from_ptr(entry.pw_gecos)
                        .to_string_lossy()
                        .into_owned()
                }),
            )
        };
        let full_name = gecos
            .as_deref()
            .and_then(|gecos| gecos.split(',').next())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        Ok(Self {
            display_name: full_name.unwrap_or_else(|| login.clone()),
            login,
        })
    }

    pub fn initial(&self) -> char {
        self.display_name
            .chars()
            .next()
            .map_or('?', |c| c.to_uppercase().next().unwrap_or(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_runner_has_an_account() {
        let user = User::current().expect("a passwd entry");
        assert!(!user.login.is_empty());
        assert_ne!(user.initial(), '?');
    }
}
