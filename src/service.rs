//! Windows 服务支持（阶段 6.3，仅 Windows 平台编译）。
//!
//! 把网关作为 Windows 服务运行（Service Control Manager 管理）：
//!
//! - `telemux --install-service [--config <path>]`：安装服务（需管理员）。
//!   服务二进制参数保存 `--service --config <path>`，由 SCM 启动时传入。
//! - `telemux --uninstall-service`：删除服务（需管理员）。
//! - `telemux --service`：由 SCM 调用；进入服务主循环，处理
//!   Stop/Shutdown 控制事件并转发为停机信号。
//!
//! 服务名：`telemux`。服务内日志走 `[general] log_dir` 滚动文件
//! （stdout 在服务会话中不可见）。

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::watch;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult,
};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, Error as WinError, Result as WinResult};

use crate::app::run_gateway;
use crate::cli::Cli;

/// Windows 服务名。
pub const SERVICE_NAME: &str = "telemux";

define_windows_service!(ffi_service_main, service_main);

/// SCM 入口（由系统调用，不直接调用）。
fn service_main(arguments: Vec<OsString>) {
    if let Err(e) = service_main_impl(arguments) {
        // 服务失败：写入事件日志不可行（无集成），打 stderr 供调试。
        eprintln!("telemux service failed: {e}");
        std::process::exit(1);
    }
}

fn service_main_impl(arguments: Vec<OsString>) -> WinResult<()> {
    // 从服务参数解析配置路径（安装时写入的 `--config <path>`）。
    let config_path = parse_config_arg(&arguments)
        .unwrap_or_else(|| PathBuf::from("config/example.toml"));

    // 停机信号通道：控制处理器（Stop/Shutdown）→ run_gateway。
    let (signal_tx, signal_rx) = watch::channel(false);

    let status_handle = service_control_handler::register(SERVICE_NAME, move |control| {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = signal_tx.send(true);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })?;

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    // 服务会话无控制台：stdout 不可见，日志走滚动文件。
    let cli = Cli {
        config: config_path,
        log_level: None,
        dashboard_port: None,
        #[cfg(windows)]
        service: false,
        #[cfg(windows)]
        install_service: false,
        #[cfg(windows)]
        uninstall_service: false,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(WinError::Winapi)?;
    let result = runtime.block_on(run_gateway(cli, signal_rx));

    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });

    result.map_err(|e| WinError::Winapi(std::io::Error::other(e.to_string())))
}

/// 由 `main` 调用：启动服务控制分发器（阻塞，直到服务停止）。
pub fn run_as_service() -> anyhow::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

/// 安装 Windows 服务（需管理员）。
pub fn install(config: &std::path::Path) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;

    let mut launch = vec![OsString::from("--service")];
    if let Some(s) = config.to_str() {
        launch.push(OsString::from("--config"));
        launch.push(OsString::from(s));
    }

    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("Telemux PCBA Telemetry Gateway"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: launch,
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };
    manager.create_service(&info, ServiceAccess::QUERY_STATUS)?;
    println!("service `{SERVICE_NAME}` installed (start via `sc start {SERVICE_NAME}`)");
    Ok(())
}

/// 卸载 Windows 服务（需管理员）。
pub fn uninstall() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(SERVICE_NAME, ServiceAccess::DELETE)?;
    service.delete()?;
    println!("service `{SERVICE_NAME}` uninstalled");
    Ok(())
}

/// 从服务参数中提取 `--config <path>`。
fn parse_config_arg(arguments: &[OsString]) -> Option<PathBuf> {
    let mut iter = arguments.iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            if let Some(path) = iter.next() {
                return Some(PathBuf::from(path));
            }
        } else if let Some(s) = arg.to_str()
            && let Some(rest) = s.strip_prefix("--config=")
        {
            return Some(PathBuf::from(rest));
        }
    }
    None
}
