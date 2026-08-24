# Dev Dashboard（开发调试面板）设计文档

> 状态：**第一、二、三期均已实现并验证**。第三期 = 公式可视化 + 动态创建寄存器（热更新）。

## 1. 功能定位

开发环境专属的本地 Web 调试面板：

- 表格展示**寄存器配置**（来自 `config/example.toml` 等）
- 展示**最近一次原始值**（`RawSample`）与**计算后结果**（`Metric`，阶段 3/4 完成后才有）
- 浏览器通过 **WebSocket** 连接，数据**实时刷新**（约 1Hz 全量快照）
- 仅开发环境可用；**生产（release 打包）二进制不含此功能**

价值：替代终端日志联调；pipeline（换算/滤波/阈值）上线后，"原始值 → 计算值"对照是调试刚需；
同时为阶段 5（Redfish HTTP 服务）提前演练 axum 基建。

## 2. 已确认决策（含实现时的调整）

| 决策点 | 结论 |
|---|---|
| 环境隔离 | **方案 C**：`cfg(any(debug_assertions, feature = "dev-dashboard"))` 门控整个模块。debug 构建自动启用（零配置）；release 默认无；需要时 `--features dev-dashboard` 可强制开（排查线上） |
| 实现时机 | **分两期**：第一期只显示原始值（已完成）；阶段 3/4 完成后扩展计算值列 |
| 数据来源 | 寄存器配置直接读 `Config`；原始值来自 `RawSampleStore`（`src/store.rs`，不门控，阶段 4 演进为完整 metric store） |
| 推送机制 | 后台任务按最小 poll 间隔（下限 250ms）读 Store 快照 → `tokio::sync::broadcast` → 推送全量 JSON 给所有 WS 客户端 |
| 前端形态 | **React + TypeScript + shadcn/ui**（`web/apps/dashboard`，pnpm workspace），构建产物 `web/dist` 由 `include_dir!` 编译期嵌入（`src/dashboard/web_assets.rs`，SPA history 回退）；未构建时 build.rs 生成占位页。开发时 `pnpm --filter dashboard dev`（vite，端口 5181，代理 `/api`→8080） |
| 绑定 | `127.0.0.1:8080`（仅本机，无鉴权；`--dashboard-port` 可覆盖） |
| 依赖（实现时调整） | axum/serde_json/futures-util 为**常驻依赖**而非 optional：cargo 无法按 profile 启用 optional 依赖（debug 下模块编译但依赖不在图内会报错）。面板模块仍由 cfg 严格门控，release 产物不含面板代码（已验证：无相关字符串、端口不监听）；axum 亦为阶段 5 所需 |

## 3. 总体架构与数据流

```
[PCBA 设备]
   │  SensorSource::read_samples()
   ▼
RawSample（原始值）──▶ pipeline（阶段3）──▶ Metric
   │                                          │
   ▼                                          ▼
RawSampleStore（src/store.rs，保留最近值）   Metric Store（阶段4）
   │                                          │
   └────────────▶ [dev dashboard]（cfg 门控）
                  ├─ GET /             → 内嵌 HTML 页面
                  ├─ GET /api/snapshot → JSON 全量快照（轮询兜底）
                  └─ WS  /api/ws       → 实时快照推送（按 poll 间隔）
                  127.0.0.1:8080
```

**对阶段 4 的约束**：Metric Store 需同时保留最近一次 `RawSample`（原始值）和 `Metric`（计算值），
面板第二期把数据源从 `RawSampleStore` 换到 Metric Store 即可，前端不变。

## 4. 数据模型（JSON 快照示例）

```json
{
  "generated_at_ms": 1724300000000,
  "devices": [
    {
      "name": "pcba-01",
      "transport": "tcp",
      "host": "127.0.0.1", "port": 1502,
      "connected": true,
      "registers": [
        {
          "name": "cpu_temp_raw",
          "sensor_id": "pcba-01.cpu_temp",
          "function": "holding",
          "address": 0, "count": 1, "value_type": "u16", "word_order": "big",
          "unit": "counts",
          "raw":    { "value": 251, "timestamp_ms": 1724300000000 },
          "metric": { "value": 25.1, "unit": "°C", "status": "normal", "timestamp_ms": 1724300000000 }
        }
      ]
    }
  ]
}
```

第一期 `metric` 为 `null`；阶段 4 后填充。前端表格列：
`寄存器 | sensor_id | 功能 | 地址 | 类型 | 字序 | 单位 | 原始值 | 计算值 | 状态 | 更新时间`。
掉线/无数据显示灰色 `—`；阈值状态着色（normal 绿 / warning 黄 / critical 红）；
`connected` 由最近采样时间与 poll 间隔推导（2×poll 内为在线）。

## 5. 实现状态（第一期已完成）

| 组件 | 位置 | 状态 |
|---|---|---|
| RawSampleStore | `src/store.rs` | ✅ 完成（不门控，阶段 4 演进） |
| 快照模型与组装 | `src/dashboard/snapshot.rs` | ✅ 完成（含单测） |
| HTTP/WS 服务 | `src/dashboard/server.rs` | ✅ 完成 |
| 前端单页 | `src/dashboard/index.html` | ✅ 完成 |
| main 接线 | `src/main.rs`（cfg 门控 spawn） | ✅ 完成 |
| 生产排除验证 | — | ✅ 已验证（release 无面板字符串、8080 不监听） |

**已验证**：debug 构建下 `GET /` 返回页面、`/api/snapshot` 返回实时 JSON、WS 每秒推送
变化数据；release 构建（无 feature）二进制不含面板代码、不监听 8080。

## 6. 第二期（已完成）

- `snapshot.rs` 填充 `metric` 字段（从 Metric Store 读，同时保留 raw + metric）✅
- 前端已展示 raw + metric 双列与状态着色 ✅

## 7. 第三期：公式可视化 + 动态创建寄存器（设计已确认，未实现）

### 7.1 需求

1. **公式可视化**：前端能看到"原始值 → 计算值"的计算链路；配置文件中也能查到公式语义。
2. **动态创建寄存器**：Web 界面创建新的寄存器地址，校验通过提交后表格自动更新（不重启网关）。

### 7.2 已确认决策

| 决策点 | 结论 |
|---|---|
| 创建范围（第一期） | **仅已有设备上新增寄存器**（复用连接，热更新路径最简，≤1 个 poll 周期生效）；新设备创建、寄存器编辑/删除列为后续 |
| 公式展示形式 | **点击行展开详情面板**（公式链路 + 阈值着色说明 + 参数），表格保持精简 |
| 公式来源 | 后端从 stage 配置**自动生成**（非手写，避免不一致） |
| 持久化 | 提交后**写回 TOML 配置文件**（重启保留）；写文件失败仅 warn（内存已生效） |
| 新寄存器 pipeline | 可选携带；不配则 metric 显示 `—`（与 uptime_ticks 一致） |

### 7.3 公式生成规则（后端 `StageConfig::formula()` / `PipelineConfig::formula()`）

| Stage | 生成公式示例 |
|---|---|
| `scale` | `v = v × 0.1 + 0 → °C` |
| `sliding_average` | `v = avg(最近 5 个值)` |
| `median` | `v = median(最近 3 个值)` |
| `math` | `v = (v - 273.15) × 10`（原样展示表达式） |
| `threshold` | `状态: <5 critical / <10 warning / >30 warning / >35 critical` |
| `aggregate` | `v = avg(窗口 4)` |

管道级公式用 ` → ` 串联各 stage；snapshot 每个寄存器加 `formula: Option<String>`（无 pipeline 为 null）。

### 7.4 动态创建：架构变化

配置从"启动时不可变"变"运行时可变"，但**仅限开发环境**——通过 `ConfigHandle` 编译期隔离：

```
开发环境（debug_assertions 或 --features dev-dashboard）:
    ConfigHandle.inner = Arc<RwLock<Config>>    ← 有 update()，可热更新
生产环境（release，无 feature）:
    ConfigHandle.inner = Arc<Config>            ← 只有 read()，纯只读

                   ConfigHandle.read()/revision()  ← 各模块统一入口（零 cfg 分支）
                                        ▲
                         POST /api/registers（dev-only，cfg 门控的 dashboard 模块内）
                              └─▶ update()（仅 dev 编译期存在）→ 校验 → 更新内存 + 写回 TOML
```

**ConfigHandle 设计要点**：

```rust
pub struct ConfigHandle {
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    inner: Arc<RwLock<Config>>,
    #[cfg(not(any(debug_assertions, feature = "dev-dashboard")))]
    inner: Arc<Config>,
    revision: Arc<AtomicU64>,   // update 时 +1；prod 恒 0
}
```

- `read()` 返回 Config 克隆（配置小，每轮 clone 可接受）；`revision()` 供 PipelinesCache 判断变化
- `update(f)` **仅开发环境存在**（`#[cfg(any(debug_assertions, feature = "dev-dashboard"))]`）：
  写锁 → 应用修改 → `validate()` 权威校验 → revision+1；生产构建中该方法不存在，调用点编译失败 → 天然保证"生产不可变"
- 生产二进制不含 RwLock / update 逻辑（编译期剔除，需验证）

| 组件 | 变化 |
|---|---|
| `Config` | 新增 `ConfigHandle` 包装（cfg 双版本内部容器）+ `save(path)`（TOML 写回）+ `add_register`（校验逻辑） |
| 采集层 | `SensorSource::read_samples` 改为接收 `registers: &[RegisterConfig]`；设备任务每轮 poll 从 `ConfigHandle` **热读**该设备寄存器列表（读锁 → clone → 释放 → IO） |
| pipeline | consumer 用 `PipelinesCache`（`revision()` 变化时重建 pipeline map） |
| dashboard | `DashboardState` 持 `ConfigHandle` + 配置文件路径；新增 `POST /api/registers`（调用 `update()`） |
| 前端 | 行点击展开详情（公式 + 阈值说明）；"新增寄存器"表单 + 错误展示；提交成功后由现有 WS 快照机制自动刷新表格 |

### 7.5 创建流程与校验

```
表单填写 → 前端基础校验（必填/数字/格式）
        → POST /api/registers
        → 服务端权威校验：
            · sensor_id 全局唯一（跨设备）
            · 地址不与同设备同功能码寄存器重叠
            · count 与 value_type 匹配
            · pipeline stages 合法（复用 validate_stage，表达式可解析）
        → 通过：写锁更新内存 Config → 写回 TOML → 200（下个快照自动出现新行）
        → 失败：400 + 具体字段错误（前端展示）
```

### 7.6 实现状态（第三期已完成 ✅）

| 步骤 | 内容 | 状态 |
|---|---|---|
| 1 | `describe_stage`/`describe_pipeline`（pipeline/mod.rs）+ snapshot 加 `formula`/`stages` 字段 | ✅（含单测） |
| 2 | 前端行点击展开详情（公式链路 + 阈值说明） | ✅ |
| 3 | `ConfigHandle`（cfg 双版本：dev 可变 / prod 只读）+ `save()` + `Config::add_register()` | ✅（含单测） |
| 4 | 采集层热读：`SensorSource::read_samples(registers)` + manager 每轮从 ConfigHandle 取配置 | ✅ |
| 5 | `PipelinesCache`（revision 重建） | ✅ |
| 6 | `POST /api/registers` + 前端表单 + 错误展示 | ✅ |
| 7 | 文档 | ✅ |

**已验证**：
- 公式字段（`formula` + 分步 `stages`）随快照输出，前端行点击展开
- `POST /api/registers` 成功添加后，下个 poll 周期快照自动出现新行（含带 pipeline 的新寄存器，metric 正常计算）——无需重启
- 校验失败返回 400 + 具体错误（重复 sensor_id / 地址重叠 / 未知设备等）
- 提交后配置写回 TOML（重启保留）
- **生产只读**：release 二进制不含 dashboard / `api/registers` / update 逻辑（字符串验证通过）

### 7.8 传输优化（增量协议，已完成 ✅）

**问题**：WS 每秒推送完整配置表 + 前端全量重渲染 → 带宽浪费 + 打断展开的公式详情行。

**方案**（HTTP 全量 / WS 增量分离）：

| 数据 | 通道 | 内容 | 时机 |
|---|---|---|---|
| 完整寄存器表（静态配置 + 当前值） | HTTP `GET /api/snapshot` | 全量 | 首次打开、新增寄存器成功后 |
| 动态值更新 | WS `/api/ws` | **增量**：`{"type":"update","samples":[{sensor_id, raw, metric}]}` | 每秒（仅 raw + metric） |

- 增量消息实测：**991 字节 vs 全量 2338 字节（缩小 ~57%）**，且不含任何静态配置
- 前端按 `sensor_id` 定点更新数据单元格（`sensorMap`），**不重建 DOM** → 展开行、滚动位置稳定不被打断
- 新增寄存器提交流程：`stopWs()`（暂停更新）→ POST 成功则 `loadFullSnapshot()`（HTTP 重取全表）+ `connect()`（恢复增量）；失败则仅显示错误 + `connect()` 恢复
- 新增寄存器首次 poll 前不在增量里 → 前端该行保持占位 `—`，首次数据到达自动填充

### 7.7 风险

- Config 可变后所有读取点需审计（dashboard snapshot / manager / consumer），统一走 `ConfigHandle::read()`
- 写锁保持短暂（校验 + 插入），文件 IO 放锁外
- 写回失败仅 warn 不阻塞；内存配置始终先行生效
- 动态添加与现有校验逻辑保持一致（复用 `validate` 的规则拆分）
- **生产只读验证**：release 构建确认 ConfigHandle 无 RwLock/update 逻辑（字符串/行为验证，同 dashboard 排除验证）

## 8. 边界与风险

- 端口规划：dashboard `8080` 与阶段 5 Redfish（计划 8000/自定义）分开
- 多客户端：broadcast 天然支持多标签页
- release 下如需临时启用：`cargo build --release --features dev-dashboard`（仅排查用）
- 依赖常驻带来的 release 编译成本：axum 等始终编译（LTO+strip 剔除未用代码），
  若未来追求极致体积，可改为 optional + 构建脚本注入 cfg
