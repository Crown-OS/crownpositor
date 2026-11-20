//! Starts the compositor nested with two outputs and checks where windows
//! map. Needs a host Wayland display and the probe example built first:
//! `cargo build --example multi_monitor_probe && cargo test --test nested_multi_monitor -- --ignored`.

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, bail, ensure};

/// Never the user's own config: a nested disable would be saved there.
const SCRATCH: &str = "/tmp/crownos-nested-m0";
const NESTED_SOCKET: &str = "wayland-1";
const STARTUP: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(200);
const APP_ID: &str = "nested-multi-monitor-test";

struct KilledOnDrop(Child);

impl Drop for KilledOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Session {
    runtime: PathBuf,
    probe: PathBuf,
}

impl Session {
    fn probe(&self) -> Command {
        let mut command = Command::new(&self.probe);
        command
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("WAYLAND_DISPLAY", NESTED_SOCKET)
            .env_remove("WORKSPACE")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }
}

fn wait_until<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> anyhow::Result<T> {
    let started = Instant::now();
    while started.elapsed() < STARTUP {
        if let Some(value) = ready() {
            return Ok(value);
        }
        sleep(POLL);
    }
    bail!("timed out waiting for {what}")
}

#[test]
#[ignore = "needs a host Wayland display"]
fn windows_map_on_the_first_nested_output() -> anyhow::Result<()> {
    let host_runtime = std::env::var_os("XDG_RUNTIME_DIR").context("no XDG_RUNTIME_DIR")?;
    let host_display = std::env::var_os("WAYLAND_DISPLAY").context("no host display")?;
    let host_socket = Path::new(&host_runtime).join(host_display);

    let compositor = PathBuf::from(env!("CARGO_BIN_EXE_compositor"));
    let probe = compositor
        .parent()
        .context("the compositor binary has no directory")?
        .join("examples/multi_monitor_probe");
    ensure!(
        probe.exists(),
        "build the probe first: cargo build --example multi_monitor_probe"
    );

    let scratch = Path::new(SCRATCH).join(std::process::id().to_string());
    let runtime = scratch.join("run");
    let config = scratch.join("config");
    std::fs::create_dir_all(&runtime)?;
    std::fs::create_dir_all(&config)?;
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))?;
    std::fs::write(
        config.join("agents.ron"),
        format!(
            "(trusted_executables: [{:?}])\n",
            probe.display().to_string()
        ),
    )?;

    let _compositor = KilledOnDrop(
        Command::new(&compositor)
            .env("CROWN_BACKEND", "winit")
            .env("CROWN_NESTED_OUTPUTS", "2")
            .env("CROWN_CONFIG_DIR", &config)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("WAYLAND_DISPLAY", &host_socket)
            .env_remove("CROWN_NESTED_NAMES")
            .env_remove("WORKSPACE")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    wait_until("the nested socket", || {
        runtime.join(NESTED_SOCKET).exists().then_some(())
    })?;

    let session = Session { runtime, probe };
    let _windows = KilledOnDrop(
        session
            .probe()
            .args(["spawn", "2", "--app-id", APP_ID])
            .spawn()?,
    );
    let placements = wait_until("both windows to map", || {
        let output = session.probe().arg("placements").output().ok()?;
        let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.contains(&format!("app_id={APP_ID} ")))
            .map(str::to_owned)
            .collect();
        (lines.len() == 2).then_some(lines)
    })?;

    for line in &placements {
        assert!(line.contains(" output=winit-1 "), "{line}");
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}
