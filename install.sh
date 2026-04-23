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
_TMP_DIR=""   # global so EXIT trap can clean it up

# ── 颜色输出（使用 printf 避免 echo -e 在各终端行为不一致）───────────────────
if [ -t 1 ] && command -v tput &>/dev/null && tput colors &>/dev/null; then
    RED=$(tput setaf 1); GREEN=$(tput setaf 2); YELLOW=$(tput setaf 3)
    CYAN=$(tput setaf 6); NC=$(tput sgr0)
else
    RED=''; GREEN=''; YELLOW=''; CYAN=''; NC=''
fi

info()    { printf '%s[info]%s %s\n'    "$CYAN"   "$NC" "$*"; }
success() { printf '%s[ok]%s  %s\n'    "$GREEN"  "$NC" "$*"; }
warn()    { printf '%s[warn]%s %s\n'   "$YELLOW" "$NC" "$*"; }
error()   { printf '%s[err]%s  %s\n'   "$RED"    "$NC" "$*"; exit 1; }
header()  { printf '\n%s==> %s%s\n'    "$CYAN"   "$NC" "$*"; }

# ── 清理 ──────────────────────────────────────────────────────────────────────
cleanup() {
    if [[ -n "$_TMP_DIR" && -d "$_TMP_DIR" ]]; then
        rm -rf "$_TMP_DIR"
    fi
}
trap cleanup EXIT

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
        x86_64|amd64)  ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *)             error "不支持的架构: $arch" ;;
    esac
}

# ── 从源码编译安装 ────────────────────────────────────────────────────────────
install_from_source() {
    info "未找到预构建包，尝试从源码编译..."

    if ! command -v cargo &>/dev/null; then
        info "未检测到 Rust，正在安装..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
        # shellcheck source=/dev/null
        source "${HOME}/.cargo/env"
    fi

    _TMP_DIR="$(mktemp -d)"
    info "克隆仓库..."
    git clone --depth=1 "https://github.com/${REPO}.git" "$_TMP_DIR/anyclaude"

    info "编译中（release），请稍候..."
    (cd "$_TMP_DIR/anyclaude" && cargo build --release)

    install -Dm755 "$_TMP_DIR/anyclaude/target/release/${BINARY}" "${INSTALL_DIR}/${BINARY}"
}

# ── 更新配置文件中的 [webui] 段（Python 辅助，避免手动解析 TOML）──────────────
update_webui_config() {
    local bind_addr="$1" username="$2" password="$3"

    python3 - "$CONFIG_FILE" "$bind_addr" "$username" "$password" <<'PYEOF'
import sys, re

config_file  = sys.argv[1]
bind_addr    = sys.argv[2]
username     = sys.argv[3]
password     = sys.argv[4]

with open(config_file, 'r') as f:
    content = f.read()

# Build new [webui] block
lines = ['[webui]', f'bind_addr = "{bind_addr}"']
if username and password:
    lines.append(f'username  = "{username}"')
    lines.append(f'password  = "{password}"')
new_block = '\n'.join(lines) + '\n'

# Replace existing [webui] block (up to next section or EOF)
pattern = r'\[webui\][^\[]*'
if re.search(pattern, content, re.DOTALL):
    content = re.sub(pattern, new_block, content, flags=re.DOTALL)
else:
    # No [webui] section — append before first [[backends]] or at end
    if '[[backends]]' in content:
        content = content.replace('[[backends]]', new_block + '\n[[backends]]', 1)
    else:
        content = content.rstrip('\n') + '\n\n' + new_block

with open(config_file, 'w') as f:
    f.write(content)

print("ok")
PYEOF
}

# ── 主安装流程 ────────────────────────────────────────────────────────────────
main() {
    printf '\n'
    printf '%s AnyClaude 安装程序 %s\n' "$CYAN" "$NC"
    printf '%s ================= %s\n' "$CYAN" "$NC"
    printf '\n'

    detect_platform
    info "平台: ${OS}/${ARCH}"

    mkdir -p "${INSTALL_DIR}"

    # ── 下载/编译二进制 ────────────────────────────────────────────────────────
    local latest_tag="" installed_via="prebuilt"
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

    # ── WebUI 配置向导 ─────────────────────────────────────────────────────────
    local webui_user="" webui_pass="" webui_pass2="" webui_bind="" webui_url=""

    header "WebUI 配置向导（管理界面账号与访问权限）"

    printf '  请选择 WebUI 访问模式:\n'
    printf '    %s1%s  仅本机访问 (localhost，最安全)\n'          "$CYAN" "$NC"
    printf '    %s2%s  局域网访问 (内网所有设备可访问)\n'        "$CYAN" "$NC"
    printf '    %s3%s  公网访问   (任意 IP 可访问，需设密码)\n'  "$CYAN" "$NC"
    printf '  请输入选项 [1-3，默认 1]: '
    local access_mode
    read -r access_mode </dev/tty || true
    access_mode="${access_mode:-1}"

    case "$access_mode" in
        2|3) webui_bind="0.0.0.0:47191" ;;
        *)   webui_bind="127.0.0.1:47191" ;;
    esac

    # 访问模式 2/3 强制提示设密码；模式 1 询问
    local enable_auth="n"
    if [[ "$access_mode" == "2" || "$access_mode" == "3" ]]; then
        warn "局域网/公网访问强烈建议设置账号密码"
        enable_auth="y"
    else
        printf '  是否启用账号密码保护? [y/N，默认 N]: '
        read -r enable_auth </dev/tty || true
        enable_auth="${enable_auth:-n}"
    fi

    if [[ "$enable_auth" =~ ^[Yy]$ ]]; then
        printf '  管理员用户名 [默认 admin]: '
        read -r webui_user </dev/tty || true
        webui_user="${webui_user:-admin}"

        while true; do
            printf '  管理员密码 (输入不显示): '
            read -rs webui_pass </dev/tty || true
            printf '\n'
            if [[ -z "$webui_pass" ]]; then
                warn "密码不能为空，请重新输入"
                continue
            fi
            printf '  再次确认密码: '
            read -rs webui_pass2 </dev/tty || true
            printf '\n'
            if [[ "$webui_pass" == "$webui_pass2" ]]; then
                break
            fi
            warn "两次密码不一致，请重新输入"
        done
        success "账号密码已设置 (用户名: ${webui_user})"
    else
        warn "未设置账号密码，WebUI 将无需登录即可访问"
    fi

    # ── 写入/更新配置 ──────────────────────────────────────────────────────────
    if [[ ! -f "$CONFIG_FILE" ]]; then
        # 全新安装 — 生成完整配置
        mkdir -p "$CONFIG_DIR"
        local webui_block="[webui]
bind_addr = \"${webui_bind}\""
        if [[ -n "$webui_user" && -n "$webui_pass" ]]; then
            webui_block="${webui_block}
username  = \"${webui_user}\"
password  = \"${webui_pass}\""
        fi

        cat > "$CONFIG_FILE" <<EOF
[defaults]
active = "anthropic"

[proxy]
bind_addr = "127.0.0.1:47190"
base_url  = "http://127.0.0.1:47190"

${webui_block}

[[backends]]
name         = "anthropic"
display_name = "Anthropic (官方)"
base_url     = "https://api.anthropic.com"
auth_type    = "passthrough"
EOF
        success "已生成配置: ${CONFIG_FILE}"
    else
        # 配置已存在 — 仅更新 [webui] 段
        info "配置文件已存在，更新 [webui] 配置段..."
        if command -v python3 &>/dev/null; then
            if update_webui_config "$webui_bind" "$webui_user" "$webui_pass" | grep -q "ok"; then
                success "已更新 [webui] 配置: ${CONFIG_FILE}"
            else
                warn "自动更新失败，请手动编辑 [webui] 段"
            fi
        else
            warn "未找到 python3，请手动编辑 [webui] 段:"
            printf '    bind_addr = "%s"\n' "$webui_bind"
            [[ -n "$webui_user" ]] && printf '    username  = "%s"\n' "$webui_user"
            [[ -n "$webui_pass" ]] && printf '    password  = "%s"\n' "$webui_pass"
        fi
    fi

    # ── 计算 WebUI 访问 URL ────────────────────────────────────────────────────
    if [[ "$webui_bind" == "0.0.0.0:47191" ]]; then
        local lan_ip
        lan_ip=$(ip route get 1.1.1.1 2>/dev/null | awk '{print $7; exit}' \
            || hostname -I 2>/dev/null | awk '{print $1}' \
            || echo "YOUR_IP")
        webui_url="http://${lan_ip}:47191"
    else
        webui_url="http://127.0.0.1:47191"
    fi

    # ── 检查 PATH ─────────────────────────────────────────────────────────────
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

    # ── 安装完成摘要 ──────────────────────────────────────────────────────────
    local version
    version=$("${INSTALL_DIR}/${BINARY}" --version 2>/dev/null || echo "unknown")

    printf '\n'
    printf '%s=== 安装完成 ===%s\n' "$GREEN" "$NC"
    printf '\n'
    printf '  二进制:    %s%s/%s%s\n'  "$CYAN" "$INSTALL_DIR" "$BINARY" "$NC"
    printf '  版本:      %s%s%s\n'    "$CYAN" "$version"     "$NC"
    printf '  安装方式:  %s%s%s\n'    "$CYAN" "$installed_via" "$NC"
    printf '  配置文件:  %s%s%s\n'    "$CYAN" "$CONFIG_FILE"  "$NC"
    printf '\n'
    printf '  %s常用命令:%s\n' "$YELLOW" "$NC"
    printf '    %sanyclaude%s                   # 启动 TUI\n'                    "$CYAN" "$NC"
    printf '    %sanyclaude --backend <name>%s  # 指定初始后端\n'               "$CYAN" "$NC"
    printf '    %sanyclaude status%s            # 查看运行状态\n'               "$CYAN" "$NC"
    printf '    %sanyclaude logs%s              # 查看日志 (最近50行)\n'        "$CYAN" "$NC"
    printf '    %sanyclaude logs -f%s           # 实时追踪日志\n'               "$CYAN" "$NC"
    printf '    %sanyclaude stop%s              # 停止运行中的实例\n'            "$CYAN" "$NC"
    printf '    %sanyclaude uninstall%s         # 卸载 (保留配置)\n'            "$CYAN" "$NC"
    printf '    %sanyclaude uninstall --purge%s # 完全卸载 (含配置)\n'          "$CYAN" "$NC"
    printf '\n'
    printf '  %sWeb 配置界面:%s\n' "$YELLOW" "$NC"
    printf '    启动后访问: %s%s%s\n' "$CYAN" "$webui_url" "$NC"
    if [[ -n "$webui_user" ]]; then
        printf '    登录账号:   %s%s%s  (密码已在安装时设置)\n' "$CYAN" "$webui_user" "$NC"
    else
        printf '    无需登录即可访问\n'
    fi
    printf '    可在线管理后端、切换 API 提供商，无需编辑配置文件\n'
    printf '\n'
    if [[ -n "$shell_rc" ]]; then
        printf '  %s注意:%s 请先执行 %ssource %s%s 使 PATH 生效\n' \
            "$YELLOW" "$NC" "$CYAN" "$shell_rc" "$NC"
        printf '\n'
    fi
}

main "$@"
