//! Redfish 服务（阶段 5）：只读传感器资源 + 对可读写寄存器的 PATCH 写入。
//! 完全由配置驱动——每次请求都基于实时的 `ConfigHandle` 构建资源树，
//! 因此新增的寄存器/computed 传感器会自动出现。
//!
//! 端点（见 `docs/REDFISH.md`）：
//! - `GET  /redfish/v1`                          ServiceRoot
//! - `GET  /redfish/v1/Chassis`                  ChassisCollection
//! - `GET  /redfish/v1/Chassis/{device}`         Chassis
//! - `GET  /redfish/v1/Chassis/{device}/Thermal` Thermal (Temps/Fans)
//! - `GET  /redfish/v1/Chassis/{device}/Power`   Power (Voltages)
//! - `GET  /redfish/v1/Chassis/{device}/Sensors` SensorCollection
//! - `GET/PATCH /redfish/v1/Chassis/{device}/Sensors/{sensorId}`  Sensor (+写)

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::{Access, RegisterFunction, StageConfig};
use crate::config_handle::ConfigHandle;
use crate::protocol::{build_views, SensorView, WriteBroker, WriteValue};
use crate::store::MetricStore;

#[derive(Clone)]
pub struct RedfishState {
    config: ConfigHandle,
    store: Arc<MetricStore>,
    broker: Arc<WriteBroker>,
}

/// 构建 Redfish 路由。
pub fn router(
    config: ConfigHandle,
    store: Arc<MetricStore>,
    broker: Arc<WriteBroker>,
) -> Router {
    let state = RedfishState {
        config,
        store,
        broker,
    };
    Router::new()
        .route("/redfish/v1", get(service_root))
        .route("/redfish/v1/Chassis", get(chassis_collection))
        .route("/redfish/v1/Chassis/{device}", get(chassis))
        .route(
            "/redfish/v1/Chassis/{device}/Thermal",
            get(thermal),
        )
        .route("/redfish/v1/Chassis/{device}/Power", get(power))
        .route(
            "/redfish/v1/Chassis/{device}/Sensors",
            get(sensor_collection),
        )
        .route(
            "/redfish/v1/Chassis/{device}/Sensors/{sensor_id}",
            get(sensor).patch(update_sensor),
        )
        .with_state(state)
}

/// 单个机箱的视图：设备的真实寄存器 + 所有 computed 传感器。
fn chassis_views(state: &RedfishState, device: &str) -> Vec<SensorView> {
    build_views(&state.config.read(), &state.store)
        .into_iter()
        .filter(|v| v.device == device || v.is_computed)
        .collect()
}

fn sensor_json(view: &SensorView) -> Value {
    let mut oem = json!({
        "SensorId": view.sensor_id,
        "Computed": view.is_computed,
        "Formula": view.formula,
    });
    if !view.is_computed {
        oem["Access"] = json!(match view.access {
            Access::Read => "read",
            Access::ReadWrite => "read_write",
        });
        oem["Address"] = json!(view.address);
        oem["Function"] = json!(match view.function {
            RegisterFunction::Holding => "holding",
            RegisterFunction::Input => "input",
            RegisterFunction::Coil => "coil",
            RegisterFunction::DiscreteInput => "discrete_input",
        });
        oem["ValueType"] = json!(format!("{:?}", view.value_type).to_lowercase());
        oem["WordOrder"] = json!(format!("{:?}", view.word_order).to_lowercase());
    }

    let mut body = json!({
        "@odata.id": format!("/redfish/v1/Chassis/{}/Sensors/{}", view.device, view.sensor_id),
        "@odata.type": "#Sensor.v1_7_0.Sensor",
        "Id": view.sensor_id,
        "Name": view.name,
        "ReadingType": reading_type(view),
        "Status": status_json(view),
        "Oem": { "Telemux": oem },
    });
    if let Some(v) = view.value {
        body["Reading"] = json!(v);
        if let Some(u) = &view.unit {
            body["ReadingUnits"] = json!(u);
        }
    }
    if view.access == Access::ReadWrite {
        body["Actions"] = json!({
            "#Sensor.SetReading": {
                "target": format!("/redfish/v1/Chassis/{}/Sensors/{}", view.device, view.sensor_id)
            }
        });
    }
    if let Some(ts) = view.timestamp_ms
        && let Ok(dt) = format_ts(ts)
    {
        body["Timestamp"] = json!(dt);
    }
    body
}

fn format_ts(ms: u64) -> Result<String, ()> {
    let secs = ms / 1000;
    let nanos = (ms % 1000) * 1_000_000;
    let d = std::time::UNIX_EPOCH + std::time::Duration::new(secs, nanos as u32);
    Ok(format!("{:?}", d))
}

fn status_json(view: &SensorView) -> Value {
    match view.value {
        None => json!({ "State": "Disabled", "Health": "NA" }),
        Some(_) => {
            let health = match view.status {
                crate::domain::MetricStatus::Normal => "OK",
                crate::domain::MetricStatus::Warning => "Warning",
                crate::domain::MetricStatus::Critical => "Critical",
                crate::domain::MetricStatus::Unknown => "Unknown",
            };
            json!({ "State": "Enabled", "Health": health })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SensorKind {
    Temperature,
    Fan,
    Voltage,
    Current,
    Other,
}

fn classify(view: &SensorView) -> SensorKind {
    let unit = view.unit.as_deref().unwrap_or("").trim().to_lowercase();
    if unit.is_empty() {
        return SensorKind::Other;
    }
    if unit.contains("°c") || unit == "c" || unit.contains("celsius") || unit.contains("deg c") {
        SensorKind::Temperature
    } else if unit.contains("rpm") {
        SensorKind::Fan
    } else if unit.contains('v') {
        SensorKind::Voltage
    } else if unit == "a" || unit == "ma" || unit == "ka" {
        SensorKind::Current
    } else {
        SensorKind::Other
    }
}

fn reading_type(view: &SensorView) -> &'static str {
    match classify(view) {
        SensorKind::Temperature => "Temperature",
        SensorKind::Fan => "Rotational",
        SensorKind::Voltage => "Voltage",
        SensorKind::Current => "Current",
        SensorKind::Other => "Other",
    }
}

// ---------- handlers ----------

async fn service_root(State(_state): State<RedfishState>) -> Json<Value> {
    Json(json!({
        "@odata.id": "/redfish/v1",
        "@odata.type": "#ServiceRoot.v1_16_0.ServiceRoot",
        "Id": "RootService",
        "Name": "Telemux Redfish Service",
        "RedfishVersion": "1.16.0",
        "UUID": "telemux-0000-0000-0000-000000000001",
        "Chassis": { "@odata.id": "/redfish/v1/Chassis" },
    }))
}

async fn chassis_collection(State(state): State<RedfishState>) -> Json<Value> {
    let config = state.config.read();
    let members: Vec<Value> = config
        .devices
        .iter()
        .map(|d| {
            json!({ "@odata.id": format!("/redfish/v1/Chassis/{}", d.name) })
        })
        .collect();
    Json(json!({
        "@odata.id": "/redfish/v1/Chassis",
        "@odata.type": "#ChassisCollection.ChassisCollection",
        "Name": "Chassis Collection",
        "Members@odata.count": members.len(),
        "Members": members,
    }))
}

async fn chassis(
    State(state): State<RedfishState>,
    Path(device): Path<String>,
) -> Response {
    let config = state.config.read();
    let Some(dev) = config.devices.iter().find(|d| d.name == device) else {
        return not_found(&format!("chassis `{device}`"));
    };
    let views = chassis_views(&state, &device);
    let fresh = views.iter().any(|v| v.value.is_some());
    Json(json!({
        "@odata.id": format!("/redfish/v1/Chassis/{device}"),
        "@odata.type": "#Chassis.v1_24_0.Chassis",
        "Id": device,
        "Name": dev.name,
        "ChassisType": "Component",
        "Status": {
            "State": if fresh { "Enabled" } else { "Disabled" },
            "Health": if fresh { "OK" } else { "NA" },
        },
        "Thermal": { "@odata.id": format!("/redfish/v1/Chassis/{device}/Thermal") },
        "Power": { "@odata.id": format!("/redfish/v1/Chassis/{device}/Power") },
        "Sensors": { "@odata.id": format!("/redfish/v1/Chassis/{device}/Sensors") },
    }))
    .into_response()
}

async fn thermal(
    State(state): State<RedfishState>,
    Path(device): Path<String>,
) -> Response {
    if !device_exists(&state, &device) {
        return not_found(&format!("chassis `{device}`"));
    }
    let views = chassis_views(&state, &device);
    let temps: Vec<Value> = views
        .iter()
        .filter(|v| classify(v) == SensorKind::Temperature)
        .map(sensor_json)
        .collect();
    let fans: Vec<Value> = views
        .iter()
        .filter(|v| classify(v) == SensorKind::Fan)
        .map(sensor_json)
        .collect();
    Json(json!({
        "@odata.id": format!("/redfish/v1/Chassis/{device}/Thermal"),
        "@odata.type": "#Thermal.v1_6_2.Thermal",
        "Id": "Thermal",
        "Name": "Thermal",
        "Temps@odata.count": temps.len(),
        "Temps": temps,
        "Fans@odata.count": fans.len(),
        "Fans": fans,
    }))
    .into_response()
}

async fn power(
    State(state): State<RedfishState>,
    Path(device): Path<String>,
) -> Response {
    if !device_exists(&state, &device) {
        return not_found(&format!("chassis `{device}`"));
    }
    let views = chassis_views(&state, &device);
    let voltages: Vec<Value> = views
        .iter()
        .filter(|v| classify(v) == SensorKind::Voltage)
        .map(sensor_json)
        .collect();
    Json(json!({
        "@odata.id": format!("/redfish/v1/Chassis/{device}/Power"),
        "@odata.type": "#Power.v1_7_3.Power",
        "Id": "Power",
        "Name": "Power",
        "Voltages@odata.count": voltages.len(),
        "Voltages": voltages,
    }))
    .into_response()
}

async fn sensor_collection(
    State(state): State<RedfishState>,
    Path(device): Path<String>,
) -> Response {
    if !device_exists(&state, &device) {
        return not_found(&format!("chassis `{device}`"));
    }
    let views = chassis_views(&state, &device);
    let members: Vec<Value> = views
        .iter()
        .map(|v| {
            json!({ "@odata.id": format!("/redfish/v1/Chassis/{device}/Sensors/{}", v.sensor_id) })
        })
        .collect();
    Json(json!({
        "@odata.id": format!("/redfish/v1/Chassis/{device}/Sensors"),
        "@odata.type": "#SensorCollection.SensorCollection",
        "Name": "Sensor Collection",
        "Members@odata.count": members.len(),
        "Members": members,
    }))
    .into_response()
}

async fn sensor(
    State(state): State<RedfishState>,
    Path((device, sensor_id)): Path<(String, String)>,
) -> Response {
    let views = chassis_views(&state, &device);
    match views.iter().find(|v| v.sensor_id == sensor_id) {
        Some(v) => Json(sensor_json(v)).into_response(),
        None => not_found(&format!("sensor `{sensor_id}` on chassis `{device}`")),
    }
}

#[derive(Debug, Deserialize)]
struct PatchBody {
    /// Redfish 传感器写入负载使用大写 `Reading` 键。
    #[serde(default, rename = "Reading")]
    reading: Option<f64>,
}

/// PATCH 一个可读写寄存器：物理值 -> 反算 scale -> 写入 PCBA。
async fn update_sensor(
    State(state): State<RedfishState>,
    Path((device, sensor_id)): Path<(String, String)>,
    Json(body): Json<PatchBody>,
) -> Response {
    let Some(phys) = body.reading else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "`reading` is required" })),
        )
            .into_response();
    };
    let views = build_views(&state.config.read(), &state.store);
    let Some(view) = views.iter().find(|v| v.sensor_id == sensor_id) else {
        return not_found(&format!("sensor `{sensor_id}`"));
    };
    if view.is_computed || view.access != Access::ReadWrite {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            Json(json!({ "error": "sensor is not writable" })),
        )
            .into_response();
    }

    // 反转任何 scale 管道阶段，得到寄存器原始值。
    let raw = reverse_scale(&state.config, &sensor_id, phys);

    let write = match view.function {
        RegisterFunction::Holding => WriteValue::Holding(raw as u16),
        RegisterFunction::Coil => WriteValue::Coil(raw != 0.0),
        _ => {
            return (
                StatusCode::METHOD_NOT_ALLOWED,
                Json(json!({ "error": "register area is not writable" })),
            )
                .into_response()
        }
    };
    match state.broker.write(&device, &sensor_id, write).await {
        Ok(()) => {
            let views = chassis_views(&state, &device);
            let updated = views.iter().find(|v| v.sensor_id == sensor_id).unwrap();
            (StatusCode::OK, Json(sensor_json(updated))).into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": format!("write failed: {e}") })),
        )
            .into_response(),
    }
}

/// 使用传感器的第一个 scale 阶段由物理值计算原始寄存器值
/// （raw = (physical - offset) / scale）；否则为恒等。
fn reverse_scale(config: &ConfigHandle, sensor_id: &str, physical: f64) -> f64 {
    let config = config.read();
    if let Some(pipe) = config.pipelines.iter().find(|p| p.sensor_id == sensor_id)
        && let Some(stage) = pipe.stages.first()
        && let StageConfig::Scale { scale, offset, .. } = stage
        && *scale != 0.0
    {
        return (physical - offset) / scale;
    }
    physical
}

fn device_exists(state: &RedfishState, device: &str) -> bool {
    state.config.read().devices.iter().any(|d| d.name == device)
}

fn not_found(what: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "@odata.type": "#Error.v1_0_0.Error", "error": { "message": format!("{what} not found") } })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::config::{Access, Config, DeviceConfig, RegisterConfig, RegisterFunction, Transport, ValueType, WordOrder};
    use crate::protocol::WriteBroker;

    fn writable_config() -> Config {
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
                    name: "fan_duty".into(),
                    sensor_id: "pcba-01.fan_duty".into(),
                    function: RegisterFunction::Holding,
                    access: Access::ReadWrite,
                    address: 0,
                    count: Some(1),
                    value_type: ValueType::U16,
                    word_order: WordOrder::Big,
                    unit: Some("%".into()),
                }],
            }],
            pipelines: vec![],
            computed: vec![],
            endpoints: Default::default(),
            sim: Default::default(),
        }
    }

    fn app() -> Router {
        router(
            ConfigHandle::new(writable_config(), "unused.toml".into()),
            Arc::new(MetricStore::new()),
            Arc::new(WriteBroker::default()),
        )
    }

    async fn get_status(uri: &str) -> (StatusCode, String) {
        let res = app()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let text = res.into_body().collect().await.unwrap().to_bytes().to_vec();
        (status, String::from_utf8_lossy(&text).into_owned())
    }

    #[tokio::test]
    async fn service_root_and_collection() {
        let (s, body) = get_status("/redfish/v1").await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.contains("ServiceRoot"));
        let (s, body) = get_status("/redfish/v1/Chassis").await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.contains("pcba-01"));
    }

    #[tokio::test]
    async fn unknown_chassis_is_404() {
        let (s, _) = get_status("/redfish/v1/Chassis/nope").await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _) = get_status("/redfish/v1/Chassis/nope/Sensors").await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _) = get_status("/redfish/v1/Chassis/nope/Thermal").await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_sensor_is_404() {
        let (s, _) = get_status("/redfish/v1/Chassis/pcba-01/Sensors/nope").await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn patch_without_reading_is_400() {
        let res = app()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan_duty")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn patch_writable_sensor_fails_without_broker_device() {
        // 空 WriteBroker：设备无 poll 任务 -> 写失败 -> 503。
        let res = app()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan_duty")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"Reading": 60}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let text = res.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&text).unwrap();
        assert!(v["error"].as_str().unwrap().contains("write failed"));
    }
}

