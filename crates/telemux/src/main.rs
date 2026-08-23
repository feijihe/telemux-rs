use clap::Parser;
#[cfg(unix)]
use tracing::warn;

use telemux::cli::Cli;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Windows 服务模式：安装/卸载/运行 均不进入普通前台流程。
    #[cfg(windows)]
    {
        if cli.install_service {
            return telemux::service::install(&cli.config);
        }
        if cli.uninstall_service {
            return telemux::service::uninstall();
        }
        if cli.service {
            return telemux::service::run_as_service();
        }
    }

    // 普通前台模式：派生任务转发停机信号（Ctrl+C / SIGTERM）到主流程。
    let (signal_tx, signal_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(forward_signals(signal_tx));

    telemux::app::run_gateway(cli, signal_rx).await
}

/// 监听 Ctrl+C（所有平台）与 SIGTERM（Unix），触发停机信号。
async fn forward_signals(tx: tokio::sync::watch::Sender<bool>) {
    #[cfg(unix)]
    {
        let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to install SIGTERM handler: {e}");
                let _ = tokio::signal::ctrl_c().await;
                let _ = tx.send(true);
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    let _ = tx.send(true);
}
