//! The few libpam calls a password check needs.

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr,
};

use crate::secret::Password;

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_REFRESH_CRED: c_int = 0x0010;
/// More messages than any module sends in one call; a guard on `num_msg`.
const MAX_MESSAGES: c_int = 32;

#[repr(C)]
struct PamMessage {
    style: c_int,
    text: *const c_char,
}

#[repr(C)]
struct PamResponse {
    text: *mut c_char,
    retcode: c_int,
}

type Converse = unsafe extern "C" fn(
    c_int,
    *mut *const PamMessage,
    *mut *mut PamResponse,
    *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    converse: Option<Converse>,
    data: *mut c_void,
}

#[repr(C)]
struct PamHandle {
    _opaque: [u8; 0],
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conversation: *const PamConv,
        handle: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_authenticate(handle: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_setcred(handle: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_end(handle: *mut PamHandle, status: c_int) -> c_int;
}

/// What a module said along the way, and the password it may ask for.
struct Conversation<'a> {
    password: &'a Password,
    said: &'a mut dyn FnMut(&str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Accepted,
    Rejected,
}

/// Checks `password` for `user` through `service`. Blocks for as long as the
/// modules take, which on a failure is seconds by design.
pub(super) fn authenticate(
    service: &CStr,
    user: &CStr,
    password: &Password,
    said: &mut dyn FnMut(&str),
) -> Verdict {
    let mut conversation = Conversation { password, said };
    let conv = PamConv {
        converse: Some(converse),
        data: (&raw mut conversation).cast(),
    };
    let mut handle = ptr::null_mut();
    // SAFETY: the strings and `conv` outlive the handle, which is ended below
    // on every path that created it.
    unsafe {
        let mut status = pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle);
        if status != PAM_SUCCESS || handle.is_null() {
            return Verdict::Rejected;
        }
        status = pam_authenticate(handle, 0);
        if status == PAM_SUCCESS {
            // Renews Kerberos tickets and the like; its failure does not
            // undo a correct password.
            pam_setcred(handle, PAM_REFRESH_CRED);
        }
        pam_end(handle, status);
        if status == PAM_SUCCESS {
            Verdict::Accepted
        } else {
            Verdict::Rejected
        }
    }
}

/// Answers every prompt with the password and passes messages on.
///
/// # Safety
///
/// Called by libpam with `count` messages and `data` pointing at the
/// `Conversation` handed to `pam_start`.
unsafe extern "C" fn converse(
    count: c_int,
    messages: *mut *const PamMessage,
    responses: *mut *mut PamResponse,
    data: *mut c_void,
) -> c_int {
    if !(1..=MAX_MESSAGES).contains(&count)
        || messages.is_null()
        || responses.is_null()
        || data.is_null()
    {
        return PAM_CONV_ERR;
    }
    let count = count as usize;
    // SAFETY: `data` is the live `Conversation` from `authenticate`.
    let conversation = unsafe { &mut *data.cast::<Conversation<'_>>() };
    // SAFETY: calloc of a non-zero count; libpam frees the array with free().
    let replies = unsafe { libc::calloc(count, size_of::<PamResponse>()) }.cast::<PamResponse>();
    if replies.is_null() {
        return PAM_BUF_ERR;
    }
    for index in 0..count {
        // Linux-PAM passes an array of pointers to messages.
        // SAFETY: libpam guarantees `count` valid message pointers.
        let message = unsafe { &**messages.add(index) };
        let text = if message.text.is_null() {
            ""
        } else {
            // SAFETY: a NUL-terminated string owned by the module.
            unsafe { CStr::from_ptr(message.text) }
                .to_str()
                .unwrap_or_default()
        };
        match message.style {
            PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON => {
                let Some(answer) = c_copy(conversation.password.as_bytes()) else {
                    // SAFETY: frees exactly what this call allocated so far.
                    unsafe { discard(replies, index) };
                    return PAM_BUF_ERR;
                };
                // SAFETY: `index` is within the `count` entries allocated.
                unsafe { (*replies.add(index)).text = answer };
            }
            PAM_ERROR_MSG | PAM_TEXT_INFO => (conversation.said)(text),
            _ => {
                // SAFETY: as above.
                unsafe { discard(replies, index) };
                return PAM_CONV_ERR;
            }
        }
    }
    // SAFETY: libpam asked for its responses here.
    unsafe { *responses = replies };
    PAM_SUCCESS
}

/// A malloc'd, NUL-terminated copy of `bytes`, which libpam frees.
fn c_copy(bytes: &[u8]) -> Option<*mut c_char> {
    if bytes.contains(&0) {
        return None;
    }
    // SAFETY: allocates `len + 1` zeroed bytes, then copies `len` into it.
    unsafe {
        let copy = libc::calloc(bytes.len() + 1, 1).cast::<u8>();
        if copy.is_null() {
            return None;
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), copy, bytes.len());
        Some(copy.cast())
    }
}

/// Wipes and frees the first `filled` answers and the array itself.
///
/// # Safety
///
/// `replies` must be the calloc'd array from `converse`.
unsafe fn discard(replies: *mut PamResponse, filled: usize) {
    for index in 0..filled {
        // SAFETY: entries up to `filled` were written by `converse`.
        let text = unsafe { (*replies.add(index)).text };
        if !text.is_null() {
            // SAFETY: a NUL-terminated copy made by `c_copy`.
            unsafe {
                let len = libc::strlen(text);
                ptr::write_bytes(text, 0, len);
                libc::free(text.cast());
            }
        }
    }
    // SAFETY: the array came from calloc.
    unsafe { libc::free(replies.cast()) };
}

/// A C string without interior NULs, for the service and user names.
pub(super) fn c_string(text: &str) -> Option<CString> {
    CString::new(text).ok()
}
