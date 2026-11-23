//! Starts the compositor nested with the portal dialog and checks that a
//! `zwp_virtual_keyboard_v1` key does not reach it once the compositor knows
//! it as a consent dialog, and does reach it otherwise. Needs a host Wayland
//! display, the probe built first and a dialog build named in
//! `CROWNOS_PORTAL_DIALOG`:
//! `cargo build --example virtual_keyboard_probe &&
//! CROWNOS_PORTAL_DIALOG=…/crownos-portal-dialog cargo test --test nested_consent_dialog -- --ignored`.

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};

const STARTUP: Duration = Duration::from_secs(60);
const DIALOG_MAPS: Duration = Duration::from_secs(8);
const POLL: Duration = Duration::from_millis(200);
/// Escape, and the key it moves to when the seat swaps Caps Lock and Escape.
const ESCAPE: [&str; 2] = ["1", "58"];
/// No X11 bridge, so the nested session takes no X display.
const NO_X11_BRIDGE: &str = "/nonexistent/xwayland-satellite";

struct KilledOnDrop(Child);

impl Drop for KilledOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Session {
    scratch: PathBuf,
    runtime: PathBuf,
    socket: String,
    _compositor: KilledOnDrop,
}

impl Session {
    fn start(name: &str, trusted_dialog: Option<&Path>) -> anyhow::Result<Self> {
        let host_runtime = std::env::var_os("XDG_RUNTIME_DIR").context("no XDG_RUNTIME_DIR")?;
        let host_display = std::env::var_os("WAYLAND_DISPLAY").context("no host display")?;
        let scratch = PathBuf::from(format!("/tmp/w1b-{name}-{}", std::process::id()));
        let runtime = scratch.join("rt");
        let config = scratch.join("config");
        std::fs::create_dir_all(&runtime)?;
        std::fs::create_dir_all(&config)?;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))?;
        let mut command = Command::new(env!("CARGO_BIN_EXE_compositor"));
        command
            .env("CROWN_BACKEND", "winit")
            .env("CROWN_CONFIG_DIR", &config)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env(
                "WAYLAND_DISPLAY",
                Path::new(&host_runtime).join(host_display),
            )
            .env("WORKSPACE", "headless")
            .env("CROWN_XWAYLAND_SATELLITE", NO_X11_BRIDGE)
            .env_remove("CROWN_CONSENT_DIALOG")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(dialog) = trusted_dialog {
            command.env("CROWN_CONSENT_DIALOG", dialog);
        }
        let compositor = KilledOnDrop(command.spawn()?);
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
        Ok(Self {
            scratch,
            runtime,
            socket,
            _compositor: compositor,
        })
    }

    fn client(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("WAYLAND_DISPLAY", &self.socket)
            .env_remove("WORKSPACE")
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("LD_PRELOAD")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// Opens the dialog, presses Escape through a virtual keyboard, and
    /// returns whether the dialog is still open afterwards.
    fn dialog_survives_a_virtual_escape(
        &self,
        dialog: &Path,
        probe: &Path,
    ) -> anyhow::Result<bool> {
        let mut opened = KilledOnDrop(
            self.client(dialog)
                .args(["--icon", "access", "--title", "Test", "--message", "Test"])
                .args(["--choice", "grant=Allow", "--cancel", "Don't Allow"])
                .spawn()?,
        );
        sleep(DIALOG_MAPS);
        if opened.0.try_wait()?.is_some() {
            bail!("the dialog closed before any key");
        }
        let status = self.client(probe).args(ESCAPE).status()?;
        if !status.success() {
            bail!("the virtual keyboard probe failed");
        }
        sleep(Duration::from_secs(2));
        Ok(opened.0.try_wait()?.is_none())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

#[test]
#[ignore = "needs a host Wayland display and a dialog build"]
fn a_virtual_keyboard_cannot_answer_a_consent_dialog() -> anyhow::Result<()> {
    let dialog = PathBuf::from(
        std::env::var_os("CROWNOS_PORTAL_DIALOG").context("set CROWNOS_PORTAL_DIALOG")?,
    )
    .canonicalize()?;
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_compositor"))
        .parent()
        .context("the compositor binary has no directory")?
        .join("examples/virtual_keyboard_probe");

    let untrusted = Session::start("vk-plain", None)?;
    assert!(
        !untrusted.dialog_survives_a_virtual_escape(&dialog, &probe)?,
        "the probe must reach an ordinary overlay, or the check below proves nothing"
    );
    drop(untrusted);

    let consent = Session::start("vk-consent", Some(&dialog))?;
    assert!(consent.dialog_survives_a_virtual_escape(&dialog, &probe)?);
    Ok(())
}
