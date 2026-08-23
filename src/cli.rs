//! CLI 参数定义（clap derive）。

use std::path::PathBuf;

use clap::Parser;

/// 网关进程的全部命令行参数。
#[derive(Debug, Clone, Parser)]
#[command(
    name = "telemux",
    version,
    about = "PCBA 遥测多协议网关（Modbus 采集 → Redfish/Modbus 出口）"
)]
pub struct Cli {
    /// TOML 配置文件路径
    #[arg(short, long, default_value = "config/example.toml")]
    pub config: PathBuf,
    /// 覆盖日志级别：trace|debug|info|warn|error
    #[arg(long)]
    pub log_level: Option<String>,
    /// 开发仪表盘端口（仅开发构建，默认 8080）
    #[arg(long)]
    pub dashboard_port: Option<u16>,

    /// 以 Windows 服务模式运行（阶段 6.3，仅 Windows）
    #[cfg(windows)]
    #[arg(long)]
    pub service: bool,
    /// 安装 Windows 服务（阶段 6.3，仅 Windows）
    #[cfg(windows)]
    #[arg(long)]
    pub install_service: bool,
    /// 卸载 Windows 服务（阶段 6.3，仅 Windows）
    #[cfg(windows)]
    #[arg(long)]
    pub uninstall_service: bool,
}
