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

    # ── 写入默认配置（仅在配置文件不存在时）────────────────────────────────
    if [[ ! -f "$CONFIG_FILE" ]]; then
        mkdir -p "$CONFIG_DIR"
        cat > "$CONFIG_FILE" <<'EOF'
[defaults]
active = "anthropic"

[proxy]
bind_addr = "127.0.0.1:47190"
base_url  = "http://127.0.0.1:47190"

[[backends]]
name         = "anthropic"
display_name = "Anthropic (官方)"
base_url     = "https://api.anthropic.com"
auth_type    = "passthrough"
EOF
        success "已生成默认配置: ${CONFIG_FILE}"
    else
        info "配置文件已存在，跳过: ${CONFIG_FILE}"
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
    echo -e "  ${YELLOW}启动方式:${NC}"
    echo -e "    ${CYAN}anyclaude${NC}                  # 启动（透传 Anthropic 官方认证）"
    echo -e "    ${CYAN}anyclaude --backend <name>${NC}  # 指定初始后端"
    echo ""
    echo -e "  ${YELLOW}Web 配置界面:${NC}"
    echo -e "    启动后浏览器访问 ${CYAN}http://127.0.0.1:47190/ui/${NC}"
    echo -e "    可在线管理后端、切换 API 提供商（无需编辑文件）"
    echo ""
    if [[ -n "$shell_rc" ]]; then
        echo -e "  ${YELLOW}注意:${NC} 请先执行 ${CYAN}source ${shell_rc}${NC} 使 PATH 生效"
        echo ""
    fi
}

main "$@"
