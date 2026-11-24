//! Turning `compositor.startup` command lines into argv.
//!
//! The compositor execs directly — there is no shell in between — so the
//! splitting has to happen here. It is deliberately not a shell: no globbing,
//! no `$VAR`, no `&&`. Quotes are honoured because wallpaper paths have spaces
//! in them, and a user who writes `swaybg -i "~/My Pictures/wall.png"` should
//! not have to learn why that failed.

/// Splits one command line into argv, honouring `"…"` and `'…'`.
///
/// A quote groups; it never survives into the argument. An unterminated quote
/// runs to the end of the line rather than erroring — the user's intent is
/// unambiguous and a startup entry is not worth failing the load over.
pub fn split_argv(line: &str) -> Vec<String> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;

    for ch in line.chars() {
        match quote {
            Some(open) if ch == open => quote = None,
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                // `""` on its own is a real, empty argument.
                started = true;
            }
            None if ch.is_whitespace() => {
                if started {
                    argv.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(ch);
                started = true;
            }
        }
    }

    if started {
        argv.push(current);
    }
    argv
}

/// Every startup entry as argv, with blank lines dropped.
pub fn commands(lines: &[String]) -> Vec<Vec<String>> {
    lines
        .iter()
        .map(|line| split_argv(line))
        .filter(|argv| !argv.is_empty())
        .collect()
}

/// The session programs, then the user's own programs that are not session
/// programs too. A home from an earlier release can still list a session
/// program in `startup`, and it must not start two times.
pub fn session_then_user_commands(session: &[String], user: &[String]) -> Vec<Vec<String>> {
    let mut all = commands(session);
    for argv in commands(user) {
        if !all.contains(&argv) {
            all.push(argv);
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_command_splits_on_whitespace() {
        assert_eq!(split_argv("swaybg -m fill"), ["swaybg", "-m", "fill"]);
        assert_eq!(split_argv("  crownbar  "), ["crownbar"]);
    }

    #[test]
    fn quotes_keep_a_path_with_spaces_in_one_argument() {
        assert_eq!(
            split_argv(r#"swaybg -i "/home/me/My Pictures/wall.png""#),
            ["swaybg", "-i", "/home/me/My Pictures/wall.png"]
        );
        assert_eq!(
            split_argv("sh -c 'echo hello world'"),
            ["sh", "-c", "echo hello world"]
        );
    }

    #[test]
    fn a_quote_groups_without_separating() {
        // Adjacent quoted and bare text is one argument, as in a shell.
        assert_eq!(split_argv(r#"--css="a b".conf"#), ["--css=a b.conf"]);
        assert_eq!(split_argv(r#"foo ""#), ["foo", ""]);
    }

    #[test]
    fn an_unterminated_quote_runs_to_the_end() {
        assert_eq!(split_argv(r#"foo "bar baz"#), ["foo", "bar baz"]);
    }

    #[test]
    fn blank_entries_are_dropped_rather_than_spawned() {
        let lines = vec!["crownbar".to_string(), "   ".to_string(), String::new()];
        assert_eq!(commands(&lines), [vec!["crownbar".to_string()]]);
    }

    #[test]
    fn a_session_program_in_the_user_list_starts_one_time() {
        let session = vec![
            "crownpaper".to_string(),
            "crownos-theme-engine watch".to_string(),
        ];
        let user = vec![
            "crownpaper".to_string(),
            "crownos-theme-engine  watch".to_string(),
            "kitty".to_string(),
        ];
        assert_eq!(
            session_then_user_commands(&session, &user),
            [
                vec!["crownpaper".to_string()],
                vec!["crownos-theme-engine".to_string(), "watch".to_string()],
                vec!["kitty".to_string()],
            ]
        );
    }
}
