use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing::{info, warn};

use telemux::acquisition::AcquisitionManager;
use telemux::config::Config;
use telemux::config_handle::ConfigHandle;
use telemux::domain::RawSample;
use telemux::pipeline::PipelinesCache;
use telemux::store::MetricStore;

#[derive(Parser)]
#[command(
    name = "telemux",
    version,
    about = "PCBA telemetry multi-protocol gateway (Modbus acquisition)"
)]
struct Cli {
    /// Path to the TOML config file
    #[arg(short, long, default_value = "config/example.toml")]
    config: PathBuf,
    /// Override log level: trace|debug|info|warn|error
    #[arg(long)]
    log_level: Option<String>,
    /// Dev dashboard port (dev builds only, default 8080)
    #[arg(long)]
    dashboard_port: Option<u16>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let config = Config::load(&cli.config)?;
    // Runtime handle: mutable (hot-update) in dev builds, read-only in release.
    let handle = ConfigHandle::new(config, cli.config.clone());

    let level = match cli
        .log_level
        .as_deref()
        .and_then(telemux::logging::level_from_str)
    {
        Some(l) => l,
        None => handle.read().general.log_level.as_tracing_level(),
    };
    telemux::logging::init(level).map_err(|e| anyhow::anyhow!("init logger: {e}"))?;

    info!(
        "telemux {} starting (config: {}, {} device(s))",
        env!("CARGO_PKG_VERSION"),
        cli.config.display(),
        handle.read().devices.len()
    );

    // Acquisition -> consumer channel. The consumer feeds the metric store
    // (raw + pipeline metric); protocol layers (phase 5) read the store.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<RawSample>>(64);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let manager = AcquisitionManager::new(&handle);
    let tasks = manager.spawn(handle.clone(), tx.clone(), shutdown_rx.clone());
    drop(tx); // device tasks own their senders; consumer owns the receiver

    // Metric store (latest raw + metric per sensor) and per-sensor pipelines.
    let store = Arc::new(MetricStore::new());
    let mut pipelines = PipelinesCache::new(&handle.read());
    info!(
        "{} pipeline(s) configured (config hot-update: {})",
        pipelines.len(),
        if handle.is_mutable() { "on" } else { "off" }
    );

    // Dev dashboard (compile-time gated; absent from release builds).
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    let _dashboard = telemux::dashboard::server::spawn(
        handle.clone(),
        store.clone(),
        cli.dashboard_port,
        shutdown_rx.clone(),
    );

    // Main loop: consume acquisition batches (store raw, run pipeline, store
    // metric) until Ctrl+C or the channel closes. Pipelines are not Send
    // (meval stages hold Rc), so this runs inline in the main task instead of
    // a spawned task.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            maybe_batch = rx.recv() => {
                match maybe_batch {
                    Some(batch) => {
                        pipelines.refresh(&handle);
                        store.update_batch_raw(&batch);
                        for s in &batch {
                            info!(
                                target: "sample",
                                "{} = {} {}",
                                s.sensor_id,
                                s.raw_value,
                                s.unit.as_deref().unwrap_or("")
                            );
                        }
                        for s in batch {
                            if let Some(pipeline) = pipelines.get_mut(&s.sensor_id) {
                                match pipeline.process(s.clone()) {
                                    Ok(metric) => {
                                        info!(
                                            target: "metric",
                                            "{} = {} {} [{}]",
                                            metric.sensor_id,
                                            metric.value,
                                            metric.unit.as_deref().unwrap_or(""),
                                            status_str(metric.status)
                                        );
                                        store.update_metric(s, metric);
                                    }
                                    Err(e) => {
                                        warn!("pipeline failed for {}: {e}", s.sensor_id);
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        info!("sample consumer: channel closed, all devices stopped");
                        break;
                    }
                }
            }
            _ = &mut ctrl_c => {
                info!("shutdown signal received, stopping...");
                break;
            }
        }
    }

    let _ = shutdown_tx.send(true);
    for t in tasks {
        let _ = t.await;
    }
    info!("telemux stopped");
    Ok(())
}

fn status_str(status: telemux::domain::MetricStatus) -> &'static str {
    match status {
        telemux::domain::MetricStatus::Normal => "normal",
        telemux::domain::MetricStatus::Warning => "warning",
        telemux::domain::MetricStatus::Critical => "critical",
        telemux::domain::MetricStatus::Unknown => "unknown",
    }
}
