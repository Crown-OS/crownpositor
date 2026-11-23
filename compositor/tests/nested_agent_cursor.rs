//! Starts the compositor nested and checks that a client whose agent cursor grant is gone gets
//! no new cursor on the manager it bound before. Needs a host Wayland display and the probe
//! built first: `cargo build --example agent_cursor_probe &&
//! cargo test --test nested_agent_cursor -- --ignored`.

use std::{
    io::{BufRead as _, BufReader, Write as _},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{Receiver, channel},
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};

const STARTUP: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(200);
const NO_X11_BRIDGE: &str = "/nonexistent/xwayland-satellite";
const NO_SESSION_BUS: &str = "unix:path=/nonexistent/bus";

struct KilledOnDrop(Child);

impl Drop for KilledOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn grants(probe: &Path, scopes: &str) -> String {
    let probe_grant = if scopes.is_empty() {
        String::new()
    } else {
        format!(
            "{:?}: (scopes: [{scopes}], expires: Never)",
            format!("exe:{}", probe.display())
        )
    };
    format!(
        "(ask_on_first_use: false, host_default: Deny, migrated_agents: true, grants: {{{probe_grant}}})\n"
    )
}

fn wait_for(lines: &Receiver<String>, heard: &mut Vec<String>, wanted: &str) -> anyhow::Result<()> {
    let started = Instant::now();
    while started.elapsed() < STARTUP {
        heard.extend(lines.try_iter());
        if heard.iter().any(|line| line.starts_with(wanted)) {
            return Ok(());
        }
        sleep(POLL);
    }
    bail!("the probe never printed {wanted}: {heard:?}")
}

#[test]
#[ignore = "needs a host Wayland display and the agent_cursor_probe example"]
fn a_client_whose_grant_is_gone_gets_no_new_cursor() -> anyhow::Result<()> {
    let built = PathBuf::from(env!("CARGO_BIN_EXE_compositor"))
        .parent()
        .context("the compositor binary has no directory")?
        .join("examples/agent_cursor_probe");
    let host_runtime = std::env::var_os("XDG_RUNTIME_DIR").context("no XDG_RUNTIME_DIR")?;
    let host_display = std::env::var_os("WAYLAND_DISPLAY").context("no host display")?;
    let scratch = PathBuf::from(format!("/tmp/w8-agent-cursor-{}", std::process::id()));
    let runtime = scratch.join("rt");
    let config = scratch.join("config");
    std::fs::create_dir_all(&runtime)?;
    std::fs::create_dir_all(&config)?;
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))?;
    let probe = scratch.join("agent-cursor-probe");
    std::fs::copy(&built, &probe)?;
    let access = config.join("context_access.ron");
    std::fs::write(&access, grants(&probe, "WaylandAgentCursor"))?;

    let _compositor = KilledOnDrop(
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
            .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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

    let mut child = Command::new(&probe)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("WAYLAND_DISPLAY", &socket)
        .env("CROWN_CONFIG_DIR", &config)
        .env_remove("WORKSPACE")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().context("no probe stdin")?;
    let stdout = child.stdout.take().context("no probe stdout")?;
    let _probe = KilledOnDrop(child);
    let (send, lines) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = send.send(line);
        }
    });
    let mut heard = Vec::new();
    wait_for(&lines, &mut heard, "created")?;

    std::fs::write(&access, grants(&probe, ""))?;
    wait_for(&lines, &mut heard, "revoked user")?;
    heard.clear();
    writeln!(stdin, "again")?;
    wait_for(&lines, &mut heard, "revoked refused")?;
    if heard.iter().any(|line| line.starts_with("created")) {
        bail!("a revoked client got a new cursor: {heard:?}");
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}
