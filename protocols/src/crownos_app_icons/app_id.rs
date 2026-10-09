/// Whether an icon registered for `icon` stands for a window whose app_id is
/// `window`.
///
/// Launchers know applications by their desktop entry, windows name
/// themselves: the two agree up to ASCII case, or the window uses the last
/// component of a reverse-DNS entry name (`org.gnome.Nautilus` as `nautilus`).
pub fn same_app(icon: &str, window: &str) -> bool {
    if icon.is_empty() || window.is_empty() {
        return false;
    }
    icon.eq_ignore_ascii_case(window)
        || icon
            .rsplit_once('.')
            .is_some_and(|(_, last)| last.eq_ignore_ascii_case(window))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_match_regardless_of_case() {
        assert!(same_app("Code", "code"));
        assert!(same_app("org.gnome.Nautilus", "org.gnome.Nautilus"));
    }

    #[test]
    fn a_reverse_dns_entry_matches_its_last_component() {
        assert!(same_app("org.gnome.Nautilus", "nautilus"));
        assert!(!same_app("nautilus", "org.gnome.Nautilus"));
    }

    #[test]
    fn different_or_empty_ids_never_match() {
        assert!(!same_app("firefox", "kitty"));
        assert!(!same_app("", ""));
        assert!(!same_app("org.gnome.", ""));
    }
}
