//! Reading the bytes a request hands over in a file descriptor.

use std::os::fd::OwnedFd;

use smithay::reexports::rustix::{
    fs::{fcntl_get_seals, fstat},
    io::pread,
};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PayloadError {
    #[error("the payload is {got} bytes, over the {limit} byte limit")]
    TooLarge { got: usize, limit: usize },
    #[error("the payload is not a memfd holding the bytes it claims")]
    Unreadable,
}

/// The first `size` bytes of `fd`, which must be a memfd.
///
/// Only a memfd is accepted: it reads from memory, so a client cannot stall
/// the compositor with a pipe or a file on a hung filesystem. The bytes are
/// copied out before anything looks at them, so the client changing the
/// memfd afterwards changes nothing.
pub fn read(fd: &OwnedFd, size: u32, limit: usize) -> Result<Vec<u8>, PayloadError> {
    let size = size as usize;
    if size > limit {
        return Err(PayloadError::TooLarge { got: size, limit });
    }
    fcntl_get_seals(fd).map_err(|_| PayloadError::Unreadable)?;
    let stat = fstat(fd).map_err(|_| PayloadError::Unreadable)?;
    if usize::try_from(stat.st_size).map_or(true, |len| len < size) {
        return Err(PayloadError::Unreadable);
    }
    let mut bytes = vec![0; size];
    let mut filled = 0;
    while filled < size {
        let read =
            pread(fd, &mut bytes[filled..], filled as u64).map_err(|_| PayloadError::Unreadable)?;
        if read == 0 {
            return Err(PayloadError::Unreadable);
        }
        filled += read;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use smithay::reexports::rustix::{
        fs::{MemfdFlags, memfd_create},
        io::write,
        pipe::pipe,
    };

    use super::*;

    fn memfd(bytes: &[u8]) -> OwnedFd {
        let fd = memfd_create("payload-test", MemfdFlags::CLOEXEC).expect("memfd");
        write(&fd, bytes).expect("written");
        fd
    }

    #[test]
    fn a_memfd_reads_back_exactly_size_bytes() {
        let fd = memfd(b"hello world");
        assert_eq!(read(&fd, 5, 64), Ok(b"hello".to_vec()));
    }

    #[test]
    fn claims_beyond_the_file_or_the_limit_are_refused() {
        let fd = memfd(b"abc");
        assert_eq!(read(&fd, 4, 64), Err(PayloadError::Unreadable));
        assert_eq!(
            read(&fd, 65, 64),
            Err(PayloadError::TooLarge { got: 65, limit: 64 })
        );
    }

    #[test]
    fn a_pipe_is_refused_rather_than_read() {
        let (reader, _writer) = pipe().expect("pipe");
        assert_eq!(read(&reader, 1, 64), Err(PayloadError::Unreadable));
    }
}
