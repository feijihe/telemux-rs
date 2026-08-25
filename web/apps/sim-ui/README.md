# sim-ui（模拟器网页 UI）

CDU 仿真器（telemux-sim）的前端：React + TypeScript + Vite + shadcn/ui。

## 功能

- Canvas 二维系统图（PHEX 板换 + 一次/二次回路 + 泵 + 传感器标点，实时值）
- 温度设定（一次侧冷水 / 二次侧热水）+ 泵/阀/风扇 duty，立即联动全系统温度
- 寄存器地图原始值表（地址 / 类型 / 原始值）

## 开发

```bash
cd web && pnpm install
pnpm --filter sim-ui dev        # http://localhost:5180
```

开发服务器将 `/api` 代理到 `http://127.0.0.1:8082`（需先启动 telemux-sim）。

## 构建

```bash
pnpm --filter sim-ui build
```

构建产物输出到 `crates/telemux-sim/web/dist`，由 Rust crate 在编译期经 `include_dir!` 嵌入（SPA history 回退由 `web_assets.rs` 处理）。

## 说明

- `@/` 别名指向 `packages/ui/src`：共享包内 shadcn 组件源码用 `@/lib/utils`、`@/components/ui/*` 相互导入。
- 状态通过 WebSocket（`/api/ws`）与 HTTP（`/api/state`）获取，两条通道共用同一份 JSON。