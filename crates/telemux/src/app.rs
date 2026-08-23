//! 网关核心运行逻辑（阶段 6 重构）。
//!
//! [`run_gateway`] 承担完整生命周期：加载配置 → 启动采集/管道/协议服务 →
//! 主循环消费样本 → 收到外部停机信号后优雅关闭。
//!
//! 停机信号由调用方注入（`signal_rx`）：
//! - 普通命令行模式：`main` 派生任务监听 Ctrl+C / SIGTERM；
//! - Windows 服务模式：服务控制处理器触发。

use std::sync::Arc;

use tokio::sync::watch;
use tracing::{info, warn};

use crate::cli::Cli;
use crate::config::Config;
use crate::config_handle::ConfigHandle;
use crate::domain::RawSample;
use crate::logging::{self, LogFileConfig};
use crate::pipeline::PipelinesCache;
use crate::store::MetricStore;

/// 运行网关主流程，直到 `signal_rx` 收到停机信号后优雅退出。
pub async fn run_gateway(cli: Cli, signal_rx: watch::Receiver<bool>) -> anyhow::Result<()> {
    // ---- 配置与日志 ----
    let config = Config::load(&cli.config)?;
    // 运行时句柄：开发构建可变（热更新），release 只读。
    let handle = ConfigHandle::new(config, cli.config.clone());

    let general = handle.read().general;
    let level = match cli
        .log_level
        .as_deref()
        .and_then(logging::level_from_str)
    {
        Some(l) => l,
        None => general.log_level.as_tracing_level(),
    };
    let file_cfg = general.log_dir.as_ref().map(|dir| LogFileConfig {
        dir: dir.clone().into(),
        prefix: "telemux".to_string(),
        max_files: general.log_max_files as usize,
    });
    let _log_guard = logging::init(level, file_cfg)
        .map_err(|e| anyhow::anyhow!("init logging: {e}"))?;

    info!(
        "telemux {} starting (config: {}, {} device(s))",
        env!("CARGO_PKG_VERSION"),
        cli.config.display(),
        handle.read().devices.len()
    );

    // ---- 采集与存储 ----
    // 采集 -> 消费者通道。消费者写入指标存储
    // （raw + 管道指标）；协议层（阶段 5）读取存储。
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<RawSample>>(64);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let manager = crate::acquisition::AcquisitionManager::new(&handle);
    let spawned = manager.spawn(handle.clone(), tx.clone(), shutdown_rx.clone());
    let tasks = spawned.tasks;
    let broker = std::sync::Arc::new(spawned.broker);
    drop(tx); // 设备任务持有各自的 sender；消费者持有 receiver

    // 指标存储（每传感器最新 raw + 指标）、每传感器管道和
    // computed（虚拟）传感器。
    let store = Arc::new(MetricStore::new());
    let mut pipelines = PipelinesCache::new(&handle.read());
    let mut computed = crate::computed::ComputedEngine::new(&handle.read());
    info!(
        "{} pipeline(s), {} computed sensor(s) (config hot-update: {})",
        pipelines.len(),
        computed.len(),
        if handle.is_mutable() { "on" } else { "off" }
    );

    // ---- 协议端点（阶段 5/6）：Redfish + Modbus + 健康检查 ----
    let endpoints = handle.read().endpoints;
    let mut protocol_tasks = Vec::new();

    if endpoints.redfish_enabled {
        let router = crate::protocol::redfish::router(handle.clone(), store.clone(), broker.clone());
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
            if let Err(e) =
                crate::protocol::modbus_server::run(h, s, b, port, unit_id, shutdown).await
            {
                warn!("modbus server stopped: {e}");
            }
        }));
    }

    // 健康/就绪端点（阶段 6.4）：独立小 HTTP 服务，供存活/就绪检查。
    if endpoints.health_enabled {
        let router = crate::health::router(handle.clone(), store.clone());
        let port = endpoints.health_port;
        let mut shutdown = shutdown_rx.clone();
        protocol_tasks.push(tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
                .await
                .expect("bind health port");
            info!("health endpoint listening on 0.0.0.0:{port}");
            tokio::select! {
                res = axum::serve(listener, router) => {
                    if let Err(e) = res { warn!("health server error: {e}"); }
                }
                _ = shutdown.changed() => info!("health endpoint: shutting down"),
            }
        }));
    }

    // 开发仪表盘（编译期门控；release 构建中不存在）。
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    let _dashboard = crate::dashboard::server::spawn(
        handle.clone(),
        store.clone(),
        cli.dashboard_port,
        shutdown_rx.clone(),
    );

    // ---- 主循环 ----
    // 消费采集批次（存储 raw、运行管道、存储指标），直到
    // 外部停机信号或通道关闭。管道不是 Send（meval 阶段持有 Rc），
    // 因此在此主任务内联运行，而非派生任务。
    let mut signal_rx = signal_rx;
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
            _ = signal_rx.changed() => {
                info!("shutdown signal received, stopping...");
                break;
            }
        }
    }

    // ---- 优雅停机 ----
    // 顺序：通知各协议/采集任务停止 → 等待任务退出 → 日志 guard 随
    // 函数返回而 drop（flush 滚动文件写入）。
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

fn status_str(status: crate::domain::MetricStatus) -> &'static str {
    match status {
        crate::domain::MetricStatus::Normal => "normal",
        crate::domain::MetricStatus::Warning => "warning",
        crate::domain::MetricStatus::Critical => "critical",
        crate::domain::MetricStatus::Unknown => "unknown",
    }
}
