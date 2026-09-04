//! Live keybinds, end to end.
//!
//! An edit to `keybinds.custom_keybinds` has to reach the compositor while it
//! is running, and an edit to anything else in that file has to not — which is
//! the whole reason the watch subscribes per key rather than per section.
//!
//! One test function, because `CROWN_CONFIG_DIR` is process-global and cargo
//! runs test functions on parallel threads.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use config::{Binding, Update, Watch};
use crownos_config::{CONFIG_DIR_ENV, Keybind, Keybinds, path_for};

/// How long to wait for an update that should arrive.
const DELIVERED: Duration = Duration::from_secs(5);
/// How long to wait before concluding an update will never arrive.
const SILENT: Duration = Duration::from_millis(400);

#[test]
fn keybind_edits_reach_a_running_compositor() {
    let dir = std::env::temp_dir().join(format!("crownpositor-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp config dir");
    // SAFETY: this is the only test in the binary, and nothing else has been
    // spawned yet.
    unsafe { std::env::set_var(CONFIG_DIR_ENV, &dir) };

    let (sender, updates) = channel();
    let _watch = Watch::spawn(move |update| {
        let _ = sender.send(update);
    });

    let mut keybinds = Keybinds {
        custom_keybinds: vec![row("Super+B", "spawn firefox")],
        ..Keybinds::default()
    };
    write(&keybinds);
    assert_eq!(
        keybinds_from(&updates),
        keybinds.custom_keybinds,
        "an edited keybind should reach the compositor"
    );

    keybinds.launcher = Keybind::NONE;
    write(&keybinds);
    assert!(
        updates.recv_timeout(SILENT).is_err(),
        "another key in the same file must not rebuild the binding table"
    );

    keybinds.custom_keybinds.push(row("Super+L", "none"));
    write(&keybinds);
    assert_eq!(keybinds_from(&updates), keybinds.custom_keybinds);

    unsafe { std::env::remove_var(CONFIG_DIR_ENV) };
    let _ = std::fs::remove_dir_all(&dir);
}

fn row(keys: &str, action: &str) -> Binding {
    Binding {
        keys: keys.to_owned(),
        action: action.to_owned(),
    }
}

/// Writes the file directly rather than through `save`, which is what another
/// app or `$EDITOR` does — and what the watcher's echo suppression lets past.
fn write(keybinds: &Keybinds) {
    let text = ron::ser::to_string_pretty(keybinds, ron::ser::PrettyConfig::default())
        .expect("serialise keybinds");
    std::fs::write(path_for(Keybinds::SECTION), text).expect("write keybinds.ron");
}

fn keybinds_from(updates: &Receiver<Update>) -> Vec<Binding> {
    match updates.recv_timeout(DELIVERED) {
        Ok(Update::CustomKeybinds(custom)) => custom,
        other => panic!("expected a keybind update, got {other:?}"),
    }
}
