#!/usr/bin/env bash
# telemux-sim Linux 安装脚本：二进制 + 可选配置 + systemd 单元（开机自启）。
#
# 用法（在 crates/telemux-sim 下执行，或任意位置用绝对路径）：
#   sudo ./deploy/install.sh                                   # 用内嵌配置，端口 1502/8082
#   sudo ./deploy/install.sh --config config/cdu.toml          # 指定仿真配置
#   sudo ./deploy/install.sh --bin /tmp/telemux-sim            # 显式指定二进制（交叉编译产物）
#   sudo ./deploy/install.sh --modbus-port 1502 --web-port 8082
#   sudo ./deploy/install.sh --no-systemd                      # 只装二进制，不注册服务
#
# 二进制查找顺序：--bin → target/aarch64-unknown-linux-musl/release/telemux-sim
#   → target/release/telemux-sim → 当前目录/脚本目录下的 telemux-sim。
#
# 安装产物：
#   /usr/local/bin/telemux-sim                可执行文件
#   /etc/telemux-sim/sim.toml                 仿真配置（仅 --config 时安装）
#   /etc/telemux-sim/sim.env                  systemd 启动参数（TELEMUX_SIM_ARGS / RUST_LOG）
#   /etc/systemd/system/telemux-sim.service   systemd 单元（enable --now）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WS_ROOT="$(cd "$CRATE_DIR/../.." && pwd)"

BIN=""
CONFIG=""
MODBUS_PORT=1502
WEB_PORT=8082
INSTALL_SYSTEMD=1

usage() {
    cat <<'EOF'
用法：sudo ./deploy/install.sh [选项]

  --bin <路径>         指定 telemux-sim 二进制（默认自动查找交叉编译/本地 release 产物）
  --config <路径>      安装为 /etc/telemux-sim/sim.toml 并传给服务（缺省用二进制内嵌配置）
  --modbus-port <端口> Modbus-TCP 从站端口（默认 1502）
  --web-port <端口>    网页 UI 端口（默认 8082）
  --no-systemd         仅安装二进制，不注册 systemd 服务
  -h, --help           显示本帮助
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --bin) BIN="${2:?--bin 需要路径}"; shift 2 ;;
        --config) CONFIG="${2:?--config 需要路径}"; shift 2 ;;
        --modbus-port) MODBUS_PORT="${2:?--modbus-port 需要端口}"; shift 2 ;;
        --web-port) WEB_PORT="${2:?--web-port 需要端口}"; shift 2 ;;
        --no-systemd) INSTALL_SYSTEMD=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "未知参数：$1（用 --help 查看用法）" >&2; exit 2 ;;
    esac
done

# 1. 定位二进制（交叉编译产物优先）
# 用 -f 而非 -x 判断：从 Windows scp 过来的产物常无执行位，权限由 install -m 0755 统一设置
if [[ -z "$BIN" ]]; then
    for candidate in \
        "$WS_ROOT/target/aarch64-unknown-linux-musl/release/telemux-sim" \
        "$CRATE_DIR/target/aarch64-unknown-linux-musl/release/telemux-sim" \
        "$WS_ROOT/target/release/telemux-sim" \
        "$CRATE_DIR/target/release/telemux-sim" \
        "$PWD/telemux-sim" \
        "$SCRIPT_DIR/telemux-sim"
    do
        if [[ -f "$candidate" ]]; then BIN="$candidate"; break; fi
    done
fi
if [[ -z "$BIN" || ! -f "$BIN" ]]; then
    echo "找不到 telemux-sim 二进制文件。" >&2
    echo "请先交叉编译/构建（cargo build --release -p telemux-sim --target aarch64-unknown-linux-musl），" >&2
    echo "或用 --bin <路径> 指定二进制。" >&2
    exit 1
fi

if [[ "$(id -u)" -ne 0 ]]; then
    echo "需要 root 权限：sudo $0 --bin $BIN ..." >&2
    exit 1
fi

# 2. 安装二进制
echo "==> 1/3 安装二进制"
if [[ "$(head -c 4 "$BIN" | od -An -tx1 | tr -d ' \n')" != "7f454c46" ]]; then
    echo "    警告：$BIN 不是 ELF（Linux）二进制，请确认没有误用 Windows/其它目标的产物" >&2
fi
install -m 0755 "$BIN" /usr/local/bin/telemux-sim
echo "    $BIN → /usr/local/bin/telemux-sim"

ARGS=("--modbus-port" "$MODBUS_PORT" "--web-port" "$WEB_PORT")

# 3. 安装配置（可选）
echo "==> 2/3 配置"
mkdir -p /etc/telemux-sim
if [[ -n "$CONFIG" ]]; then
    if [[ ! -f "$CONFIG" ]]; then echo "配置文件不存在：$CONFIG" >&2; exit 1; fi
    install -m 0644 "$CONFIG" /etc/telemux-sim/sim.toml
    ARGS+=("--config" "/etc/telemux-sim/sim.toml")
    echo "    已安装：/etc/telemux-sim/sim.toml（来自 $CONFIG）"
else
    echo "    未指定 --config：使用二进制编译期嵌入的仿真配置（自包含，无需外部文件）"
fi

# 4. systemd 单元（开机自启）
if [[ "$INSTALL_SYSTEMD" == "1" ]]; then
    echo "==> 3/3 注册 systemd 服务"
    printf 'TELEMUX_SIM_ARGS=%s\nRUST_LOG=info\n' "${ARGS[*]}" > /etc/telemux-sim/sim.env
    chmod 0644 /etc/telemux-sim/sim.env
    install -m 0644 "$SCRIPT_DIR/telemux-sim.service" /etc/systemd/system/telemux-sim.service
    if [[ -d /run/systemd/system ]]; then
        systemctl daemon-reload
        systemctl enable --now telemux-sim
        echo "    已启用并启动：$(systemctl is-active telemux-sim 2>/dev/null || echo unknown)"
        echo "完成。状态：systemctl status telemux-sim；日志：journalctl -u telemux-sim -f"
    else
        echo "    未检测到运行中的 systemd（/run/systemd/system 不存在）：文件已安装，请手动启用" >&2
        echo "    手动：systemctl daemon-reload && systemctl enable --now telemux-sim" >&2
    fi
else
    echo "==> 3/3 跳过 systemd（--no-systemd）"
    echo "完成。前台运行：/usr/local/bin/telemux-sim ${ARGS[*]}"
fi
