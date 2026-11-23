//! Starts the compositor nested and asks for a new `wl_keyboard` from a client that has the
//! keyboard focus as a consent dialog, then as a screen locker. The compositor hides the focused
//! surface of such a client from smithay's protocol code, so this once ended the session.
//! Needs a host Wayland display and the probe built first:
//! `cargo build --example keyboard_focus_probe &&
//! cargo test --test nested_keyboard_focus -- --ignored`.

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, bail, ensure};

const STARTUP: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(200);
/// No X11 bridge, so the nested session takes no X display.
const NO_X11_BRIDGE: &str = "/nonexistent/xwayland-satellite";
/// No session bus either, so the nested session never talks to the user's.
const NO_SESSION_BUS: &str = "unix:path=/nonexistent/bus";

struct KilledOnDrop(Child);

impl Drop for KilledOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn probe() -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_BIN_EXE_compositor"))
        .parent()
        .context("the compositor binary has no directory")?
        .join("examples/keyboard_focus_probe")
        .canonicalize()?)
}

#[test]
#[ignore = "needs a host Wayland display and the keyboard_focus_probe example"]
fn a_new_keyboard_from_a_consent_dialog_or_a_locker_keeps_the_session() -> anyhow::Result<()> {
    let probe = probe()?;
    let host_runtime = std::env::var_os("XDG_RUNTIME_DIR").context("no XDG_RUNTIME_DIR")?;
    let host_display = std::env::var_os("WAYLAND_DISPLAY").context("no host display")?;
    let scratch = PathBuf::from(format!("/tmp/w8-keyboard-{}", std::process::id()));
    let runtime = scratch.join("rt");
    let config = scratch.join("config");
    std::fs::create_dir_all(&runtime)?;
    std::fs::create_dir_all(&config)?;
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))?;
    let mut compositor = KilledOnDrop(
        Command::new(env!("CARGO_BIN_EXE_compositor"))
            .env("CROWN_BACKEND", "winit")
            .env("CROWN_CONFIG_DIR", &config)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env(
                "WAYLAND_DISPLAY",
                Path::new(&host_runtime).join(host_display),
            )
            .env("WORKSPACE", "headless")
            .env("CROWN_XWAYLAND_SATELLITE", NO_X11_BRIDGE)
            .env("CROWN_CONSENT_DIALOG", &probe)
            .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(scratch.join("compositor.log"))?)
            .spawn()?,
    );
    let started = Instant::now();
    let socket = loop {
        let found = std::fs::read_dir(&runtime)?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .find(|name| name.starts_with("wayland-") && !name.ends_with(".lock"));
        if let Some(socket) = found {
            break socket;
        }
        if started.elapsed() > STARTUP {
            bail!("the nested compositor did not start");
        }
        sleep(POLL);
    };

    for mode in ["consent", "lock"] {
        let output = Command::new(&probe)
            .arg(mode)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("WAYLAND_DISPLAY", &socket)
            .env_remove("WORKSPACE")
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("LD_PRELOAD")
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .output()?;
        let printed = String::from_utf8_lossy(&output.stdout);
        sleep(Duration::from_millis(300));
        ensure!(
            compositor.0.try_wait()?.is_none(),
            "the compositor ended after a new keyboard in {mode} focus; see {}",
            scratch.join("compositor.log").display()
        );
        ensure!(
            output.status.success() && printed.contains("survived"),
            "the {mode} probe failed: {printed}"
        );
    }
    drop(compositor);
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}
