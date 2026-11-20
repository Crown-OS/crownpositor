use std::{
    path::PathBuf,
    sync::mpsc::{Sender, channel},
    time::Duration,
};

use crownos_ipc::{RemoteError, ServiceBuilder, blocking};

use crate::crownpositor;

const TIMEOUT: Option<Duration> = Some(Duration::from_secs(5));

struct ForwardingHandler {
    received: Sender<String>,
}

impl crownpositor::Handler for ForwardingHandler {
    fn action(&mut self, spelling: String) -> Result<(), RemoteError> {
        if spelling.starts_with("spawn") {
            return Err(RemoteError::new(RemoteError::UNAUTHORIZED, "refused"));
        }
        self.received
            .send(spelling)
            .map_err(|error| RemoteError::handler(error.to_string()))
    }
}

fn socket_directory() -> PathBuf {
    std::env::temp_dir().join(format!("crownpositor-control-{}", std::process::id()))
}

#[test]
fn a_client_runs_an_action_by_its_spelling() {
    let directory = socket_directory();
    let mut server = ServiceBuilder::new(crownpositor::SERVICE)
        .directory(&directory)
        .build()
        .expect("bind the control socket");
    let (received, spellings) = channel();
    std::thread::spawn(move || {
        let mut handler = ForwardingHandler { received };
        blocking::serve(&mut server, |server, peer, message| {
            crownpositor::dispatch(&mut handler, server, peer, message).map(|()| true)
        })
    });

    let mut client = crownpositor::Client::connect_in(&directory).expect("connect");
    let pending = client
        .action("toggle-workspace-view".into())
        .expect("send the action");
    client
        .wait(pending, TIMEOUT)
        .expect("the action is accepted");
    assert_eq!(
        spellings.recv_timeout(Duration::from_secs(5)).as_deref(),
        Ok("toggle-workspace-view")
    );

    let pending = client.action("spawn foot".into()).expect("send the action");
    match client.wait(pending, TIMEOUT) {
        Err(crownos_ipc::Error::Remote(remote)) => {
            assert_eq!(remote.code, RemoteError::UNAUTHORIZED);
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    std::fs::remove_dir_all(&directory).expect("remove the socket directory");
}
