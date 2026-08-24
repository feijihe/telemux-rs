//! 前端静态资源嵌入与服务（telemux-sim 网页 UI）。
//!
//! `web/dist` 由 `web/apps/sim-ui` 的 `pnpm run build` 生成：
//! - 存在时用 `include_dir!` 编译期嵌入（release 二进制自包含）；
//! - 不存在时（未运行 pnpm build）由 build.rs 生成占位 index.html，
//!   保证 cargo 仍可编译（提示先运行 pnpm build）。
//!
//! 服务行为：精确路径命中返回对应文件（含 /assets/*.js 等构建产物）；
//! 否则非 /api 路径回退 index.html（SPA history 回退）。

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::{IntoResponse, Response};

// 由 `web/apps/sim-ui` 构建输出（build.rs 保证目录存在）。
const WEB_DIST: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/web/dist");

/// 网页路由：`GET /` 与任意前端路径（history 回退到 index.html）。
pub async fn index_or_static(req: Request<Body>) -> Response {
    let path = req.uri().path().to_string();
    // 精确文件命中（含 /assets/*.js 等构建产物）。
    if let Some(file) = WEB_DIST.get_file(path.trim_start_matches('/')) {
        return serve_embedded(file).into_response();
    }
    // SPA 回退：非 /api 路径 → index.html。
    if !path.starts_with("/api")
        && let Some(index) = WEB_DIST.get_file("index.html")
    {
        return serve_embedded(index).into_response();
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn serve_embedded(file: &include_dir::File) -> Response {
    let mime = mime_for(file.path().extension().and_then(|s| s.to_str()).unwrap_or(""));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .body(Body::from(file.contents().to_vec()))
        .expect("static response")
}

fn mime_for(ext: &str) -> &'static str {
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "map" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
