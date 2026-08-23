//! 集成测试：CDU 仿真因果链（阶段 8 扩展）。
//!
//! 需要网关以 `config/cdu.toml` 运行（端口 1503/8081），否则跳过：
//! 1. 读取仿真传感器与 computed 派生量；
//! 2. 通过 Modbus 写 pump1_duty（保持寄存器地址 0）→ 流量 F2 与泵压差变化。

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
async fn cdu_sim_read_sensors_and_computed() {
    let mut ctx = match connect(1503).await {
        Some(c) => c,
        None => {
            eprintln!("gateway modbus server not reachable; skipping");
            return;
        }
    };
    // 输入区布局（append 顺序）：computed 4 个 f32 (8 字) 在前，
    // 然后 sim 传感器 29 个 f32。cdu.pri.p1 是第一个 sim 传感器 → addr 8-9。
    let words = tokio::time::timeout(Duration::from_secs(3), ctx.read_input_registers(8, 2))
        .await
        .expect("read timeout")
        .expect("transport ok")
        .expect("no exception");
    let p1 = f32::from_bits(((words[0] as u32) << 16) | words[1] as u32);
    eprintln!("cdu.pri.p1 = {p1} kPa");
    assert!((p1 - 336.0).abs() < 5.0, "p1 ≈ 300+30*1.2=336, got {p1}");
}

#[tokio::test]
async fn cdu_sim_write_duty_drives_flow_and_dp() {
    let mut ctx = match connect(1503).await {
        Some(c) => c,
        None => {
            eprintln!("gateway modbus server not reachable; skipping");
            return;
        }
    };

    // 保持区：可写控制变量。pump1_duty 在 addr 0（u16, 0-100）。
    // 读取 pump1_duty 与 pump2_duty 的初始值。
    let duty0 = tokio::time::timeout(Duration::from_secs(3), ctx.read_holding_registers(0, 1))
        .await
        .expect("read duty timeout")
        .expect("transport ok")
        .expect("no exception");
    eprintln!("pump1_duty initial = {}", duty0[0]);

    // 输入区布局：computed 8 字 + sim 58 字 = 66 字。扫描 8..66 找 f2。
    let mut found_f2 = false;
    for start in (8..66).step_by(2) {
        let words = tokio::time::timeout(Duration::from_secs(3), ctx.read_input_registers(start, 2))
            .await
            .expect("read timeout")
            .expect("transport ok")
            .expect("no exception");
        let v = f32::from_bits(((words[0] as u32) << 16) | words[1] as u32);
        if (v - 109.0).abs() < 2.0 {
            eprintln!("found f2_flow = {v} at input addr {start}");
            found_f2 = true;
            break;
        }
    }
    assert!(found_f2, "expected f2_flow ≈ 109 L/min (pump1=50, pump2=40)");

    // 写 pump1_duty = 80：f2 = 10 + (80+40)*1.1 = 142；pump1 扬程 = 80²*0.08 = 512。
    tokio::time::timeout(Duration::from_secs(3), ctx.write_single_register(0, 80))
        .await
        .expect("write duty timeout")
        .expect("transport ok")
        .expect("no exception");
    eprintln!("wrote pump1_duty = 80");
    tokio::time::sleep(Duration::from_millis(2500)).await; // 等下一轮采集

    // 重新扫描输入区，f2 应 ≈ 142。
    let mut found_new = false;
    for start in (8..66).step_by(2) {
        let words = tokio::time::timeout(Duration::from_secs(3), ctx.read_input_registers(start, 2))
            .await
            .expect("read timeout")
            .expect("transport ok")
            .expect("no exception");
        let v = f32::from_bits(((words[0] as u32) << 16) | words[1] as u32);
        if (v - 142.0).abs() < 2.0 {
            eprintln!("f2_flow after duty=80: {v} at input addr {start}");
            found_new = true;
            break;
        }
    }
    assert!(found_new, "expected f2_flow ≈ 142 after pump1_duty=80");

    // 恢复 duty = 50。
    tokio::time::timeout(Duration::from_secs(3), ctx.write_single_register(0, 50))
        .await
        .expect("restore duty timeout")
        .expect("transport ok")
        .expect("no exception");
}
