//! 稳定性演练（阶段 7.3）：长时间运行 + 断线重连恢复。
//!
//! 完整走 采集 manager → 样本通道 → 存储 的路径：
//! 1. 连续轮询一段时间，每批样本都有数据；
//! 2. 杀掉 mock（模拟 PCBA 掉线）→ 轮询失败但进程不崩；
//! 3. 换端口重启 mock 并热更新配置 → 一个轮询周期内自动恢复。

#![cfg(any(debug_assertions, feature = "dev-dashboard"))]

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use telemux::config::{Config, DeviceConfig, RegisterConfig, RegisterFunction, Transport, ValueType, WordOrder};
use telemux::config_handle::ConfigHandle;
use telemux::domain::RawSample;
use telemux::mock::{MockHandle, MockPcba};
use telemux::store::MetricStore;

fn reg() -> RegisterConfig {
    RegisterConfig {
        name: "r0".into(),
        sensor_id: "s.0".into(),
        function: RegisterFunction::Holding,
        address: 0,
        count: Some(1),
        value_type: ValueType::U16,
        word_order: WordOrder::Big,
        unit: None,
        access: telemux::config::Access::Read,
    }
}

fn device_config(port: u16) -> DeviceConfig {
    DeviceConfig {
        name: "test-pcba".into(),
        transport: Transport::Tcp,
        unit_id: 1,
        host: "127.0.0.1".into(),
        port,
        poll_interval_ms: 100,
        timeout_ms: 1000,
        reconnect_initial_ms: 100,
        reconnect_max_ms: 500,
        serial_port: None,
        baud_rate: None,
        registers: vec![reg()],
    }
}

fn full_config(port: u16) -> Config {
    Config {
        general: Default::default(),
        devices: vec![device_config(port)],
        pipelines: vec![],
        computed: vec![],
        endpoints: Default::default(),
    }
}

#[tokio::test]
async fn survives_device_outage_and_recovers() {
    // 1. 启动 mock 与完整采集链路。
    let pcba = MockPcba::dynamic();
    let handle = pcba.spawn("127.0.0.1:0").await.unwrap();
    let cfg_handle = ConfigHandle::new(full_config(handle.addr.port()), "unused.toml".into());

    let store = Arc::new(MetricStore::new());
    let (tx, mut rx) = mpsc::channel::<Vec<RawSample>>(16);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let manager = telemux::acquisition::AcquisitionManager::new(&cfg_handle);
    let spawned = manager.spawn(cfg_handle.clone(), tx.clone(), shutdown_rx);
    drop(tx);

    // 消费者：把每批写入 store（与主循环一致）。
    let consumer = tokio::spawn({
        let store = store.clone();
        async move {
            while let Some(batch) = rx.recv().await {
                store.update_batch_raw(&batch);
            }
        }
    });

    // 2. 连续轮询 ~1s（10 批），store 内样本持续更新。
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let s0 = store.get(&telemux::domain::SensorId("s.0".into())).unwrap();
    let v_before = s0.raw.as_ref().unwrap().raw_value;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let s1 = store.get(&telemux::domain::SensorId("s.0".into())).unwrap();
    let v_after = s1.raw.as_ref().unwrap().raw_value;
    assert_ne!(v_before, v_after, "dynamic values must keep flowing");
    assert_eq!(store.len(), 1, "store bounded during steady state");

    // 3. 杀掉 mock：轮询进入失败-退避，进程不崩。
    drop(handle); // abort mock 服务任务
    tokio::time::sleep(Duration::from_millis(800)).await;

    // 4. 换端口重启 mock，热更新配置端口。
    let pcba2 = MockPcba::dynamic();
    let handle2: MockHandle = pcba2.spawn("127.0.0.1:0").await.unwrap();
    cfg_handle
        .update(|cfg| {
            cfg.devices[0].port = handle2.addr.port();
            Ok(())
        })
        .unwrap();

    // 5. 一个轮询周期（100ms）+ 退避后自动恢复（值继续变化）。
    let mut recovered = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let s = store.get(&telemux::domain::SensorId("s.0".into())).unwrap();
        if let Some(raw) = s.raw
            && raw.raw_value != v_after
        {
            recovered = true;
            break;
        }
    }
    assert!(recovered, "did not recover after mock restart");

    // 6. 恢复后存储容量保持有界（无泄漏）。
    assert_eq!(store.len(), 1, "store must stay bounded (1 sensor)");

    drop(handle2);
    shutdown_tx.send(true).unwrap();
    for t in spawned.tasks {
        let _ = t.await;
    }
    consumer.abort();
}
