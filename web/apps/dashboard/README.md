# dashboard（网关开发仪表盘）

telemux 网关（crates/telemux）的开发调试仪表盘：React + TypeScript + Vite + shadcn/ui。仅 dev 构建启用。

## 功能

- 查看各采集设备的状态与原始样本
- 寄存器配置查看（`RegisterModal`）
- 实时调试数据展示

## 开发

```bash
cd web && pnpm install
pnpm --filter dashboard dev     # http://localhost:5181
```

开发服务器将 `/api` 代理到 `http://127.0.0.1:8080`（需先启动 telemux 网关）。

## 构建

```bash
pnpm --filter dashboard build
```

构建产物输出到 `crates/telemux/web/dist`，由 Rust crate 在编译期经 `include_dir!` 嵌入。

## 说明

- `@/` 别名指向 `packages/ui/src`：共享包内 shadcn 组件源码用 `@/lib/utils`、`@/components/ui/*` 相互导入。
- 网关侧 `dashboard` 模块被 `cfg(any(debug_assertions, feature = "dev-dashboard"))` 门控：debug 构建自动启用；release 构建默认排除，需用 `cargo build --release --features dev-dashboard` 才能内置本仪表盘。