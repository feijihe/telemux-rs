//! 集成测试：在内存双工流上的 Modbus-RTU 帧，
//! 使用挂到 `tokio::io::duplex` 上的 tokio-modbus RTU 客户端。
//!
//! 真实串口传输（tokio-serial）无法在没有硬件的情况下自动化；
//! 本测试在双工流另一端用手工构造的 RTU 服务器，
//! 覆盖完整的 RTU 帧路径（地址、功能码、CRC16、异常处理）。

use crc::{Crc, CRC_16_MODBUS};
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

use telemux::acquisition::rtu::ModbusRtuSource;
use telemux::acquisition::SensorSource;
use telemux::config::{
    DeviceConfig, RegisterConfig, RegisterFunction, Transport, ValueType, WordOrder,
};

fn device() -> DeviceConfig {
    DeviceConfig {
        name: "rtu-test".into(),
        transport: Transport::Rtu,
        unit_id: 1,
        host: String::new(),
        port: 0,
        poll_interval_ms: 100,
        timeout_ms: 1000,
        reconnect_initial_ms: 100,
        reconnect_max_ms: 1000,
        serial_port: Some("COM-test".into()),
        baud_rate: Some(9600),
        registers: vec![RegisterConfig {
            name: "r0".into(),
            sensor_id: "rtu.r0".into(),
            function: RegisterFunction::Holding,
            address: 0,
            count: Some(1),
            value_type: ValueType::U16,
            word_order: WordOrder::Big,
            unit: None,
            access: telemux::config::Access::Read,
        }],
    }
}

#[tokio::test]
async fn rtu_source_reads_registers() {
    let crc = Crc::<u16>::new(&CRC_16_MODBUS);
    let (client_side, mut server_side) = duplex(256);
    let device = device();
    let mut source = ModbusRtuSource::from_stream(client_side, &device);

    let server = tokio::spawn(async move {
        // RTU 请求帧：unit(1) fc(1) addr(2) cnt(2) crc(2)
        let mut req = [0u8; 8];
        server_side.read_exact(&mut req).await.unwrap();
        assert_eq!(req[0], 1, "单元 id");
        assert_eq!(req[1], 0x03, "读保持寄存器");
        assert_eq!(&req[2..6], &[0x00, 0x00, 0x00, 0x01], "addr=0 count=1");
        assert_eq!(
            crc.checksum(&req[..6]),
            u16::from_le_bytes([req[6], req[7]]),
            "请求 CRC"
        );

        // 以寄存器值 0x1234 响应。
        let mut resp = vec![0x01, 0x03, 0x02, 0x12, 0x34];
        let checksum = crc.checksum(&resp);
        resp.extend_from_slice(&checksum.to_le_bytes());
        server_side.write_all(&resp).await.unwrap();
        server_side.flush().await.unwrap();
    });

    let samples = source.read_samples(&device.registers).await.unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].raw_value, 0x1234 as f64);
    server.await.unwrap();
}

#[tokio::test]
async fn rtu_source_surfaces_exception() {
    let crc = Crc::<u16>::new(&CRC_16_MODBUS);
    let (client_side, mut server_side) = duplex(256);
    let device = device();
    let mut source = ModbusRtuSource::from_stream(client_side, &device);

    let server = tokio::spawn(async move {
        let mut req = [0u8; 8];
        server_side.read_exact(&mut req).await.unwrap();
        // 非法数据地址（0x02）异常帧。
        let mut resp = vec![0x01, 0x83, 0x02];
        let checksum = crc.checksum(&resp);
        resp.extend_from_slice(&checksum.to_le_bytes());
        server_side.write_all(&resp).await.unwrap();
        server_side.flush().await.unwrap();
    });

    let err = source.read_samples(&device.registers).await.unwrap_err();
    match err {
        telemux::acquisition::AcquisitionError::Exception(code) => {
            assert_eq!(u8::from(code), 0x02, "非法数据地址");
        }
        other => panic!("期望 modbus 异常，实际 {other:?}"),
    }
    server.await.unwrap();
}
