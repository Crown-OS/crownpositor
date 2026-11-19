use tracing_subscriber::{
    EnvFilter, filter::Directive, fmt, layer::SubscriberExt, util::SubscriberInitExt,
};

/// smithay traces the name of every key pressed, a typed password included;
/// that stays out of the log however verbose `RUST_LOG` asks it to be.
const KEYSTROKES: &str = "smithay::input::keyboard=debug";

/// What `journalctl -t` finds the compositor's log under.
const JOURNAL_IDENTIFIER: &str = "crownpositor";

/// Installs the global tracing subscriber, honouring `RUST_LOG` when it is set.
///
/// Everything goes to the terminal and, when journald is listening, to the
/// journal as well. Launched from a tty, the terminal is the console the
/// compositor is about to cover, so a session that dies on a dark screen
/// leaves its log only in the journal. The fields are journal fields, which
/// only the verbose view shows: `journalctl -b -1 -t crownpositor -o verbose`.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let filter = match KEYSTROKES.parse::<Directive>() {
        Ok(directive) => filter.add_directive(directive),
        Err(_) => filter,
    };
    let journal = tracing_journald::layer()
        .ok()
        .map(|layer| layer.with_syslog_identifier(JOURNAL_IDENTIFIER.to_owned()));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .with(journal)
        .init();
}
