//! telemux-sim：CDU 仿真器（Modbus-TCP 从站 + 网页 UI）。
//!
//! 用法：
//!   telemux-sim [--config config/cdu.toml] [--modbus-port 1502] [--web-port 8082]
//!
//! 网关侧把设备配置为 `transport = "tcp"` + `host/port` 指向本从站，
//! 与连接真实 CDU 完全同构（见 `docs/SIMULATION.md`）。

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::sync::watch;
use tracing::info;

use telemux_sim::model::SimConfig;
use telemux_sim::server::{run as run_modbus, SimSlaveState};

#[derive(Parser)]
#[command(
    name = "telemux-sim",
    version,
    about = "CDU 仿真器：物理模型 + Modbus-TCP 从站 + 网页 UI"
)]
struct Cli {
    /// 仿真配置文件（TOML，`[sim]` 段）
    #[arg(short, long, default_value = "config/cdu.toml")]
    config: PathBuf,
    /// Modbus-TCP 从站端口
    #[arg(long, default_value_t = 1502)]
    modbus_port: u16,
    /// 网页 UI 端口
    #[arg(long, default_value_t = 8082)]
    web_port: u16,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    ).init();

    let cli = Cli::parse();
    let config = SimConfig::load(&cli.config)?;
    info!(
        "telemux-sim starting: {} control(s), {} sensor(s)",
        config.controls.len(),
        config.sensors.len()
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let slave = Arc::new(SimSlaveState::new(telemux_sim::model::SimEngine::new(config)));

    // Modbus-TCP 从站。
    let modbus_slave = slave.clone();
    let modbus_shutdown = shutdown_rx.clone();
    let modbus_task = tokio::spawn(async move {
        if let Err(e) = run_modbus(modbus_slave, cli.modbus_port, modbus_shutdown).await {
            tracing::warn!("modbus server stopped: {e}");
        }
    });

    // 网页 UI。
    let web_slave = slave.clone();
    let mut web_shutdown = shutdown_rx.clone();
    let web_task = tokio::spawn(async move {
        let router = telemux_sim::web::router(web_slave);
        let listener = match tokio::net::TcpListener::bind(format!("127.0.0.1:{}", cli.web_port)).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("web ui bind failed: {e}");
                return;
            }
        };
        info!("telemux-sim web UI on http://127.0.0.1:{}", cli.web_port);
        tokio::select! {
            res = axum::serve(listener, router) => {
                if let Err(e) = res { tracing::warn!("web ui error: {e}"); }
            }
            _ = web_shutdown.changed() => {}
        }
    });

    let _ = tokio::signal::ctrl_c().await;
    info!("shutting down");
    let _ = shutdown_tx.send(true);
    let _ = modbus_task.await;
    let _ = web_task.await;
    Ok(())
}
