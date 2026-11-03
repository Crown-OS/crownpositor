//! Reading another process's environment, as it was when the process started.

/// The value of `name` in `pid`'s initial environment. `None` when it is
/// unset, not UTF-8, or the process is gone or not ours to read.
pub fn variable(pid: i32, name: &str) -> Option<String> {
    let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    let value = find(&environ, name)?;
    String::from_utf8(value.to_vec()).ok()
}

fn find<'a>(environ: &'a [u8], name: &str) -> Option<&'a [u8]> {
    environ
        .split(|&byte| byte == 0)
        .find_map(|entry| entry.strip_prefix(name.as_bytes())?.strip_prefix(b"="))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENVIRON: &[u8] = b"HOME=/home/me\0WORKSPACE_X=1\0WORKSPACE=3\0EMPTY=\0";

    #[test]
    fn finds_the_exact_name_only() {
        assert_eq!(find(ENVIRON, "WORKSPACE"), Some(&b"3"[..]));
        assert_eq!(find(ENVIRON, "WORK"), None);
        assert_eq!(find(ENVIRON, "MISSING"), None);
    }

    #[test]
    fn an_empty_value_is_still_set() {
        assert_eq!(find(ENVIRON, "EMPTY"), Some(&b""[..]));
    }

    #[test]
    fn reads_our_own_environment() {
        let pid = i32::try_from(std::process::id()).unwrap_or_default();
        let path = std::env::var("PATH").ok();
        assert_eq!(variable(pid, "PATH"), path);
    }
}
