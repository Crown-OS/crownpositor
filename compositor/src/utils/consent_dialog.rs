//! A consent question in the root-owned `crownos-portal-dialog`, which the
//! compositor starts itself. Only the dialog's exit status is read, as
//! `crownos_config::consent_exit` encodes it. No D-Bus service is in the
//! path, and synthetic input never reaches the dialog's surface.

use std::{path::Path, process::Stdio, time::Duration};

use crownos_config::consent_exit::QuestionShape;
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
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }

    /// The choice in the dialog's exit status, if it is one of this question's.
    pub fn choice(&self, status: Option<i32>) -> Option<&'static str> {
        let shape = QuestionShape {
            choices: self.choices.len(),
            options: Vec::new(),
        };
        let answer = shape.decode(status)?;
        self.choices.get(answer.choice).map(|(id, _)| *id)
    }
}

/// How the dialog ended, before the compositor checks that it showed itself.
/// The dialog is killed on a timeout or a withdrawal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogExit {
    /// Its exit status; `None` when a signal ended it.
    Exited(Option<i32>),
    Failed,
    TimedOut,
    Withdrawn,
}

pub async fn wait_for_dialog(mut child: Child, withdrawn: oneshot::Receiver<()>) -> DialogExit {
    tokio::select! {
        finished = tokio::time::timeout(ANSWER_DEADLINE, child.wait()) => match finished {
            Ok(Ok(status)) => DialogExit::Exited(status.code()),
            Ok(Err(_)) => DialogExit::Failed,
            Err(_) => DialogExit::TimedOut,
        },
        _ = withdrawn => DialogExit::Withdrawn,
    }
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
    fn only_a_status_of_this_question_is_a_choice() {
        assert_eq!(question().choice(Some(16)), Some("grant"));
        for status in [Some(17), Some(0), Some(1), Some(2), None] {
            assert_eq!(question().choice(status), None, "{status:?}");
        }
    }
}
