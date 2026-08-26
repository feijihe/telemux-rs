# telemux-sim（CDU 仿真器 crate）

开发/测试用的 CDU 仿真器：**物理模型 + Modbus-TCP 从站 + 网页 UI**。无需真实硬件，用配置驱动的稳态代数模型模拟一台液冷 CDU 的全部传感器，供网关以与真实设备完全同构的方式接入。

## 目录

- `src/` — `model`（物理模型/稳态求值）、`registers`（Modbus 寄存器地图）、`server`（Modbus-TCP 从站）、`web`（网页 UI）、`web_assets.rs`（前端资源嵌入）
- `config/cdu.toml` — 仿真配置（传感器按回路/出入口分组）
- `web/dist/` — 前端构建产物，由 `web/apps/sim-ui` 生成，编译期经 `include_dir!` 嵌入

## 运行

```bash
cargo run -p telemux-sim -- --config crates/telemux-sim/config/cdu.toml \
    --modbus-port 1502 --web-port 8082
```

- Modbus-TCP 从站：`1502`
- 网页 UI：`http://127.0.0.1:8082`（仅绑定本机）

## 配置结构

传感器按**回路侧 → 出入口**两级组织（`config/cdu.toml`）：

- 一次侧：`[[sim.pri.in]]`（入口）、`[[sim.pri.out]]`（出口）、`[[sim.pri.aux]]`（流量/液位辅助）
- 二次侧：`[[sim.sec.in]]`、`[[sim.sec.out]]`、`[[sim.sec.aux]]`（流量 + 泵）
- 水箱/环境/泄漏：`[[sim.sensors]]` 平铺

每个传感器通过 `formula`（meval 稳态表达式）从控制变量与其他传感器求值；`sin(t)`/`cos(t)` 时变项产生数值抖动。`SimConfig::iter_sensors()` 按 in/out/aux 顺序扁平化，Modbus 输入寄存器地址按组连续划分。

## 构建注意

- 前端需先构建一次（`cd web && pnpm run build`），否则 `build.rs` 生成占位 `index.html`，二进制不含真实 UI。
- 主函数 `#[tokio::main(flavor = "current_thread")]`，单线程 runtime。