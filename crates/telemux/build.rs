//! 构建脚本：确保 `web/dist` 存在，供 `include_dir!` 编译期嵌入。
//!
//! 若 `web/apps/dashboard` 尚未执行 `pnpm run build`，则生成一个最小占位
//! index.html（提示先运行 pnpm build），保证 `cargo build` 不因目录缺失而失败。

use std::fs;
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dist = Path::new(&manifest).join("web/dist");
    if !dist.exists() {
        fs::create_dir_all(&dist).expect("create web/dist");
        let placeholder = r#"<!doctype html>
<html lang="zh-CN">
<head><meta charset="UTF-8"><title>Telemux Dev Dashboard</title></head>
<body style="font-family: monospace; padding: 32px;">
  <h1>Telemux Dev Dashboard</h1>
  <p>前端尚未构建：请先运行 <code>pnpm --filter dashboard build</code>（在仓库根 <code>web/</code> 目录），
  然后重新 <code>cargo build</code>。</p>
</body>
</html>
"#;
        fs::write(dist.join("index.html"), placeholder).expect("write placeholder index.html");
        println!("cargo:warning=web/dist 不存在，已生成占位 index.html（请运行 pnpm --filter dashboard build 构建前端）");
    }
}
