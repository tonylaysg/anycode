# anycode

[English](#english) | [中文](#中文)

---

## English

TUI wrapper for Claude Code with hot-swappable API backend support.

**Goal:** Switch between API providers instantly — configure once, swap with `Ctrl+B`, no restarts or config edits required.

> Only Anthropic API-compatible backends are supported.

### Features

- **Hot-Swap Backends** — Switch providers mid-session with `Ctrl+B`
- **Agent Routing** — Route subagents and teammates to separate backends
- **Thinking Block Filtering** — Automatically filters previous backend's thinking blocks on switch
- **Adaptive Thinking Conversion** — Converts adaptive thinking for non-Anthropic backends (`thinking_compat`)
- **Model Mapping** — Remap model names per backend
- **Transparent Proxy** — Routes all Claude Code API requests through active backend
- **WebUI** — Browser-based backend management, no config editing needed
- **Debug Logging** — Configurable request/response logging

### Installation

**One-line install (recommended):**

```bash
curl -fsSL https://raw.githubusercontent.com/tonylaysg/anycode/main/install.sh | bash
```

The installer will:
- Auto-detect your platform (Linux/macOS, x86_64/aarch64)
- Download the prebuilt binary or build from source
- Walk you through WebUI access and password setup
- Add the binary to your PATH

**Build from source:**

```bash
# Requires Rust (https://rustup.rs)
cargo install --path .
```

### Quick Start

1. Run `anycode` — this starts the TUI with Claude Code embedded
2. Configure backends via WebUI at `http://127.0.0.1:47191`
3. Press `Ctrl+B` to switch backends at any time

### Commands

```
anycode                          Start TUI (default backend)
anycode --backend <name>        Start with specific backend
anycode status                  Show running instance status
anycode logs [-n 50] [-f]      View or follow debug logs
anycode stop                    Stop running instance
anycode webui                   Start WebUI configuration server
anycode webui --daemon          Start WebUI in background
anycode bind [local|lan|public] Change WebUI access mode
anycode passwd                  Set WebUI login credentials
anycode uninstall               Uninstall (keeps config)
anycode uninstall --purge       Uninstall and remove all config
```

### Hotkeys

| Key | Action |
|-----|--------|
| `Ctrl+B` | Backend switcher popup |
| `Ctrl+H` | Backend switch history |
| `Ctrl+E` | Settings dialog |
| `Ctrl+R` | Restart Claude Code (preserves session) |
| `Ctrl+Q` | Quit |
| `1-9` | Quick-select backend (in switcher) |

### Configuration

Config file: `~/.config/anycode/config.toml`

**Minimal:**

```toml
[defaults]
active = "anthropic"

[[backends]]
name = "anthropic"
display_name = "Anthropic"
base_url = "https://api.anthropic.com"
auth_type = "passthrough"

[[backends]]
name = "alternative"
display_name = "Alternative Provider"
base_url = "https://your-provider.com/api"
auth_type = "bearer"
api_key = "your-api-key"
```

**Full example:**

```toml
[defaults]
active = "anthropic"
timeout_seconds = 300
connect_timeout_seconds = 5
idle_timeout_seconds = 60

[proxy]
bind_addr = "127.0.0.1:47190"
base_url  = "http://127.0.0.1:47190"

[webui]
bind_addr = "127.0.0.1:47191"
# username = "admin"
# password = "yourpassword"

[terminal]
scrollback_lines = 10000

[debug_logging]
level       = "verbose"   # off | basic | verbose | full
destination = "file"      # stderr | file | both
file_path   = "~/.config/anycode/logs/debug.log"
format      = "console"   # console | json

[[backends]]
name = "anthropic"
display_name = "Anthropic"
base_url = "https://api.anthropic.com"
auth_type = "passthrough"

[[backends]]
name = "alternative"
display_name = "Alternative Provider"
base_url = "https://your-provider.com/api"
auth_type = "bearer"
api_key = "your-api-key"
thinking_compat = true          # Convert adaptive->enabled thinking
thinking_budget_tokens = 10000
model_opus   = "custom-opus"
model_sonnet = "custom-sonnet"
model_haiku  = "custom-haiku"

[agents]
teammate_backend = "alternative"
subagent_backend = "alternative"
```

**Authentication types:**

| Type | Header sent | Use case |
|------|-------------|----------|
| `passthrough` | Forwards original headers | Anthropic OAuth, custom auth |
| `api_key` | `x-api-key: <value>` | Anthropic API key |
| `bearer` | `Authorization: Bearer <value>` | Most third-party providers |

### Model Mapping

Map Anthropic model families to provider-specific names:

```toml
[[backends]]
name         = "my-provider"
base_url     = "https://api.example.com"
auth_type    = "bearer"
api_key      = "key"
model_opus   = "provider-large"   # claude-opus-*  -> provider-large
model_sonnet = "provider-medium"  # claude-sonnet-* -> provider-medium
model_haiku  = "provider-small"   # claude-haiku-*  -> provider-small
```

Responses are reverse-mapped so Claude Code always sees consistent model names.

### Agent Routing

```toml
[agents]
teammate_backend = "alternative"
subagent_backend = "alternative"
```

- Main agent uses the active backend (switchable via `Ctrl+B`)
- Subagents are pinned per-session via CC hooks
- Teammates are routed via tmux shim

### Thinking Block Handling

**Filtering (always active):** When switching backends mid-session, thinking blocks from the old backend are automatically removed. Each provider's thinking blocks contain cryptographic signatures that other providers reject.

**Conversion (`thinking_compat`):** Claude Code uses adaptive thinking, which non-Anthropic backends don't support. Set `thinking_compat = true` to convert it to the explicit `enabled` format:

```toml
[[backends]]
name = "alternative"
base_url = "https://your-provider.com/api"
auth_type = "bearer"
api_key = "key"
thinking_compat = true
thinking_budget_tokens = 10000
```

---

## 中文

Claude Code 的 TUI 包装器，支持一键热切换 API 后端。

**目标：** 随时切换 API 服务商，只需配置一次，按 `Ctrl+B` 切换，无需重启或修改配置文件。

> 仅支持兼容 Anthropic API 格式的后端。

### 功能特性

- **热切换后端** — 按 `Ctrl+B` 即时切换服务商，无需重启
- **Agent 路由** — 将子 Agent 和 Teammate 分配到不同后端
- **思考块过滤** — 切换后端时自动过滤旧后端的 thinking block
- **Adaptive Thinking 转换** — 非 Anthropic 后端自动转换 thinking 格式（`thinking_compat`）
- **模型名映射** — 每个后端可独立配置模型名映射
- **透明代理** — 所有 API 请求自动路由到当前后端
- **WebUI 管理界面** — 浏览器在线管理后端配置，无需手动编辑文件
- **调试日志** — 可配置详细程度的请求/响应日志

### 安装

**一键安装（推荐）：**

```bash
curl -fsSL https://raw.githubusercontent.com/tonylaysg/anycode/main/install.sh | bash
```

安装程序会自动：
- 检测系统平台（Linux/macOS，x86_64/aarch64）
- 下载预编译二进制包（若不可用则从源码编译）
- 引导配置 WebUI 访问权限及账号密码
- 自动将二进制文件添加到 PATH

**从源码编译：**

```bash
# 需要 Rust (https://rustup.rs)
cargo install --path .
```

### 快速上手

1. 运行 `anycode` — 启动 TUI，内嵌 Claude Code
2. 访问 `http://127.0.0.1:47191` 通过 WebUI 配置后端
3. 随时按 `Ctrl+B` 切换后端

### 命令

```
anycode                          启动 TUI（使用默认后端）
anycode --backend <名称>        指定初始后端启动
anycode status                  查看运行状态
anycode logs [-n 50] [-f]      查看 / 实时追踪日志
anycode stop                    停止运行中的实例
anycode webui                   启动 WebUI 配置服务
anycode webui --daemon          后台模式启动 WebUI
anycode bind [local|lan|public] 更改 WebUI 访问模式
anycode passwd                  设置 WebUI 登录密码
anycode uninstall               卸载（保留配置文件）
anycode uninstall --purge       完全卸载（含配置文件）
```

### 快捷键

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+B` | 打开后端切换弹窗 |
| `Ctrl+H` | 查看后端切换历史 |
| `Ctrl+E` | 打开设置对话框 |
| `Ctrl+R` | 重启 Claude Code（保留会话） |
| `Ctrl+Q` | 退出 |
| `1-9` | 在切换弹窗中快速选择后端 |

### 配置

配置文件位置：`~/.config/anycode/config.toml`

**最简配置：**

```toml
[defaults]
active = "anthropic"

[[backends]]
name         = "anthropic"
display_name = "Anthropic（官方）"
base_url     = "https://api.anthropic.com"
auth_type    = "passthrough"

[[backends]]
name         = "alternative"
display_name = "其他服务商"
base_url     = "https://your-provider.com/api"
auth_type    = "bearer"
api_key      = "your-api-key"
```

**完整配置示例：**

```toml
[defaults]
active                   = "anthropic"
timeout_seconds          = 300
connect_timeout_seconds  = 5
idle_timeout_seconds     = 60

[proxy]
bind_addr = "127.0.0.1:47190"
base_url  = "http://127.0.0.1:47190"

[webui]
bind_addr = "127.0.0.1:47191"
# username = "admin"
# password = "yourpassword"

[terminal]
scrollback_lines = 10000

[debug_logging]
level       = "verbose"   # off | basic | verbose | full
destination = "file"      # stderr | file | both
file_path   = "~/.config/anycode/logs/debug.log"
format      = "console"   # console | json

[[backends]]
name         = "anthropic"
display_name = "Anthropic（官方）"
base_url     = "https://api.anthropic.com"
auth_type    = "passthrough"

[[backends]]
name                   = "alternative"
display_name           = "其他服务商"
base_url               = "https://your-provider.com/api"
auth_type              = "bearer"
api_key                = "your-api-key"
thinking_compat        = true   # 转换 adaptive thinking 格式
thinking_budget_tokens = 10000
model_opus             = "custom-opus"
model_sonnet           = "custom-sonnet"
model_haiku            = "custom-haiku"

[agents]
teammate_backend = "alternative"
subagent_backend = "alternative"
```

**认证类型说明：**

| 类型 | 发送的 Header | 适用场景 |
|------|--------------|----------|
| `passthrough` | 透传原始 Header | Anthropic OAuth、自定义认证 |
| `api_key` | `x-api-key: <value>` | Anthropic API Key |
| `bearer` | `Authorization: Bearer <value>` | 大多数第三方服务商 |

### 模型名映射

将 Anthropic 模型族名映射到服务商自定义名称：

```toml
[[backends]]
name         = "my-provider"
base_url     = "https://api.example.com"
auth_type    = "bearer"
api_key      = "key"
model_opus   = "provider-large"   # claude-opus-*   -> provider-large
model_sonnet = "provider-medium"  # claude-sonnet-* -> provider-medium
model_haiku  = "provider-small"   # claude-haiku-*  -> provider-small
```

响应中会自动反向映射，Claude Code 始终看到一致的模型名。

### Agent 路由

```toml
[agents]
teammate_backend = "alternative"
subagent_backend = "alternative"
```

- 主 Agent 使用当前活跃后端（可通过 `Ctrl+B` 切换）
- 子 Agent 通过 CC hooks 按会话绑定到指定后端
- Teammate Agent 通过 tmux shim 路由

### Thinking Block 处理

**过滤（始终启用）：** 切换后端时，上一个后端产生的 thinking block 会被自动过滤。每个服务商的 thinking block 包含加密签名，换到其他服务商后会导致 400 错误。

**格式转换（`thinking_compat`）：** Claude Code 使用 adaptive thinking 格式，非 Anthropic 后端不支持。设置 `thinking_compat = true` 可自动转换为 `enabled` 格式：

```toml
[[backends]]
name                   = "alternative"
base_url               = "https://your-provider.com/api"
auth_type              = "bearer"
api_key                = "key"
thinking_compat        = true
thinking_budget_tokens = 10000
```

---

## License

Apache 2.0
