#!/usr/bin/env bash
# Telmux-rs Linux 安装脚本（阶段 8）。
#
# 用法：sudo ./deploy/install.sh [--config config/example.toml]
#
# 步骤：
#   1. 构建 release 二进制（若 target/release/telemux 不存在）
#   2. 安装到 /usr/local/bin/telemux
#   3. 创建 /etc/telemux 并复制配置（如未提供则用默认示例）
#   4. 安装 systemd 单元并启用（可选：--no-systemd 跳过）
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_SRC="$ROOT/target/release/telemux"
CONFIG_SRC="${1:-$ROOT/config/example.toml}"
INSTALL_SYSTEMD=1
if [[ "${2:-}" == "--no-systemd" ]]; then INSTALL_SYSTEMD=0; fi

echo "==> 1/4 构建 release 二进制"
if [[ ! -x "$BIN_SRC" ]]; then
    (cd "$ROOT" && cargo build --release)
fi

echo "==> 2/4 安装二进制"
install -m 0755 "$BIN_SRC" /usr/local/bin/telemux

echo "==> 3/4 安装配置"
mkdir -p /etc/telemux
install -m 0644 "$CONFIG_SRC" /etc/telemux/telemux.toml
echo "    配置：/etc/telemux/telemux.toml（请按需编辑，如 log_dir、设备清单）"

if [[ "$INSTALL_SYSTEMD" == "1" ]]; then
    echo "==> 4/4 安装 systemd 单元"
    install -m 0644 "$ROOT/deploy/telemux.service" /etc/systemd/system/telemux.service
    systemctl daemon-reload
    systemctl enable telemux
    echo "完成。启动：systemctl start telemux"
else
    echo "==> 4/4 跳过 systemd（--no-systemd）"
    echo "完成。前台运行：/usr/local/bin/telemux --config /etc/telemux/telemux.toml"
fi
