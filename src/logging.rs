//! 基于 `tracing` + `tracing-subscriber` 的日志设置。
//!
//! 设置了 `RUST_LOG` 时使用它（通过 `EnvFilter`），否则使用传入的日志级别。
//! `tracing-log` 将任何基于 `log` 的输出（例如来自 tokio-modbus）桥接到
//! tracing 管道中。

use tracing::Level;
use tracing_subscriber::EnvFilter;

/// 安装 tracing 订阅器（格式：文本输出到 stdout，tty 上使用 ansi 颜色）。
pub fn init(
    max_level: Level,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let filter = EnvFilter::from_default_env().add_directive(max_level.into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
}

/// 解析日志级别名称（供 CLI `--log-level` 参数使用）。
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
