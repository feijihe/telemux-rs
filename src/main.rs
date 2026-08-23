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
    about = "PCBA 遥测多协议网关（Modbus 采集）"
)]
struct Cli {
    /// TOML 配置文件路径
    #[arg(short, long, default_value = "config/example.toml")]
    config: PathBuf,
    /// 覆盖日志级别：trace|debug|info|warn|error
    #[arg(long)]
    log_level: Option<String>,
    /// 开发仪表盘端口（仅开发构建，默认 8080）
    #[arg(long)]
    dashboard_port: Option<u16>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let config = Config::load(&cli.config)?;
    // 运行时句柄：开发构建可变（热更新），release 只读。
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

    // 采集 -> 消费者通道。消费者写入指标存储
    // （raw + 管道指标）；协议层（阶段 5）读取存储。
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<RawSample>>(64);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let manager = AcquisitionManager::new(&handle);
    let spawned = manager.spawn(handle.clone(), tx.clone(), shutdown_rx.clone());
    let tasks = spawned.tasks;
    let broker = std::sync::Arc::new(spawned.broker);
    drop(tx); // 设备任务持有各自的 sender；消费者持有 receiver

    // 指标存储（每传感器最新 raw + 指标）、每传感器管道和
    // computed（虚拟）传感器。
    let store = Arc::new(MetricStore::new());
    let mut pipelines = PipelinesCache::new(&handle.read());
    let mut computed = telemux::computed::ComputedEngine::new(&handle.read());
    info!(
        "{} pipeline(s), {} computed sensor(s) (config hot-update: {})",
        pipelines.len(),
        computed.len(),
        if handle.is_mutable() { "on" } else { "off" }
    );

    // 开发仪表盘（编译期门控；release 构建中不存在）。
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    let _dashboard = telemux::dashboard::server::spawn(
        handle.clone(),
        store.clone(),
        cli.dashboard_port,
        shutdown_rx.clone(),
    );

    // 协议端点（阶段 5）：Redfish + Modbus 服务器，由配置驱动。
    let endpoints = handle.read().endpoints;
    let mut protocol_tasks = Vec::new();
    if endpoints.redfish_enabled {
        let router = telemux::protocol::redfish::router(handle.clone(), store.clone(), broker.clone());
        let port = endpoints.redfish_port;
        let mut shutdown = shutdown_rx.clone();
        protocol_tasks.push(tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
                .await
                .expect("bind redfish port");
            info!("redfish listening on 0.0.0.0:{port}");
            tokio::select! {
                res = axum::serve(listener, router) => {
                    if let Err(e) = res { warn!("redfish server error: {e}"); }
                }
                _ = shutdown.changed() => info!("redfish: shutting down"),
            }
        }));
    }
    if endpoints.modbus_enabled {
        let h = handle.clone();
        let s = store.clone();
        let b = broker.clone();
        let port = endpoints.modbus_port;
        let unit_id = endpoints.modbus_unit_id;
        let shutdown = shutdown_rx.clone();
        protocol_tasks.push(tokio::spawn(async move {
            if let Err(e) = telemux::protocol::modbus_server::run(
                h,
                s,
                b,
                port,
                unit_id,
                shutdown,
            )
            .await
            {
                warn!("modbus server stopped: {e}");
            }
        }));
    }

    // 主循环：消费采集批次（存储 raw、运行管道、存储指标），直到
    // Ctrl+C 或通道关闭。管道不是 Send（meval 阶段持有 Rc），
    // 因此在此主任务内联运行，而非派生任务。
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            maybe_batch = rx.recv() => {
                match maybe_batch {
                    Some(batch) => {
                        pipelines.refresh(&handle);
                        computed.refresh(&handle);
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
                                        store.update_metric(Some(s), metric);
                                    }
                                    Err(e) => {
                                        warn!("pipeline failed for {}: {e}", s.sensor_id);
                                    }
                                }
                            }
                        }
                        // 本批次后重新求值 computed（虚拟）传感器。
                        computed.run(&store);
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
    for t in protocol_tasks {
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
