//! 集成测试：针对进程内模拟 PCBA 的 Modbus-TCP 采集。
//!
//! 模拟 PCBA 仅存在于开发构建中（见 `src/mock.rs`），因此本文件在
//! release 测试运行中被跳过。

#![cfg(any(debug_assertions, feature = "dev-dashboard"))]

use std::collections::HashMap;
use std::time::Duration;

use telemux::acquisition::{create_source, AcquisitionManager};
use telemux::config::{
    Config, DeviceConfig, RegisterConfig, RegisterFunction, Transport, ValueType, WordOrder,
};
use telemux::domain::RawSample;
use telemux::mock::MockPcba;

fn reg(
    name: &str,
    sensor_id: &str,
    function: RegisterFunction,
    address: u16,
    value_type: ValueType,
    word_order: WordOrder,
) -> RegisterConfig {
    RegisterConfig {
        name: name.to_string(),
        sensor_id: sensor_id.to_string(),
        function,
        address,
        count: Some(value_type.register_count()),
        value_type,
        word_order,
        unit: None,
        access: telemux::config::Access::Read,
    }
}

fn device(registers: Vec<RegisterConfig>) -> DeviceConfig {
    DeviceConfig {
        name: "test-pcba".into(),
        transport: Transport::Tcp,
        unit_id: 1,
        host: "127.0.0.1".into(),
        port: 0, // 测试在绑定 mock 后填充
        poll_interval_ms: 100,
        timeout_ms: 1000,
        reconnect_initial_ms: 100,
        reconnect_max_ms: 1000,
        serial_port: None,
        baud_rate: None,
        registers,
    }
}

#[tokio::test]
async fn tcp_source_decodes_all_value_types() {
    let pcba = MockPcba::fixed();
    let handle = pcba.spawn("127.0.0.1:0").await.unwrap();

    let mut dev = device(vec![
        reg("r_u16", "s.u16", RegisterFunction::Holding, 0, ValueType::U16, WordOrder::Big),
        reg("r_i16", "s.i16", RegisterFunction::Holding, 1, ValueType::I16, WordOrder::Big),
        reg("r_u32", "s.u32", RegisterFunction::Holding, 2, ValueType::U32, WordOrder::Big),
        reg("r_f32", "s.f32", RegisterFunction::Holding, 4, ValueType::F32, WordOrder::Big),
        reg("r_u32l", "s.u32l", RegisterFunction::Holding, 2, ValueType::U32, WordOrder::Little),
        reg("r_in", "s.in", RegisterFunction::Input, 0, ValueType::U16, WordOrder::Big),
    ]);
    dev.port = handle.addr.port();

    let mut source = create_source(&dev, &telemux::config::SimConfig::default()).await.unwrap();
    let samples = source.read_samples(&dev.registers).await.unwrap();
    assert_eq!(samples.len(), 6);

    let by_name: HashMap<&str, f64> = samples
        .iter()
        .map(|s| (s.name.as_str(), s.raw_value))
        .collect();
    assert_eq!(by_name["r_u16"], 0x1234 as f64);
    assert_eq!(by_name["r_i16"], -1.0);
    assert_eq!(by_name["r_u32"], 0xDEAD_BEEFu32 as f64);
    // 对字 [0xDEAD, 0xBEEF] 的小端字序 -> 0xBEEFDEAD
    assert_eq!(by_name["r_u32l"], 0xBEEF_DEADu32 as f64);
    assert_eq!(by_name["r_f32"], 12.5);
    assert_eq!(by_name["r_in"], 200.0);
}

#[tokio::test]
async fn tcp_source_recovers_after_server_restart() {
    // 针对 mock 轮询，杀掉它，在新端口重启，期望重连后成功
    // （惰性重连在数据源内部）。
    let pcba = MockPcba::fixed();
    let handle = pcba.spawn("127.0.0.1:0").await.unwrap();

    let mut dev = device(vec![reg(
        "r0",
        "s.0",
        RegisterFunction::Holding,
        0,
        ValueType::U16,
        WordOrder::Big,
    )]);
    dev.port = handle.addr.port();

    let mut source = create_source(&dev, &telemux::config::SimConfig::default()).await.unwrap();
    let samples = source.read_samples(&dev.registers).await.unwrap();
    assert_eq!(samples[0].raw_value, 0x1234 as f64);

    // 模拟设备死亡：mock 句柄的任务在 drop 时被 abort。
    drop(handle);
    assert!(source.read_samples(&dev.registers).await.is_err());
    assert!(source.read_samples(&dev.registers).await.is_err()); // 仍然宕机
    let pcba2 = MockPcba::fixed();
    let handle2 = pcba2.spawn("127.0.0.1:0").await.unwrap();
    dev.port = handle2.addr.port();
    drop(source);

    // 重建数据源（如同调度器退避后的做法）并轮询。
    let mut source = create_source(&dev, &telemux::config::SimConfig::default()).await.unwrap();
    let samples = source.read_samples(&dev.registers).await.unwrap();
    assert_eq!(samples[0].raw_value, 0x1234 as f64);
}

#[tokio::test]
async fn manager_streams_samples_from_mock() {
    use tokio::sync::{mpsc, watch};

    let pcba = MockPcba::dynamic();
    let handle = pcba.spawn("127.0.0.1:0").await.unwrap();
    let mut dev = device(vec![reg(
        "r0",
        "s.0",
        RegisterFunction::Holding,
        0,
        ValueType::U16,
        WordOrder::Big,
    )]);
    dev.port = handle.addr.port();

    let cfg = Config {
        general: Default::default(),
        devices: vec![dev],
        pipelines: vec![],
        computed: vec![],
        endpoints: Default::default(),
        sim: Default::default(),
    };
    let handle = telemux::config_handle::ConfigHandle::new(cfg, "unused.toml".into());

    let (tx, mut rx) = mpsc::channel::<Vec<RawSample>>(16);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let manager = AcquisitionManager::new(&handle);
    let spawned = manager.spawn(handle.clone(), tx.clone(), shutdown_rx);
    drop(tx);

    // 第一个 tick 立即触发；动态值：holding[0] = 250 + (snapshot % 50)，
    // snapshot 从 1 开始 -> 251。
    let batch = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("等待第一批样本超时")
        .expect("任何批次到达前通道已关闭");
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].sensor_id.as_str(), "s.0");
    assert_eq!(batch[0].raw_value, 251.0);

    // 第二次轮询应最终到达且值不同。
    let batch = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("等待第二批样本超时")
        .expect("通道已关闭");
    assert!(batch[0].raw_value >= 250.0 && batch[0].raw_value < 300.0);

    shutdown_tx.send(true).unwrap();
    for t in spawned.tasks {
        t.await.unwrap();
    }
    assert!(rx.recv().await.is_none(), "关闭后通道应关闭");
}
