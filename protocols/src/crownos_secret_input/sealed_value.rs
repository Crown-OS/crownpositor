//! Reads one value from a sealed memfd, as the protocol requires it.

use std::os::fd::OwnedFd;

use rustix::fs::SealFlags;
use zeroize::{Zeroize, Zeroizing};

/// The largest value the compositor accepts, in bytes.
pub const MAX_VALUE_BYTES: usize = 4096;

const REQUIRED_SEALS: SealFlags = SealFlags::WRITE
    .union(SealFlags::SHRINK)
    .union(SealFlags::GROW);

/// Why a value was refused. It never holds the value.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValueRefusal {
    #[error("the value is not a memfd sealed against writes and size changes")]
    NotSealed,
    #[error("the value is empty or longer than {MAX_VALUE_BYTES} bytes")]
    BadSize,
    #[error("the value could not be read")]
    Unreadable,
    #[error("the value is not UTF-8 text")]
    NotText,
    #[error("the value holds a control character")]
    ControlCharacter,
}

/// Reads the value and closes the fd. The returned text is zeroed on drop.
pub fn read_sealed_value(fd: OwnedFd) -> Result<Zeroizing<String>, ValueRefusal> {
    let seals = rustix::fs::fcntl_get_seals(&fd).map_err(|_| ValueRefusal::NotSealed)?;
    if !seals.contains(REQUIRED_SEALS) {
        return Err(ValueRefusal::NotSealed);
    }
    let size = rustix::fs::fstat(&fd)
        .map_err(|_| ValueRefusal::Unreadable)?
        .st_size;
    let size = usize::try_from(size).map_err(|_| ValueRefusal::BadSize)?;
    if size == 0 || size > MAX_VALUE_BYTES {
        return Err(ValueRefusal::BadSize);
    }
    let mut bytes = Zeroizing::new(vec![0_u8; size]);
    let mut filled = 0;
    while filled < size {
        let offset = u64::try_from(filled).map_err(|_| ValueRefusal::Unreadable)?;
        match rustix::io::pread(&fd, &mut bytes[filled..], offset) {
            Ok(0) | Err(_) => return Err(ValueRefusal::Unreadable),
            Ok(read) => filled += read,
        }
    }
    drop(fd);
    let text = match String::from_utf8(std::mem::take(&mut *bytes)) {
        Ok(text) => Zeroizing::new(text),
        Err(error) => {
            error.into_bytes().zeroize();
            return Err(ValueRefusal::NotText);
        }
    };
    if text.chars().any(char::is_control) {
        return Err(ValueRefusal::ControlCharacter);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use rustix::fs::{MemfdFlags, fcntl_add_seals, memfd_create};

    use super::*;

    fn memfd(content: &[u8], seals: SealFlags) -> OwnedFd {
        let fd = memfd_create(
            "secret-input-test",
            MemfdFlags::ALLOW_SEALING | MemfdFlags::CLOEXEC,
        )
        .unwrap();
        let mut file = std::fs::File::from(fd);
        file.write_all(content).unwrap();
        let fd = OwnedFd::from(file);
        fcntl_add_seals(&fd, seals).unwrap();
        fd
    }

    #[test]
    fn a_sealed_value_is_read() {
        let value = read_sealed_value(memfd(b"hunter2", REQUIRED_SEALS)).unwrap();
        assert_eq!(value.as_str(), "hunter2");
    }

    #[test]
    fn a_value_without_every_seal_is_refused() {
        for seals in [
            SealFlags::empty(),
            SealFlags::WRITE,
            SealFlags::SHRINK | SealFlags::GROW,
        ] {
            assert_eq!(
                read_sealed_value(memfd(b"hunter2", seals)),
                Err(ValueRefusal::NotSealed)
            );
        }
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(b"hunter2").unwrap();
        assert_eq!(
            read_sealed_value(OwnedFd::from(reader)),
            Err(ValueRefusal::NotSealed)
        );
    }

    #[test]
    fn the_size_is_limited() {
        let largest = vec![b'x'; MAX_VALUE_BYTES];
        assert!(read_sealed_value(memfd(&largest, REQUIRED_SEALS)).is_ok());
        let too_large = vec![b'x'; MAX_VALUE_BYTES + 1];
        assert_eq!(
            read_sealed_value(memfd(&too_large, REQUIRED_SEALS)),
            Err(ValueRefusal::BadSize)
        );
        assert_eq!(
            read_sealed_value(memfd(b"", REQUIRED_SEALS)),
            Err(ValueRefusal::BadSize)
        );
    }

    #[test]
    fn only_plain_text_is_accepted() {
        for value in [&b"line\nbreak"[..], b"tab\there", b"\x1b[31m", b"nul\0"] {
            assert_eq!(
                read_sealed_value(memfd(value, REQUIRED_SEALS)),
                Err(ValueRefusal::ControlCharacter)
            );
        }
        assert_eq!(
            read_sealed_value(memfd(&[0xff, 0xfe], REQUIRED_SEALS)),
            Err(ValueRefusal::NotText)
        );
        assert!(read_sealed_value(memfd("pässwörd ✓".as_bytes(), REQUIRED_SEALS)).is_ok());
    }
}
