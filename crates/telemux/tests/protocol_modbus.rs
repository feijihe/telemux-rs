//! 集成测试：用 tokio-modbus 客户端查询正在运行的网关 Modbus 服务器（1503）
//! 和模拟 PCBA（1502）。需要网关以 `config/test-p5.toml` 运行（否则跳过）。

#![cfg(any(debug_assertions, feature = "dev-dashboard"))]

use std::time::Duration;

use tokio_modbus::client::{Reader, Writer};
use tokio_modbus::slave::Slave;

async fn connect(port: u16) -> Option<tokio_modbus::client::Context> {
    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    match tokio::time::timeout(
        Duration::from_millis(1500),
        tokio_modbus::client::tcp::connect_slave(addr, Slave(1)),
    )
    .await
    {
        Ok(Ok(ctx)) => Some(ctx),
        _ => None,
    }
}

#[tokio::test]
async fn gateway_modbus_read_and_write() {
    let mut ctx = match connect(1503).await {
        Some(c) => c,
        None => {
            eprintln!("网关 modbus 服务器不可达；跳过");
            return;
        }
    };

    // 读输入寄存器：地址 0 处的 vin (u16)。
    let words = tokio::time::timeout(Duration::from_secs(3), ctx.read_input_registers(0, 1))
        .await
        .expect("读输入寄存器超时")
        .expect("传输正常")
        .expect("无异常");
    assert_eq!(words.len(), 1);
    eprintln!("vin raw = {} (mV)", words[0]);

    // 读离散输入：地址 0 处的 leak (bool)。
    let coils = tokio::time::timeout(Duration::from_secs(3), ctx.read_discrete_inputs(0, 1))
        .await
        .expect("读离散输入超时")
        .expect("传输正常")
        .expect("无异常");
    eprintln!("leak = {}", coils[0]);
    assert!(coils[0], "mock 离散输入地址 0 为 true");

    // 写保持寄存器：地址 0 处的 fan1_duty（read_write）。
    tokio::time::timeout(Duration::from_secs(3), ctx.write_single_register(0, 60))
        .await
        .expect("写保持寄存器超时")
        .expect("传输正常")
        .expect("无异常");
    eprintln!("已写 fan1_duty = 60");

    // 写只读寄存器（地址 1 的 fan1_speed）必须抛出异常。
    let err = tokio::time::timeout(Duration::from_secs(3), ctx.write_single_register(1, 5))
        .await
        .expect("写只读寄存器超时")
        .expect("传输正常");
    assert!(
        err.is_err(),
        "写只读寄存器应抛出异常"
    );
    eprintln!("只读写被正确拒绝：{err:?}");
}

#[tokio::test]
async fn mock_pcba_serves_modbus() {
    let mut ctx = match connect(1502).await {
        Some(c) => c,
        None => {
            eprintln!("mock pcba 不可达；跳过");
            return;
        }
    };
    let words = tokio::time::timeout(Duration::from_secs(3), ctx.read_holding_registers(0, 1))
        .await
        .expect("timeout")
        .expect("transport ok")
        .expect("no exception");
    eprintln!("mock holding[0] = {}", words[0]);
}
