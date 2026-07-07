use crate::config::LogFormat;
use tracing_subscriber::{EnvFilter, fmt};

/// Initialize the global tracing subscriber.
///
/// Call once at startup before any tracing macros are used.
pub fn init_logging(format: &LogFormat, level: &str) {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));

    match format {
        LogFormat::Json => {
            fmt()
                .json()
                .with_env_filter(filter)
                .with_current_span(false)
                .init();
        }
        LogFormat::Pretty => {
            fmt().pretty().with_env_filter(filter).init();
        }
    }
}
