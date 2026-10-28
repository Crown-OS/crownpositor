use std::fmt;

use zeroize::Zeroize;

/// Longest password accepted, in bytes of UTF-8.
pub const PASSWORD_BYTES: usize = 512;

/// A password being typed.
///
/// The bytes live in one fixed heap block that never grows — a growing buffer
/// leaves copies of its old contents behind in freed memory — which is kept
/// out of swap while it lives and wiped when it drops.
pub struct Password {
    bytes: Box<[u8; PASSWORD_BYTES]>,
    len: usize,
    locked: bool,
}

impl Password {
    pub fn new() -> Self {
        let bytes = Box::new([0; PASSWORD_BYTES]);
        // SAFETY: the range is one live allocation this value owns until drop,
        // where it is unlocked again.
        let locked = unsafe { libc::mlock(bytes.as_ptr().cast(), PASSWORD_BYTES) == 0 };
        Self {
            bytes,
            len: 0,
            locked,
        }
    }

    /// Appends `c`, or refuses it when it would not fit or is a control
    /// character.
    pub fn push(&mut self, c: char) -> bool {
        let width = c.len_utf8();
        if c.is_control() || self.len + width > PASSWORD_BYTES {
            return false;
        }
        c.encode_utf8(&mut self.bytes[self.len..self.len + width]);
        self.len += width;
        true
    }

    /// Removes the last character.
    pub fn pop(&mut self) -> bool {
        let Some(start) = (0..self.len)
            .rev()
            .find(|&index| self.bytes[index] & 0xC0 != 0x80)
        else {
            return false;
        };
        self.bytes[start..self.len].zeroize();
        self.len = start;
        true
    }

    pub fn clear(&mut self) {
        self.bytes[..self.len].zeroize();
        self.len = 0;
    }

    /// Moves the password out, leaving this one empty.
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Characters typed, for drawing one dot each.
    pub fn chars(&self) -> usize {
        self.as_bytes()
            .iter()
            .filter(|&&byte| byte & 0xC0 != 0x80)
            .count()
    }
}

impl Default for Password {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        self.bytes.zeroize();
        if self.locked {
            // SAFETY: the same range `new` locked, still allocated.
            unsafe { libc::munlock(self.bytes.as_ptr().cast(), PASSWORD_BYTES) };
        }
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_keeps_whole_characters() {
        let mut password = Password::new();
        for c in "pä😀".chars() {
            assert!(password.push(c));
        }
        assert_eq!(password.chars(), 3);
        assert!(password.pop());
        assert_eq!(password.as_bytes(), "pä".as_bytes());
        assert!(!password.push('\n'));
        password.clear();
        assert!(password.is_empty() && !password.pop());
    }

    #[test]
    fn it_never_grows_past_its_block() {
        let mut password = Password::new();
        let accepted = (0..PASSWORD_BYTES + 10)
            .filter(|_| password.push('x'))
            .count();
        assert_eq!(accepted, PASSWORD_BYTES);
        assert!(!password.push('é'));
    }

    #[test]
    fn taking_leaves_it_empty_and_debug_hides_it() {
        let mut password = Password::new();
        password.push('s');
        let taken = password.take();
        assert!(password.is_empty());
        assert_eq!(taken.as_bytes(), b"s");
        assert_eq!(format!("{taken:?}"), "Password(..)");
    }
}
