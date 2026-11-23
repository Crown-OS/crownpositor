//! Starts the compositor nested and checks `crownos_context_v1` against the
//! grants in `context_access.ron`: two clients of different scope, one
//! without a grant, and a revoke that cuts a live client. Needs a host
//! Wayland display and the probes built first:
//! `cargo build --example context_probe --example multi_monitor_probe &&
//! cargo test --test nested_context_access -- --ignored`.

use std::{
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{Receiver, channel},
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, bail, ensure};

const STARTUP: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(200);
const APP_ID: &str = "nested-context-test";
/// No X11 bridge, so the nested session does not take an X display, which
/// every session on the machine shares.
const NO_X11_BRIDGE: &str = "/nonexistent/xwayland-satellite";

struct KilledOnDrop(Child);

impl Drop for KilledOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A probe and the lines it printed so far.
struct Probe {
    _child: KilledOnDrop,
    lines: Receiver<String>,
    heard: Vec<String>,
}

impl Probe {
    fn heard(&mut self) -> &[String] {
        self.heard.extend(self.lines.try_iter());
        &self.heard
    }

    fn has_heard(&mut self, line: &str) -> bool {
        self.heard().iter().any(|heard| heard == line)
    }
}

struct Session {
    runtime: PathBuf,
    config: PathBuf,
    socket: String,
}

impl Session {
    fn client(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("WAYLAND_DISPLAY", &self.socket)
            .env("CROWN_CONFIG_DIR", &self.config)
            .env_remove("WORKSPACE")
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        command
    }
}

fn wait_until(what: &str, mut ready: impl FnMut() -> bool) -> anyhow::Result<()> {
    let started = Instant::now();
    while started.elapsed() < STARTUP {
        if ready() {
            return Ok(());
        }
        sleep(POLL);
    }
    bail!("timed out waiting for {what}")
}

fn grants(probes: &[(&Path, &str)]) -> String {
    let grants: Vec<String> = probes
        .iter()
        .map(|(path, scopes)| {
            format!(
                "\"exe:{}\": (scopes: [{scopes}], expires: Never)",
                path.display()
            )
        })
        .collect();
    format!(
        "(ask_on_first_use: false, host_default: Deny, migrated_agents: true, grants: {{{}}})\n",
        grants.join(", ")
    )
}

#[test]
#[ignore = "needs a host Wayland display"]
fn each_client_gets_its_grant_and_a_revoke_cuts_a_live_client() -> anyhow::Result<()> {
    let host_runtime = std::env::var_os("XDG_RUNTIME_DIR").context("no XDG_RUNTIME_DIR")?;
    let host_display = std::env::var_os("WAYLAND_DISPLAY").context("no host display")?;
    let host_socket = Path::new(&host_runtime).join(host_display);

    let compositor = PathBuf::from(env!("CARGO_BIN_EXE_compositor"));
    let examples = compositor
        .parent()
        .context("the compositor binary has no directory")?
        .join("examples");
    let context_probe = examples.join("context_probe");
    let window_probe = examples.join("multi_monitor_probe");
    ensure!(
        context_probe.exists() && window_probe.exists(),
        "build the probes first: cargo build --example context_probe --example multi_monitor_probe"
    );

    let scratch = PathBuf::from(format!("/tmp/w1b-{}", std::process::id()));
    let runtime = scratch.join("rt");
    let config = scratch.join("config");
    let programs = scratch.join("bin");
    for directory in [&runtime, &config, &programs] {
        std::fs::create_dir_all(directory)?;
    }
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))?;
    let with_titles = programs.join("probe-with-titles");
    let without_titles = programs.join("probe-without-titles");
    let without_grant = programs.join("probe-without-grant");
    for copy in [&with_titles, &without_titles, &without_grant] {
        std::fs::copy(&context_probe, copy)?;
    }
    let access = config.join("context_access.ron");
    std::fs::write(
        &access,
        grants(&[
            (&with_titles, "Metadata, Text"),
            (&without_titles, "Metadata"),
        ]),
    )?;

    let _compositor = KilledOnDrop(
        Command::new(&compositor)
            .env("CROWN_BACKEND", "winit")
            .env("CROWN_CONFIG_DIR", &config)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("WAYLAND_DISPLAY", &host_socket)
            .env("WORKSPACE", "headless")
            .env("CROWN_XWAYLAND_SATELLITE", NO_X11_BRIDGE)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    wait_until("the nested socket", || {
        std::fs::read_dir(&runtime).is_ok_and(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().starts_with("wayland-"))
        })
    })?;
    let socket = std::fs::read_dir(&runtime)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .find(|name| name.starts_with("wayland-") && !name.ends_with(".lock"))
        .context("no nested socket")?;
    let session = Session {
        runtime,
        config,
        socket,
    };

    let _window = KilledOnDrop(
        session
            .client(&window_probe)
            .args(["spawn", "1", "--app-id", APP_ID])
            .stdout(Stdio::null())
            .spawn()?,
    );
    let mut probes = Vec::new();
    for program in [&with_titles, &without_titles, &without_grant] {
        let mut child = session.client(program).stdout(Stdio::piped()).spawn()?;
        let stdout = child.stdout.take().context("no probe stdout")?;
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        probes.push(Probe {
            _child: KilledOnDrop(child),
            lines,
            heard: Vec::new(),
        });
    }
    let app_line = format!("app_id {APP_ID}");
    wait_until("both granted probes to see the window", || {
        let titles = probes[0].has_heard(&app_line);
        let metadata = probes[1].has_heard(&app_line);
        titles && metadata
    })?;
    wait_until("the probe without a grant to be refused", || {
        probes[2].has_heard("revoked")
    })?;
    assert!(probes[0].has_heard("title"), "{:?}", probes[0].heard());
    assert!(!probes[1].has_heard("title"), "{:?}", probes[1].heard());
    assert!(!probes[2].has_heard(&app_line), "{:?}", probes[2].heard());

    std::fs::write(&access, grants(&[(&without_titles, "Metadata")]))?;
    wait_until("the revoke to reach the probe with titles", || {
        probes[0].has_heard("revoked")
    })?;
    assert!(!probes[1].has_heard("revoked"), "{:?}", probes[1].heard());

    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}
