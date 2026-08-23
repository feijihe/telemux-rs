//! 全链路一致性测试（阶段 7）：进程内完成
//! mock PCBA → 采集解码 → 管道换算 → 指标存储 → 协议编码，
//! 断言各层数值一致（与阶段 5 验收的端到端链路等价，但不依赖外部进程）。

#![cfg(any(debug_assertions, feature = "dev-dashboard"))]

use std::sync::Arc;

use telemux::acquisition::create_source;
use telemux::config::{
    Access, Config, DeviceConfig, PipelineConfig, RegisterConfig, RegisterFunction, StageConfig,
    Transport, ValueType, WordOrder,
};
use telemux::domain::RawSample;
use telemux::mock::MockPcba;
use telemux::pipeline::PipelinesCache;
use telemux::protocol::{build_views, modbus_server};
use telemux::store::MetricStore;

fn reg(
    name: &str,
    sensor_id: &str,
    function: RegisterFunction,
    address: u16,
    value_type: ValueType,
    access: Access,
) -> RegisterConfig {
    RegisterConfig {
        name: name.into(),
        sensor_id: sensor_id.into(),
        function,
        address,
        count: Some(value_type.register_count()),
        value_type,
        word_order: WordOrder::Big,
        unit: None,
        access,
    }
}

fn scale_stage(scale: f64, unit: &str) -> StageConfig {
    StageConfig::Scale {
        scale,
        offset: 0.0,
        unit: Some(unit.into()),
    }
}

fn sample_config() -> Config {
    Config {
        general: Default::default(),
        devices: vec![DeviceConfig {
            name: "pcba-01".into(),
            transport: Transport::Tcp,
            unit_id: 1,
            host: "127.0.0.1".into(),
            port: 0, // 测试中填充
            poll_interval_ms: 100,
            timeout_ms: 1000,
            reconnect_initial_ms: 100,
            reconnect_max_ms: 1000,
            serial_port: None,
            baud_rate: None,
            registers: vec![
                reg(
                    "temp_raw",
                    "pcba-01.temp",
                    RegisterFunction::Holding,
                    0,
                    ValueType::U16,
                    Access::Read,
                ),
                reg(
                    "vcore",
                    "pcba-01.vcore",
                    RegisterFunction::Holding,
                    4,
                    ValueType::F32,
                    Access::Read,
                ),
                reg(
                    "vin",
                    "pcba-01.vin",
                    RegisterFunction::Input,
                    0,
                    ValueType::U16,
                    Access::Read,
                ),
                reg(
                    "leak",
                    "pcba-01.leak",
                    RegisterFunction::DiscreteInput,
                    0,
                    ValueType::Bool,
                    Access::Read,
                ),
            ],
        }],
        pipelines: vec![
            PipelineConfig {
                sensor_id: "pcba-01.temp".into(),
                stages: vec![scale_stage(0.1, "°C")], // 0x1234=4660 -> 466.0 °C
            },
            PipelineConfig {
                sensor_id: "pcba-01.vin".into(),
                stages: vec![scale_stage(0.001, "V")], // 200 -> 0.2 V
            },
        ],
        computed: vec![telemux::config::ComputedConfig {
            sensor_id: "pcba-01.ratio".into(),
            name: "ratio".into(),
            unit: Some("ratio".into()),
            inputs: [("v".to_string(), "pcba-01.vin".to_string())].into(),
            expression: "v * 10".into(),
        }],
        endpoints: Default::default(),
    }
}

#[tokio::test]
async fn full_chain_values_consistent() {
    let pcba = MockPcba::fixed();
    let handle = pcba.spawn("127.0.0.1:0").await.unwrap();

    let mut cfg = sample_config();
    cfg.devices[0].port = handle.addr.port();
    cfg.validate().unwrap();

    // 1. 采集：mock 固定值 -> 解码原始样本
    let mut source = create_source(&cfg.devices[0]).await.unwrap();
    let samples = source.read_samples(&cfg.devices[0].registers).await.unwrap();
    assert_eq!(samples.len(), 4);

    let by_id: std::collections::HashMap<&str, &RawSample> =
        samples.iter().map(|s| (s.sensor_id.as_str(), s)).collect();
    // mock fixed: holding[0]=0x1234, holding[4..6]=12.5f32, input[0]=200, 离散输入 addr0=true
    assert_eq!(by_id["pcba-01.temp"].raw_value, 0x1234 as f64);
    assert_eq!(by_id["pcba-01.vcore"].raw_value, 12.5);
    assert_eq!(by_id["pcba-01.vin"].raw_value, 200.0);
    assert_eq!(by_id["pcba-01.leak"].raw_value, 1.0);

    // 2. 管道：换算 -> 指标
    let store = Arc::new(MetricStore::new());
    let mut pipelines = PipelinesCache::new(&cfg);
    for s in samples {
        store.update_raw(s.clone());
        if let Some(pipeline) = pipelines.get_mut(&s.sensor_id) {
            let metric = pipeline.process(s.clone()).unwrap();
            store.update_metric(Some(s), metric);
        }
    }
    // computed 虚拟传感器：输入取 store 最新值（与主循环行为一致）。
    let engine = telemux::computed::ComputedEngine::new(&cfg);
    engine.run(&store);

    let temp = store.get(&telemux::domain::SensorId("pcba-01.temp".into())).unwrap();
    assert_eq!(temp.metric.as_ref().unwrap().value, 466.0);
    assert_eq!(temp.metric.as_ref().unwrap().unit.as_deref(), Some("°C"));
    let vin = store.get(&telemux::domain::SensorId("pcba-01.vin".into())).unwrap();
    assert_eq!(vin.metric.as_ref().unwrap().value, 0.2);
    assert_eq!(vin.metric.as_ref().unwrap().unit.as_deref(), Some("V"));

    // 3. 协议视图：computed 与真实传感器同等出现
    let views = build_views(&cfg, &store);
    assert_eq!(views.len(), 5); // 4 真实 + 1 computed
    let ratio = views.iter().find(|v| v.sensor_id == "pcba-01.ratio").unwrap();
    assert!(ratio.is_computed);
    assert_eq!(ratio.value, Some(2.0)); // 0.2 * 10

    // 4. Modbus 编码一致性：metric 值经 encode_value 后与解码器互逆
    let table = modbus_server::build_table(&cfg);
    // temp 在 holding 区（0x1234 原始值，无 raw -> metric 466.0 -> u16 round）
    let words = modbus_server::encode_value(
        temp.metric.as_ref().unwrap().value,
        ValueType::U16,
        WordOrder::Big,
    );
    assert_eq!(words, vec![466u16]);
    // vcore f32 在 input 区（12.5 -> [0x4148, 0x0000]）
    let vcore = store
        .get(&telemux::domain::SensorId("pcba-01.vcore".into()))
        .unwrap();
    let vcore_words = modbus_server::encode_value(
        vcore.raw.as_ref().unwrap().raw_value,
        ValueType::F32,
        WordOrder::Big,
    );
    assert_eq!(vcore_words, vec![0x4148, 0x0000]);
    // computed ratio f32 追加在 input 区末尾（vin 1 字 + ratio 2 字 = 3 字）
    assert_eq!(table.inputs.len(), 3);
    assert_eq!(table.inputs[1].as_ref().unwrap().sensor_id, "pcba-01.ratio");
}
