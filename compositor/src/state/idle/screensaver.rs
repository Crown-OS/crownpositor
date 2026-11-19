//! `org.freedesktop.ScreenSaver`: what Chrome, Firefox, VLC, Electron apps
//! and the GTK portal (for Flatpaks) ask to keep the screen on through a
//! video. Nothing is kept here: every call is handed on as a [`Call`].

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use futures_util::StreamExt;
use zbus::{connection, fdo::DBusProxy, interface, message::Header};

const NAME: &str = "org.freedesktop.ScreenSaver";
/// Where clients look for it: freedesktop's path, and KDE's older one.
const PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];

/// What a client asked for, by its unique bus name.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Call {
    Hold {
        cookie: u32,
        owner: String,
    },
    Release {
        cookie: u32,
        owner: String,
    },
    Activity,
    /// The name left the bus, and every hold it had with it.
    Left(String),
}

struct ScreenSaver<F> {
    on_call: F,
    /// Shared by both paths, so a cookie means the same hold on either.
    next_cookie: Arc<AtomicU32>,
}

#[interface(name = "org.freedesktop.ScreenSaver")]
impl<F: Fn(Call) + Send + Sync + 'static> ScreenSaver<F> {
    fn inhibit(
        &self,
        #[zbus(header)] header: Header<'_>,
        application_name: &str,
        reason_for_inhibit: &str,
    ) -> u32 {
        let cookie = self.next_cookie.fetch_add(1, Ordering::Relaxed);
        if let Some(owner) = header.sender().map(ToString::to_string) {
            tracing::info!(application_name, reason_for_inhibit, cookie, "idle held");
            (self.on_call)(Call::Hold { cookie, owner });
        }
        cookie
    }

    fn un_inhibit(&self, #[zbus(header)] header: Header<'_>, cookie: u32) {
        if let Some(owner) = header.sender().map(ToString::to_string) {
            tracing::info!(cookie, "idle released");
            (self.on_call)(Call::Release { cookie, owner });
        }
    }

    fn simulate_user_activity(&self) {
        (self.on_call)(Call::Activity);
    }
}

/// Serves on the bus `builder` connects to until it goes away.
pub(super) async fn serve<F>(builder: connection::Builder<'_>, on_call: F) -> zbus::Result<()>
where
    F: Fn(Call) + Clone + Send + Sync + 'static,
{
    let next_cookie = Arc::new(AtomicU32::new(1));
    let mut builder = builder.name(NAME)?;
    for path in PATHS {
        builder = builder.serve_at(
            path,
            ScreenSaver {
                on_call: on_call.clone(),
                next_cookie: next_cookie.clone(),
            },
        )?;
    }
    let connection = builder.build().await?;
    let mut owners = DBusProxy::new(&connection)
        .await?
        .receive_name_owner_changed()
        .await?;
    while let Some(change) = owners.next().await {
        let Ok(args) = change.args() else {
            continue;
        };
        if args.new_owner().is_none() && args.name().starts_with(':') {
            on_call(Call::Left(args.name().to_string()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
        sync::mpsc,
        time::Duration,
    };

    use super::*;

    #[zbus::proxy(
        interface = "org.freedesktop.ScreenSaver",
        default_service = "org.freedesktop.ScreenSaver",
        default_path = "/ScreenSaver"
    )]
    trait ScreenSaverClient {
        fn inhibit(&self, application_name: &str, reason_for_inhibit: &str) -> zbus::Result<u32>;
        fn un_inhibit(&self, cookie: u32) -> zbus::Result<()>;
    }

    /// A private bus, so the test never meets the session's own.
    struct PrivateBus(Child);

    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.0.kill();
        }
    }

    fn private_bus() -> Option<(PrivateBus, String)> {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        let mut address = String::new();
        BufReader::new(daemon.stdout.take()?)
            .read_line(&mut address)
            .ok()?;
        Some((PrivateBus(daemon), address.trim().to_owned()))
    }

    #[test]
    fn a_hold_is_taken_released_and_dropped_with_its_client() {
        let Some((_bus, address)) = private_bus() else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let (calls, received) = mpsc::channel();
        let next = || received.recv_timeout(Duration::from_secs(5));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a test runtime");
        let owner = runtime.block_on(async {
            let server = connection::Builder::address(address.as_str()).expect("an address");
            tokio::spawn(serve(server, move |call| {
                let _ = calls.send(call);
            }));
            let client = connection::Builder::address(address.as_str())
                .expect("an address")
                .build()
                .await
                .expect("a client");
            let owner = client.unique_name().expect("a unique name").to_string();
            let bus = DBusProxy::new(&client).await.expect("the bus");
            while !bus
                .name_has_owner(NAME.try_into().expect("a bus name"))
                .await
                .expect("an answer")
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let freedesktop = ScreenSaverClientProxy::builder(&client)
                .path("/org/freedesktop/ScreenSaver")
                .expect("a path")
                .build()
                .await
                .expect("a proxy");
            let kde = ScreenSaverClientProxy::new(&client).await.expect("a proxy");

            let cookie = freedesktop
                .inhibit("Chrome", "video")
                .await
                .expect("a cookie");
            assert_eq!(cookie, 1);
            kde.un_inhibit(cookie).await.expect("released");
            kde.inhibit("Chrome", "video").await.expect("a cookie");
            drop(client);
            owner
        });
        let held = |cookie| Call::Hold {
            cookie,
            owner: owner.clone(),
        };
        assert_eq!(next().expect("a hold"), held(1));
        assert_eq!(
            next().expect("a release"),
            Call::Release {
                cookie: 1,
                owner: owner.clone()
            }
        );
        assert_eq!(next().expect("a second hold"), held(2));
        assert_eq!(next().expect("the client leaving"), Call::Left(owner));
    }
}
