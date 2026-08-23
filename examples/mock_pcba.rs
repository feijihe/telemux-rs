//! 用于本地开发的模拟 PCBA Modbus-TCP 从站。
//!
//! 用法：`cargo run --example mock_pcba [port]`（默认端口 1502）。
//! 提供 `config/example.toml` 期望的寄存器映射，每次读取值都变化，
//! 以便观察网关实时数据日志。
//!
//! mock 仅存在于开发构建中（见 `src/mock.rs`）；在 release 构建中
//! 本示例编译为说明限制的桩程序。

#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
use telemux::mock::MockPcba;

#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1502".to_string());
    let bind = format!("127.0.0.1:{port}");
    let pcba = MockPcba::dynamic();
    let handle = pcba.spawn(&bind).await?;
    println!("mock PCBA 监听 {bind}（动态寄存器值）");
    println!("  holding[0]      = cpu 温度原始值 (u16)");
    println!("  holding[1]      = 风扇转速原始值  (u16)");
    println!("  holding[2..4]   = 运行时长 ticks  (u32, big)");
    println!("  holding[4..6]   = vcore          (f32, big)");
    println!("  input[0]        = vin 原始值     (u16)");
    println!("  input[1]        = iin 原始值     (u16)");
    println!("按 Ctrl+C 停止");
    tokio::signal::ctrl_c().await?;
    drop(handle);
    println!("mock PCBA stopped");
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "dev-dashboard")))]
fn main() {
    eprintln!(
        "mock_pcba 示例仅适用于开发构建 \
         (debug_assertions 或 --features dev-dashboard)"
    );
    std::process::exit(1);
}
