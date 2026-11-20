//! Drives the nested multi-monitor harness from outside: opens windows,
//! and prints where the compositor put them and which workspaces each
//! output has.
//!
//! ```text
//! cargo run -p compositor --example multi_monitor_probe -- spawn 2 --app-id probe [--workspace 2] \
//!     [--fullscreen-on OUTPUT | --fullscreen-later-on OUTPUT] [--decoration server]
//! cargo run -p compositor --example multi_monitor_probe -- placements
//! cargo run -p compositor --example multi_monitor_probe -- move APP_ID_OR_WINDOW OUTPUT
//! cargo run -p compositor --example multi_monitor_probe -- workspaces
//! cargo run -p compositor --example multi_monitor_probe -- mode [OUTPUT [tiling|floating]]
//! ```
//!
//! `--fullscreen-on` asks for fullscreen on OUTPUT before the window maps,
//! `--fullscreen-later-on` once it has drawn its first frame. `--decoration
//! server` asks the compositor to draw the title bar, which a drag can hold.
//!
//! `placements` and `move` bind `crownos_window_control_manager_v1` and `mode` binds
//! `crownos_workspace_mode_manager_v1`, which the compositor offers only to
//! executables that `agents.ron` trusts.

mod mode;
mod move_to_output;
mod placements;
mod spawn;
mod workspaces;

use anyhow::{Context, bail};

use spawn::{Decoration, Fullscreen};

enum Command {
    Spawn {
        count: usize,
        app_id: String,
        workspace: Option<u32>,
        fullscreen: Option<Fullscreen>,
        decoration: Decoration,
    },
    Placements,
    Move {
        window: String,
        output: String,
    },
    Workspaces,
    Mode {
        output: Option<String>,
        mode: Option<String>,
    },
}

fn parse(mut arguments: impl Iterator<Item = String>) -> anyhow::Result<Command> {
    let usage = "usage: multi_monitor_probe spawn N --app-id X [--workspace K] \
        [--fullscreen-on OUTPUT | --fullscreen-later-on OUTPUT] [--decoration server] | placements \
        | move APP_ID_OR_WINDOW OUTPUT | workspaces | mode [OUTPUT [tiling|floating]]";
    match arguments.next().as_deref() {
        Some("spawn") => {
            let count = arguments
                .next()
                .context(usage)?
                .parse()
                .context("the window count must be a number")?;
            let mut app_id = None;
            let mut workspace = None;
            let mut fullscreen = None;
            let mut decoration = Decoration::Client;
            while let Some(flag) = arguments.next() {
                let value = arguments
                    .next()
                    .with_context(|| format!("{flag} needs a value"))?;
                match flag.as_str() {
                    "--app-id" => app_id = Some(value),
                    "--workspace" => {
                        workspace = Some(value.parse().context("the workspace must be a number")?)
                    }
                    "--fullscreen-on" => fullscreen = Some(Fullscreen::BeforeMap(value)),
                    "--fullscreen-later-on" => fullscreen = Some(Fullscreen::AfterMap(value)),
                    "--decoration" => {
                        decoration = match value.as_str() {
                            "server" => Decoration::Server,
                            "client" => Decoration::Client,
                            other => bail!("unknown decoration {other}; {usage}"),
                        }
                    }
                    other => bail!("unknown flag {other}; {usage}"),
                }
            }
            Ok(Command::Spawn {
                count,
                app_id: app_id.context("spawn needs --app-id")?,
                workspace,
                fullscreen,
                decoration,
            })
        }
        Some("placements") => Ok(Command::Placements),
        Some("move") => Ok(Command::Move {
            window: arguments.next().context(usage)?,
            output: arguments.next().context(usage)?,
        }),
        Some("workspaces") => Ok(Command::Workspaces),
        Some("mode") => Ok(Command::Mode {
            output: arguments.next(),
            mode: arguments.next(),
        }),
        _ => bail!(usage),
    }
}

fn main() -> anyhow::Result<()> {
    match parse(std::env::args().skip(1))? {
        Command::Spawn {
            count,
            app_id,
            workspace,
            fullscreen,
            decoration,
        } => spawn::run(count, &app_id, workspace, fullscreen.as_ref(), decoration),
        Command::Placements => placements::run(),
        Command::Move { window, output } => move_to_output::run(&window, &output),
        Command::Workspaces => workspaces::run(),
        Command::Mode { output, mode } => mode::run(output.as_deref(), mode.as_deref()),
    }
}
