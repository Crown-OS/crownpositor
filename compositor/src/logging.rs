use tracing_subscriber::EnvFilter;

/// Installs the global tracing subscriber, honouring `RUST_LOG` when it is set.
pub fn init() {
    match EnvFilter::try_from_default_env() {
        Ok(filter) => tracing_subscriber::fmt().with_env_filter(filter).init(),
        Err(_) => tracing_subscriber::fmt().init(),
    }
}
