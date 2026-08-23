//! 集成测试：telemux-sim 模拟器 Modbus-TCP 从站。
//!
//! 需要模拟器以 `crates/telemux-sim/config/cdu.toml` 运行（端口 1502），
//! 否则跳过：
//!   1. 读保持寄存器（控制变量 duty）；
//!   2. 读输入寄存器（传感器 f32）；
//!   3. 写 pump1_duty → 传感器联动变化（因果链）。

#![cfg(any(debug_assertions, feature = "dev-dashboard"))]

use std::time::Duration;

use tokio::sync::Mutex;
use tokio_modbus::client::{Reader, Writer};
use tokio_modbus::slave::Slave;

/// 写测试串行化：模拟器共享状态，并行写会互相干扰。
static WRITE_LOCK: Mutex<()> = Mutex::const_new(());

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

/// 读模拟器 f2（输入区 addr 20，f32 双字）。
async fn read_f2(ctx: &mut tokio_modbus::client::Context) -> f32 {
    let words = tokio::time::timeout(Duration::from_secs(3), ctx.read_input_registers(20, 2))
        .await
        .expect("read f2 timeout")
        .expect("transport ok")
        .expect("no exception");
    f32::from_bits(((words[0] as u32) << 16) | words[1] as u32)
}

#[tokio::test]
async fn sim_reads_controls_and_sensors() {
    let mut ctx = match connect(1502).await {
        Some(c) => c,
        None => {
            eprintln!("telemux-sim not reachable on 1502; skipping");
            return;
        }
    };
    // 保持区：pump1_duty@0。并行测试可能改过值，只断言范围。
    let duty = tokio::time::timeout(Duration::from_secs(3), ctx.read_holding_registers(0, 1))
        .await
        .expect("read duty timeout")
        .expect("transport ok")
        .expect("no exception");
    assert!(duty[0] <= 100, "pump1_duty in range, got {}", duty[0]);
    eprintln!("pump1_duty = {}", duty[0]);

    // 输入区：cdu.pri.p1@0 (f32 双字) = 300 + duty*1.2。
    let words = tokio::time::timeout(Duration::from_secs(3), ctx.read_input_registers(0, 2))
        .await
        .expect("read p1 timeout")
        .expect("transport ok")
        .expect("no exception");
    let p1 = f32::from_bits(((words[0] as u32) << 16) | words[1] as u32);
    eprintln!("cdu.pri.p1 = {p1} kPa");
    assert!((300.0..=420.0).contains(&p1), "p1 in [300, 420], got {p1}");
}

#[tokio::test]
async fn sim_write_duty_drives_flow() {
    let _lock = WRITE_LOCK.lock().await;
    let mut ctx = match connect(1502).await {
        Some(c) => c,
        None => {
            eprintln!("telemux-sim not reachable on 1502; skipping");
            return;
        }
    };
    // 重置 pump1_duty = 50，消除并行测试串扰。
    tokio::time::timeout(Duration::from_secs(3), ctx.write_single_register(0, 50))
        .await
        .expect("reset duty timeout")
        .expect("transport ok")
        .expect("no exception");
    tokio::time::sleep(Duration::from_millis(200)).await;

    let f2_before = read_f2(&mut ctx).await;
    eprintln!("f2 before = {f2_before}");
    assert!((f2_before - 109.0).abs() < 1.0, "f2 ≈ 109 at duty 50+40, got {f2_before}");

    // 写 pump1_duty = 80 → f2 = 10 + (80+40)*1.1 = 142。
    tokio::time::timeout(Duration::from_secs(3), ctx.write_single_register(0, 80))
        .await
        .expect("write duty timeout")
        .expect("transport ok")
        .expect("no exception");
    tokio::time::sleep(Duration::from_millis(200)).await;

    let f2_after = read_f2(&mut ctx).await;
    eprintln!("f2 after = {f2_after}");
    assert!(
        (f2_after - 142.0).abs() < 0.5,
        "f2 ≈ 142 after duty=80, got {f2_after}"
    );

    // 恢复 duty = 50。
    tokio::time::timeout(Duration::from_secs(3), ctx.write_single_register(0, 50))
        .await
        .expect("restore timeout")
        .expect("transport ok")
        .expect("no exception");
}

/// 完整闭环：网关（1503）写 pump1_duty → WriteBroker → 模拟器（1502）
/// 模型联动 → f2 变化。需要网关以 cdu-gateway.toml 运行（连模拟器）。
#[tokio::test]
async fn gateway_write_reaches_simulator() {
    let _lock = WRITE_LOCK.lock().await;
    let mut sim_ctx = match connect(1502).await {
        Some(c) => c,
        None => {
            eprintln!("telemux-sim not reachable on 1502; skipping");
            return;
        }
    };
    let mut gw_ctx = match connect(1503).await {
        Some(c) => c,
        None => {
            eprintln!("gateway not reachable on 1503; skipping");
            return;
        }
    };

    let f2_before = read_f2(&mut sim_ctx).await;
    eprintln!("f2 before (via simulator) = {f2_before}");

    // 通过网关写保持寄存器 0（pump1_duty）= 90。
    tokio::time::timeout(Duration::from_secs(3), gw_ctx.write_single_register(0, 90))
        .await
        .expect("gateway write timeout")
        .expect("transport ok")
        .expect("no exception");
    tokio::time::sleep(Duration::from_millis(2500)).await; // 等网关轮询 + 模拟器联动

    let f2_after = read_f2(&mut sim_ctx).await;
    eprintln!("f2 after (via simulator) = {f2_after}");
    // duty 90+40=130 → f2 = 10 + 130*1.1 = 153。
    assert!(
        (f2_after - 153.0).abs() < 1.0,
        "f2 ≈ 153 after gateway write duty=90, got {f2_after}"
    );

    // 恢复。
    tokio::time::timeout(Duration::from_secs(3), gw_ctx.write_single_register(0, 50))
        .await
        .expect("restore timeout")
        .expect("transport ok")
        .expect("no exception");
}
