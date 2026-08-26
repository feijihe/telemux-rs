// 共享类型：telemux-sim 模拟器 API 与 telemux 网关 dashboard API。

// ============ telemux-sim（模拟器） ============

/** 控制变量（模拟器 /api/state -> controls） */
export interface SimControl {
  name: string;
  value: number;
  unit: string | null;
  writable: boolean;
}

/** 仿真传感器（模拟器 /api/state -> sensors） */
export interface SimSensor {
  sensor_id: string;
  name: string;
  kind: string;
  unit: string | null;
  formula: string;
  value: number | null;
}

/** 保持寄存器槽位：控制变量或只读传感器 */
export interface HoldingSlot {
  type: "control" | "sensor";
  /** type=control 时的控制变量名 */
  control?: string;
  /** type=control 时是否可写 */
  writable?: boolean;
  /** type=sensor 时的传感器 id */
  sensor?: string;
  /** type=sensor 时的存储格式 */
  storage?: "f32" | "u16";
}

export interface HoldingEntry {
  addr: number;
  slot: HoldingSlot | null;
  raw: number;
}

/** 寄存器地图输入区条目（f32 双字或 u16 单字） */
export interface InputEntry {
  addr: number;
  sensor: string | null;
  /** "f32"（双字）| "u16"（单字） */
  storage?: "f32" | "u16";
  raw_hi: number;
  raw_lo: number | null;
  value_f32: number | null;
}

/** 模拟器完整状态 */
export interface SimState {
  controls: SimControl[];
  sensors: SimSensor[];
  holding: HoldingEntry[];
  inputs: InputEntry[];
}

/** 回路侧分组（一次/二次侧 in/out/aux） */
export interface SideSensors {
  in: SimSensor[];
  out: SimSensor[];
  aux: SimSensor[];
}

// ============ telemux 网关 dev dashboard ============

/** 传感器状态（normal/warning/critical/unknown） */
export type MetricStatus = "normal" | "warning" | "critical" | "unknown";

/** 原始样本值 */
export interface ValueSnapshot {
  value: number;
  timestamp_ms: number;
}

/** 处理后指标 */
export interface MetricSnapshot {
  value: number;
  unit: string | null;
  status: MetricStatus;
  timestamp_ms: number;
}

/** 单个寄存器（配置 + 最新值） */
export interface RegisterSnapshot {
  name: string;
  sensor_id: string;
  function: string;
  address: number;
  count: number;
  value_type: string;
  word_order: string;
  unit: string | null;
  raw: ValueSnapshot | null;
  metric: MetricSnapshot | null;
  formula: string | null;
  stages: string[];
}

/** 设备快照 */
export interface DeviceSnapshot {
  name: string;
  transport: string;
  host: string;
  port: number;
  connected: boolean;
  registers: RegisterSnapshot[];
}

/** 完整快照（GET /api/snapshot） */
export interface DashboardSnapshot {
  generated_at_ms: number;
  devices: DeviceSnapshot[];
}

/** 增量更新样本（WS /api/ws） */
export interface SampleUpdate {
  sensor_id: string;
  raw: ValueSnapshot | null;
  metric: MetricSnapshot | null;
}

/** 增量更新消息 */
export interface UpdateMessage {
  type: "update";
  generated_at_ms: number;
  samples: SampleUpdate[];
}

/** 新增寄存器请求体（POST /api/registers） */
export interface CreateRegisterRequest {
  device: string;
  register: {
    name: string;
    sensor_id: string;
    function: string;
    address: number;
    count: number | null;
    value_type: string;
    word_order: string;
    unit: string | null;
    access: string;
  };
  pipeline?: {
    sensor_id: string;
    stages: unknown[];
  } | null;
}
