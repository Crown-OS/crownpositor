use tracing_subscriber::{EnvFilter, filter::Directive};

/// smithay traces the name of every key pressed, a typed password included;
/// that stays out of the log however verbose `RUST_LOG` asks it to be.
const KEYSTROKES: &str = "smithay::input::keyboard=debug";

/// Installs the global tracing subscriber, honouring `RUST_LOG` when it is set.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let filter = match KEYSTROKES.parse::<Directive>() {
        Ok(directive) => filter.add_directive(directive),
        Err(_) => filter,
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
