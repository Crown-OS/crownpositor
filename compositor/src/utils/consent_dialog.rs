//! A consent question in the root-owned `crownos-portal-dialog`, which the
//! compositor starts itself. The question and its answer are
//! `crownos_config::consent_dialog`'s: the same arguments as the portal, and
//! only the exit status is read. No D-Bus service is in the path, and
//! synthetic input never reaches the dialog's surface.

use std::{path::Path, process::Stdio, time::Duration};

use crownos_config::consent_dialog::{ConsentAnswer, ConsentQuestion};
use tokio::{process::Child, sync::oneshot};

/// How long the user has to answer.
pub const ANSWER_DEADLINE: Duration = Duration::from_secs(120);

/// What came of a question. An answer counts only when the dialog that gave
/// it also showed its surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentOutcome {
    Answered(ConsentAnswer),
    TimedOut,
    /// The question was withdrawn while the dialog was open.
    Withdrawn,
    /// No dialog could run, it asked nobody, or it allowed without showing itself.
    Unavailable,
}

impl ConsentOutcome {
    pub fn granted(&self) -> bool {
        matches!(self, Self::Answered(answer) if answer.granted)
    }
}

/// The dialog process for `question`, killed when it is dropped.
pub fn dialog_command(question: &ConsentQuestion, executable: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(executable);
    command
        .args(question.arguments())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
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

    #[test]
    fn the_command_line_is_the_shared_one() {
        let question = ConsentQuestion {
            dangerous: true,
            title: "Type a secret?".into(),
            message: "App: Firefox".into(),
            grant: "Allow".into(),
            deny: "Deny".into(),
            options: Vec::new(),
        };
        let command = dialog_command(&question, Path::new("/usr/lib/crownos-portal-dialog"));
        let arguments: Vec<String> = command
            .as_std()
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(arguments, question.arguments());
    }
}
