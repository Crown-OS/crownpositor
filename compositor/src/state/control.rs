//! The `crownpositor` control socket: apps such as Settings run compositor
//! actions by their keybinding spelling.
//!
//! Only the main loop knows whether the session is locked, so an accepted
//! action is answered from there. The verdict travels back to the socket
//! through an outbox, and no tokio worker waits for the main loop.

use crownos_ipc::{
    Error, Kind, Message, MethodMsg, PeerId, RemoteError, Server, ServiceBuilder,
    adapter::tokio::serve_with_outbox, postcard,
};
use crownpositor_control::crownpositor;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::{input::shortcuts::Action, state::State, utils::runtime::TaskSender};

type Verdict = Result<(), RemoteError>;
type PendingReply = (PeerId, u32, Verdict);

impl State {
    pub fn start_control_service(&self) {
        let mut server = match ServiceBuilder::new(crownpositor::SERVICE).build() {
            Ok(server) => server,
            Err(Error::ServiceRunning { .. }) => {
                tracing::info!("another compositor owns the control socket; nested session");
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "no control socket");
                return;
            }
        };
        let (verdicts, mut pending_replies) = unbounded_channel();
        let control = RemoteControl {
            tasks: self.common.tasks.clone(),
            verdicts,
        };
        self.common.tasks.spawn(async move {
            let served = serve_with_outbox(
                &mut server,
                |server, peer, message| {
                    control.dispatch(server, peer, message);
                    Ok(true)
                },
                |context| pending_replies.poll_recv(context),
                send_reply,
            )
            .await;
            if let Err(error) = served {
                tracing::warn!(%error, "control socket stopped");
            }
        });
    }
}

struct RemoteControl {
    tasks: TaskSender,
    verdicts: UnboundedSender<PendingReply>,
}

impl RemoteControl {
    fn dispatch(&self, server: &mut Server, peer: PeerId, message: Message) {
        let Message {
            kind,
            selector,
            req_id,
            payload,
            ..
        } = message;
        if selector != <crownpositor::action as MethodMsg>::SELECTOR {
            if kind == Kind::Request {
                let _ = server.reply_err(peer, req_id, &RemoteError::unknown_method(selector));
            }
            return;
        }
        let decoded = postcard::from_bytes::<crownpositor::action>(&payload);
        let (Kind::Request, Ok(crownpositor::action { spelling })) = (kind, decoded) else {
            server.drop_peer(peer);
            return;
        };
        let action = match permitted_remote_action(&spelling) {
            Ok(action) => action,
            Err(refusal) => {
                let _ = server.reply_err(peer, req_id, &refusal);
                return;
            }
        };
        let verdicts = self.verdicts.clone();
        self.tasks.deliver(move |state| {
            let verdict = session_verdict(state.shell.is_locked());
            let accepted = verdict.is_ok();
            let _ = verdicts.send((peer, req_id, verdict));
            if accepted {
                state.handle_action(action);
            }
        });
    }
}

fn send_reply(server: &mut Server, (peer, req_id, verdict): PendingReply) {
    let _ = match verdict {
        Ok(()) => server.reply(peer, req_id, &[], Vec::new()),
        Err(error) => server.reply_err(peer, req_id, &error),
    };
}

fn permitted_remote_action(spelling: &str) -> Result<Action, RemoteError> {
    let action = spelling
        .parse::<Action>()
        .map_err(|error| RemoteError::handler(error.to_string()))?;
    if matches!(action, Action::Spawn(_) | Action::SwitchVt(_)) {
        return Err(RemoteError::new(
            RemoteError::UNAUTHORIZED,
            format!("`{spelling}` cannot run over the control socket"),
        ));
    }
    Ok(action)
}

fn session_verdict(session_locked: bool) -> Verdict {
    if session_locked {
        Err(RemoteError::new(
            RemoteError::UNAUTHORIZED,
            "the session is locked",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal_code(spelling: &str) -> Option<u32> {
        permitted_remote_action(spelling)
            .err()
            .map(|error| error.code)
    }

    #[test]
    fn session_and_workspace_actions_are_permitted() {
        assert_eq!(
            permitted_remote_action("restart-session").ok(),
            Some(Action::RestartSession)
        );
        assert_eq!(
            permitted_remote_action("toggle-workspace-view").ok(),
            Some(Action::ToggleWorkspaceView)
        );
    }

    #[test]
    fn spawn_and_switch_vt_are_unauthorized() {
        assert_eq!(refusal_code("spawn foot"), Some(RemoteError::UNAUTHORIZED));
        assert_eq!(refusal_code("switch-vt 2"), Some(RemoteError::UNAUTHORIZED));
    }

    #[test]
    fn an_unknown_spelling_is_a_handler_error() {
        assert_eq!(refusal_code("frobnicate"), Some(RemoteError::HANDLER));
    }

    #[test]
    fn a_locked_session_refuses_every_action() {
        let refusal = session_verdict(true).err();
        assert_eq!(
            refusal
                .as_ref()
                .map(|error| (error.code, error.message.as_str())),
            Some((RemoteError::UNAUTHORIZED, "the session is locked"))
        );
        assert!(session_verdict(false).is_ok());
    }
}
