//! Logging setup on top of `tracing` + `tracing-subscriber`.
//!
//! Uses `RUST_LOG` when set (via `EnvFilter`), otherwise the level passed in.
//! `tracing-log` bridges any `log`-based output (e.g. from tokio-modbus) into
//! the tracing pipeline.

use tracing::Level;
use tracing_subscriber::EnvFilter;

/// Install the tracing subscriber (format: text to stdout, ansi colors on tty).
pub fn init(
    max_level: Level,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let filter = EnvFilter::from_default_env().add_directive(max_level.into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
}

/// Parse a level name (used by the CLI `--log-level` flag).
pub fn level_from_str(s: &str) -> Option<Level> {
    match s.to_ascii_lowercase().as_str() {
        "trace" => Some(Level::TRACE),
        "debug" => Some(Level::DEBUG),
        "info" => Some(Level::INFO),
        "warn" | "warning" => Some(Level::WARN),
        "error" => Some(Level::ERROR),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_level_names() {
        assert_eq!(level_from_str("info"), Some(Level::INFO));
        assert_eq!(level_from_str("WARN"), Some(Level::WARN));
        assert_eq!(level_from_str("bogus"), None);
    }
}
