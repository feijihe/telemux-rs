//! 基于 `tracing` + `tracing-subscriber` 的日志设置。
//!
//! 输出两个通道（阶段 6.1）：
//! - **stdout**：按 `RUST_LOG`（或传入的级别）过滤，tty 上使用 ansi 颜色。
//! - **滚动文件**（可选，配置 `log_dir`）：按日轮转、保留最近 N 个文件，
//!   记录 **TRACE 级全量**（排障用），不受 stdout 过滤级别限制。
//!
//! `tracing-log` 将任何基于 `log` 的输出（例如来自 tokio-modbus）桥接到
//! tracing 管道中。

use std::io::IsTerminal;
use std::path::PathBuf;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// 滚动文件日志配置（来自 `[general]` 配置段）。
#[derive(Debug, Clone)]
pub struct LogFileConfig {
    /// 日志目录。
    pub dir: PathBuf,
    /// 文件名前缀（如 "telemux" -> telemux.YYYY-MM-DD.log）。
    pub prefix: String,
    /// 保留的最大文件数（滚动删除最旧的）。
    pub max_files: usize,
}

/// 日志 guard：持有非阻塞文件写入者的 worker，进程退出前 drop 以 flush。
pub struct LogGuard {
    _file_guard: Option<WorkerGuard>,
}

/// 安装 tracing 订阅器。
///
/// - stdout 层：`RUST_LOG` 优先，否则 `max_level`，tty 上 ansi 颜色。
/// - 文件层（`file` 为 `Some` 时）：按日轮转 + `max_files` 保留，全量 TRACE。
pub fn init(
    max_level: Level,
    file: Option<LogFileConfig>,
) -> Result<LogGuard, Box<dyn std::error::Error + Send + Sync>> {
    let filter = EnvFilter::from_default_env().add_directive(max_level.into());

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_ansi(std::io::stdout().is_terminal())
        .with_filter(filter);

    let (file_layer, guard) = match file {
        Some(cfg) => {
            let builder = tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix(&cfg.prefix)
                .max_log_files(cfg.max_files);
            let appender = builder.build(&cfg.dir).map_err(|e| {
                std::io::Error::other(format!(
                    "build log appender in `{}`: {e}",
                    cfg.dir.display()
                ))
            })?;
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(non_blocking)
                .with_filter(LevelFilter::TRACE);
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };

    let registry = tracing_subscriber::registry().with(stdout_layer);
    match file_layer {
        Some(layer) => registry.with(layer).try_init()?,
        None => registry.try_init()?,
    }
    Ok(LogGuard {
        _file_guard: guard,
    })
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
    use std::time::Duration;

    #[test]
    fn parses_level_names() {
        assert_eq!(level_from_str("info"), Some(Level::INFO));
        assert_eq!(level_from_str("WARN"), Some(Level::WARN));
        assert_eq!(level_from_str("bogus"), None);
    }

    #[test]
    fn file_logging_writes_rotated_files() {
        // 全局 subscriber 只能安装一次；若已被其它测试安装则跳过。
        let dir = std::env::temp_dir().join(format!("telemux-log-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg = LogFileConfig {
            dir: dir.clone(),
            prefix: "telemux".into(),
            max_files: 3,
        };
        let Ok(_guard) = init(Level::INFO, Some(cfg)) else {
            return; // 已安装（其它测试）——跳过
        };
        tracing::info!("file logging test message");
        // WorkerGuard drop 时 flush；等待片刻确保写入完成。
        drop(_guard);
        std::thread::sleep(Duration::from_millis(200));

        let files: Vec<_> = std::fs::read_dir(&dir)
            .expect("log dir readable")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("telemux"))
            .collect();
        assert!(!files.is_empty(), "expected at least one rotated log file");

        // 内容应包含测试消息。
        let path = files[0].path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(text.contains("file logging test message"), "got: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
