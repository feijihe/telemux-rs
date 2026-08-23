//! 健康/就绪 HTTP 端点（阶段 6.4）。
//!
//! - `GET /healthz` — 存活探测：进程活着即返回 200 `{"status":"ok"}`。
//! - `GET /readyz`  — 就绪探测：至少一台设备在最近 2 个轮询间隔内有数据时
//!   返回 200，并附设备/协议状态；否则 503。
//!
//! 独立于 Redfish/Modbus 端口，供容器/服务管理器的存活与就绪检查使用。

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::config_handle::ConfigHandle;
use crate::domain::SensorId;
use crate::store::MetricStore;

#[derive(Clone)]
struct HealthState {
    config: ConfigHandle,
    store: Arc<MetricStore>,
}

/// 构建健康端点路由。
pub fn router(config: ConfigHandle, store: Arc<MetricStore>) -> Router {
    let state = HealthState { config, store };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn readyz(State(state): State<HealthState>) -> Response {
    let config = state.config.read();
    let now_ms = now_ms();
    let mut devices = Vec::with_capacity(config.devices.len());
    let mut any_fresh = false;

    for device in &config.devices {
        let window = device.poll_interval_ms * 2;
        let mut with_data = 0usize;
        let mut fresh = false;
        for reg in &device.registers {
            let state = state.store.get(&SensorId(reg.sensor_id.clone()));
            let ts = state
                .as_ref()
                .and_then(|s| s.raw.as_ref())
                .map(crate::domain::RawSample::timestamp_ms)
                .or_else(|| {
                    state
                        .as_ref()
                        .and_then(|s| s.metric.as_ref())
                        .map(crate::domain::Metric::timestamp_ms)
                });
            if let Some(ts) = ts {
                with_data += 1;
                if now_ms.saturating_sub(ts) <= window {
                    fresh = true;
                    any_fresh = true;
                }
            }
        }
        devices.push(json!({
            "name": device.name,
            "connected": fresh,
            "sensors_total": device.registers.len(),
            "sensors_with_data": with_data,
        }));
    }

    let endpoints = &config.endpoints;
    let body = json!({
        "status": if any_fresh { "ready" } else { "not_ready" },
        "devices": devices,
        "endpoints": {
            "redfish": { "enabled": endpoints.redfish_enabled, "port": endpoints.redfish_port },
            "modbus": { "enabled": endpoints.modbus_enabled, "port": endpoints.modbus_port },
            "health": { "enabled": endpoints.health_enabled, "port": endpoints.health_port },
        },
        "computed_sensors": config.computed.len(),
    });

    if any_fresh {
        (StatusCode::OK, Json(body)).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
    }
}

fn now_ms() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as u64,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::config::{
        Config, DeviceConfig, RegisterConfig, RegisterFunction, Transport, ValueType, WordOrder,
    };
    use crate::domain::RawSample;

    fn sample_config() -> Config {
        Config {
            general: Default::default(),
            devices: vec![DeviceConfig {
                name: "pcba-01".into(),
                transport: Transport::Tcp,
                unit_id: 1,
                host: "127.0.0.1".into(),
                port: 1502,
                poll_interval_ms: 500,
                timeout_ms: 1000,
                reconnect_initial_ms: 100,
                reconnect_max_ms: 1000,
                serial_port: None,
                baud_rate: None,
                registers: vec![RegisterConfig {
                    name: "r1".into(),
                    sensor_id: "s.r1".into(),
                    function: RegisterFunction::Input,
                    address: 0,
                    count: Some(1),
                    value_type: ValueType::U16,
                    word_order: WordOrder::Big,
                    unit: None,
                    access: crate::config::Access::Read,
                }],
            }],
            pipelines: vec![],
            computed: vec![],
            endpoints: Default::default(),
        }
    }

    #[tokio::test]
    async fn healthz_always_ok() {
        let app = router(
            ConfigHandle::new(sample_config(), "unused.toml".into()),
            Arc::new(MetricStore::new()),
        );
        let res = app
            .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let text = res.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&text).unwrap();
        assert_eq!(v["status"], "ok");
    }

    #[tokio::test]
    async fn readyz_not_ready_without_data() {
        let app = router(
            ConfigHandle::new(sample_config(), "unused.toml".into()),
            Arc::new(MetricStore::new()),
        );
        let res = app
            .oneshot(Request::builder().uri("/readyz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn readyz_ready_with_fresh_data() {
        let store = Arc::new(MetricStore::new());
        store.update_raw(RawSample {
            sensor_id: SensorId("s.r1".into()),
            name: "r1".into(),
            raw_value: 1.0,
            unit: None,
            timestamp: std::time::SystemTime::now(),
        });
        let app = router(ConfigHandle::new(sample_config(), "unused.toml".into()), store);
        let res = app
            .oneshot(Request::builder().uri("/readyz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let text = res.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&text).unwrap();
        assert_eq!(v["status"], "ready");
        assert_eq!(v["devices"][0]["name"], "pcba-01");
    }
}
