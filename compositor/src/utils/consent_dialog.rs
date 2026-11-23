//! A consent question in the root-owned `crownos-portal-dialog`, which the
//! compositor starts itself. The dialog prints the id of the chosen button and
//! exits 0, or exits 1 when the user declines. No D-Bus service is in the
//! path, and synthetic input never reaches the dialog's surface.

use std::{path::Path, process::Stdio, time::Duration};

use tokio::{process::Child, sync::oneshot};

/// How long the user has to answer.
pub const ANSWER_DEADLINE: Duration = Duration::from_secs(120);

/// One question: a title, a message, one button per choice and a decline
/// button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentQuestion {
    pub dangerous: bool,
    pub title: String,
    pub message: String,
    pub decline: &'static str,
    /// The id that the dialog prints, and the button label.
    pub choices: Vec<(&'static str, &'static str)>,
}

/// What the user did. A choice counts only when the dialog that printed it
/// also showed its surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentAnswer {
    Chosen(&'static str),
    Declined,
    TimedOut,
    /// The question was withdrawn while the dialog was open.
    Withdrawn,
    /// No dialog could run, or it answered without showing itself.
    Unavailable,
}

impl ConsentQuestion {
    /// The dialog process for this question, killed when it is dropped.
    pub fn command(&self, executable: &Path) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(executable);
        command
            .args(["--icon", if self.dangerous { "warning" } else { "access" }])
            .args(["--title", &self.title, "--message", &self.message])
            .args(["--cancel", self.decline]);
        for (id, label) in &self.choices {
            command.arg("--choice").arg(format!("{id}={label}"));
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }

    /// The choice that the dialog printed, if it is one of this question's.
    pub fn choice(&self, printed: &str) -> Option<&'static str> {
        self.choices
            .iter()
            .map(|(id, _)| *id)
            .find(|id| *id == printed)
    }
}

/// What the dialog printed and how it ended, before the compositor checks
/// that it showed itself. The dialog is killed on a timeout or a withdrawal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogExit {
    Printed(String),
    Declined,
    TimedOut,
    Withdrawn,
}

pub async fn wait_for_dialog(child: Child, withdrawn: oneshot::Receiver<()>) -> DialogExit {
    tokio::select! {
        finished = tokio::time::timeout(ANSWER_DEADLINE, child.wait_with_output()) => match finished {
            Ok(Ok(output)) if output.status.success() => {
                last_line(&output.stdout).map_or(DialogExit::Declined, |line| DialogExit::Printed(line.to_owned()))
            }
            Ok(_) => DialogExit::Declined,
            Err(_) => DialogExit::TimedOut,
        },
        _ = withdrawn => DialogExit::Withdrawn,
    }
}

fn last_line(output: &[u8]) -> Option<&str> {
    std::str::from_utf8(output)
        .ok()?
        .lines()
        .last()
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question() -> ConsentQuestion {
        ConsentQuestion {
            dangerous: true,
            title: "Type a secret?".into(),
            message: "App: Firefox".into(),
            decline: "Deny",
            choices: vec![("grant", "Allow")],
        }
    }

    #[test]
    fn the_command_line_is_the_dialogs_own_format() {
        let command = question().command(Path::new("/usr/lib/crownos-portal-dialog"));
        let arguments: Vec<_> = command
            .as_std()
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            arguments,
            [
                "--icon",
                "warning",
                "--title",
                "Type a secret?",
                "--message",
                "App: Firefox",
                "--cancel",
                "Deny",
                "--choice",
                "grant=Allow"
            ]
        );
    }

    #[test]
    fn only_a_known_choice_on_the_last_line_counts() {
        assert_eq!(question().choice("grant"), Some("grant"));
        assert_eq!(question().choice("granted"), None);
        assert_eq!(last_line(b"noise\ngrant\n"), Some("grant"));
        assert_eq!(last_line(&[0xff]), None);
    }
}
