#!/usr/bin/env bash
# AnyClaude 一键安装脚本
# 用法: curl -fsSL <url>/install.sh | bash
#   或: bash install.sh

set -euo pipefail

REPO="arttttt/AnyClaude"
INSTALL_DIR="${HOME}/.local/bin"
CONFIG_DIR="${HOME}/.config/anyclaude"
CONFIG_FILE="${CONFIG_DIR}/config.toml"
BINARY="anyclaude"

# ── 颜色输出 ─────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()    { echo -e "${CYAN}[info]${NC} $*"; }
success() { echo -e "${GREEN}[✓]${NC} $*"; }
warn()    { echo -e "${YELLOW}[warn]${NC} $*"; }
error()   { echo -e "${RED}[error]${NC} $*"; exit 1; }

# ── 检测平台 ─────────────────────────────────────────────────────────────────
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  OS="linux" ;;
        Darwin) OS="macos" ;;
        *)      error "不支持的操作系统: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) error "不支持的架构: $arch" ;;
    esac
}

# ── 从源码编译安装（无预构建包时的回退方案）─────────────────────────────────
install_from_source() {
    info "未找到预构建包，尝试从源码编译..."

    if ! command -v cargo &>/dev/null; then
        info "未检测到 Rust，正在安装..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
        # shellcheck source=/dev/null
        source "${HOME}/.cargo/env"
    fi

    local tmp_dir
    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' EXIT

    info "克隆仓库..."
    git clone --depth=1 "https://github.com/${REPO}.git" "$tmp_dir/anyclaude"

    info "编译中（release），请稍候..."
    (cd "$tmp_dir/anyclaude" && cargo build --release)

    install -Dm755 "$tmp_dir/anyclaude/target/release/${BINARY}" "${INSTALL_DIR}/${BINARY}"
}

# ── 主安装流程 ────────────────────────────────────────────────────────────────
main() {
    echo ""
    echo -e "${CYAN}╔══════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║      AnyClaude 安装程序              ║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════╝${NC}"
    echo ""

    detect_platform
    info "平台: ${OS}/${ARCH}"

    # 创建安装目录
    mkdir -p "${INSTALL_DIR}"

    # 尝试下载预构建二进制
    local latest_tag installed_via="prebuilt"
    if command -v curl &>/dev/null; then
        latest_tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            2>/dev/null | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/' || echo "")
    fi

    if [[ -n "$latest_tag" ]]; then
        local asset_name="${BINARY}-${OS}-${ARCH}"
        local download_url="https://github.com/${REPO}/releases/download/${latest_tag}/${asset_name}"

        info "尝试下载 ${latest_tag} (${asset_name})..."
        if curl -fsSL "$download_url" -o "${INSTALL_DIR}/${BINARY}" 2>/dev/null; then
            chmod +x "${INSTALL_DIR}/${BINARY}"
            success "下载成功: ${latest_tag}"
        else
            warn "预构建包不存在，切换到源码编译"
            install_from_source
            installed_via="source"
        fi
    else
        warn "无法获取最新版本信息，切换到源码编译"
        install_from_source
        installed_via="source"
    fi

    # ── 交互式 WebUI 配置向导 ───────────────────────────────────────────────
    local webui_user webui_pass webui_pass2 webui_bind webui_url

    echo ""
    echo -e "${CYAN}─────────────────────────────────────────${NC}"
    echo -e "${CYAN}  WebUI 配置向导（管理界面账号与访问权限）${NC}"
    echo -e "${CYAN}─────────────────────────────────────────${NC}"
    echo ""

    # 访问模式
    echo -e "  请选择 WebUI 访问模式："
    echo -e "    ${CYAN}1${NC}) 仅本机访问（localhost，最安全）"
    echo -e "    ${CYAN}2${NC}) 局域网访问（内网所有设备可访问）"
    echo -e "    ${CYAN}3${NC}) 公网访问（任意 IP 可访问，需设置密码）"
    echo -n "  请输入选项 [1-3，默认 1]: "
    local access_mode
    read -r access_mode </dev/tty || true
    access_mode="${access_mode:-1}"

    case "$access_mode" in
        2) webui_bind="0.0.0.0:47191" ;;
        3) webui_bind="0.0.0.0:47191" ;;
        *) webui_bind="127.0.0.1:47191" ;;
    esac

    # 是否启用账号密码
    local enable_auth="n"
    if [[ "$access_mode" == "2" || "$access_mode" == "3" ]]; then
        echo -e "  ${YELLOW}提示：局域网/公网访问强烈建议设置账号密码${NC}"
        enable_auth="y"
    else
        echo -n "  是否启用账号密码保护？[y/N，默认 N]: "
        read -r enable_auth </dev/tty || true
        enable_auth="${enable_auth:-n}"
    fi

    if [[ "$enable_auth" =~ ^[Yy]$ ]]; then
        # 用户名
        echo -n "  管理员用户名 [默认 admin]: "
        read -r webui_user </dev/tty || true
        webui_user="${webui_user:-admin}"

        # 密码（带确认）
        while true; do
            echo -n "  管理员密码（输入不显示）: "
            read -rs webui_pass </dev/tty || true
            echo ""
            if [[ -z "$webui_pass" ]]; then
                echo -e "  ${RED}密码不能为空，请重新输入${NC}"
                continue
            fi
            echo -n "  再次确认密码: "
            read -rs webui_pass2 </dev/tty || true
            echo ""
            if [[ "$webui_pass" == "$webui_pass2" ]]; then
                break
            fi
            echo -e "  ${RED}两次密码不一致，请重新输入${NC}"
        done
        success "账号密码已设置（用户名: ${webui_user}）"
    else
        webui_user=""
        webui_pass=""
        warn "未设置账号密码，WebUI 将无需登录即可访问"
    fi

    # ── 写入配置（仅在配置文件不存在时）──────────────────────────────────
    if [[ ! -f "$CONFIG_FILE" ]]; then
        mkdir -p "$CONFIG_DIR"

        # 构建 [webui] 段
        local webui_section
        webui_section="[webui]
bind_addr = \"${webui_bind}\""
        if [[ -n "$webui_user" && -n "$webui_pass" ]]; then
            webui_section="${webui_section}
username  = \"${webui_user}\"
password  = \"${webui_pass}\""
        fi

        cat > "$CONFIG_FILE" <<EOF
[defaults]
active = "anthropic"

[proxy]
bind_addr = "127.0.0.1:47190"
base_url  = "http://127.0.0.1:47190"

${webui_section}

[[backends]]
name         = "anthropic"
display_name = "Anthropic (官方)"
base_url     = "https://api.anthropic.com"
auth_type    = "passthrough"
EOF
        success "已生成配置: ${CONFIG_FILE}"
    else
        info "配置文件已存在，跳过自动生成: ${CONFIG_FILE}"
        warn "如需更新访问设置，请手动编辑 [webui] 段"
    fi

    # 计算 WebUI 访问 URL（用于最终提示）
    if [[ "$webui_bind" == "0.0.0.0:47191" ]]; then
        # 尝试获取本机局域网 IP
        local lan_ip
        lan_ip=$(ip route get 1.1.1.1 2>/dev/null | awk '{print $7; exit}' || hostname -I 2>/dev/null | awk '{print $1}' || echo "YOUR_IP")
        webui_url="http://${lan_ip}:47191"
    else
        webui_url="http://127.0.0.1:47191"
    fi

    # ── 检查 PATH ────────────────────────────────────────────────────────────
    local shell_rc=""
    if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
        warn "${INSTALL_DIR} 不在 PATH 中，正在自动添加..."

        case "${SHELL##*/}" in
            zsh)  shell_rc="${HOME}/.zshrc" ;;
            bash) shell_rc="${HOME}/.bashrc" ;;
            fish) shell_rc="${HOME}/.config/fish/config.fish" ;;
            *)    shell_rc="${HOME}/.profile" ;;
        esac

        if [[ "${SHELL##*/}" == "fish" ]]; then
            echo "fish_add_path ${INSTALL_DIR}" >> "$shell_rc"
        else
            echo "export PATH=\"\${PATH}:${INSTALL_DIR}\"" >> "$shell_rc"
        fi
        warn "已写入 ${shell_rc}，请运行: source ${shell_rc}"
    fi

    # ── 安装完成 ─────────────────────────────────────────────────────────────
    local version
    version=$("${INSTALL_DIR}/${BINARY}" --version 2>/dev/null || echo "unknown")

    echo ""
    echo -e "${GREEN}╔══════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║      安装完成！                      ║${NC}"
    echo -e "${GREEN}╚══════════════════════════════════════╝${NC}"
    echo ""
    echo -e "  二进制:    ${CYAN}${INSTALL_DIR}/${BINARY}${NC}"
    echo -e "  版本:      ${CYAN}${version}${NC}"
    echo -e "  安装方式:  ${CYAN}${installed_via}${NC}"
    echo -e "  配置文件:  ${CYAN}${CONFIG_FILE}${NC}"
    echo ""
    echo -e "  ${YELLOW}常用命令:${NC}"
    echo -e "    ${CYAN}anyclaude${NC}                    # 启动 TUI"
    echo -e "    ${CYAN}anyclaude --backend <name>${NC}   # 指定初始后端"
    echo -e "    ${CYAN}anyclaude status${NC}             # 查看运行状态"
    echo -e "    ${CYAN}anyclaude logs${NC}               # 查看日志（最近50行）"
    echo -e "    ${CYAN}anyclaude logs -f${NC}            # 实时追踪日志"
    echo -e "    ${CYAN}anyclaude stop${NC}               # 停止运行中的实例"
    echo -e "    ${CYAN}anyclaude uninstall${NC}          # 卸载（保留配置）"
    echo -e "    ${CYAN}anyclaude uninstall --purge${NC}  # 完全卸载（含配置）"
    echo ""
    echo -e "  ${YELLOW}Web 配置界面:${NC}"
    echo -e "    启动后浏览器访问 ${CYAN}${webui_url}${NC}"
    if [[ -n "$webui_user" ]]; then
        echo -e "    登录账号: ${CYAN}${webui_user}${NC}  （密码已在安装时设置）"
    fi
    echo -e "    可在线管理后端、切换 API 提供商（无需编辑文件）"
    echo ""
    if [[ -n "$shell_rc" ]]; then
        echo -e "  ${YELLOW}注意:${NC} 请先执行 ${CYAN}source ${shell_rc}${NC} 使 PATH 生效"
        echo ""
    fi
}

main "$@"
