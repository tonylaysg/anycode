# Custom Agent Profiles — Requirements Document

> **文档状态**：v0.2（已审计修订）
> **审计日期**：2026-04-24

---

## 审计发现与修订说明

本文档在 v0.1 初稿基础上经代码交叉验证后修订。以下是审计发现的关键问题（已在正文中修正，此处汇总）：

### 🔴 严重错误（已修正）

1. **认证/环境变量耦合远比初稿描述的复杂**
   初稿仅提及 `ANTHROPIC_BASE_URL` 和 `COPILOT_API_URL` 两个环境变量。实际代码中 `src/args/env_builder.rs` 注入了多组 Anthropic/Copilot 特定的环境变量（`ANTHROPIC_AUTH_TOKEN`、`ANTHROPIC_API_KEY`、`ANTHROPIC_CUSTOM_HEADERS`、`COPILOT_HOME`、Copilot 模式下的 `ANTHROPIC_BASE_URL` 二次注入等）。这些包含 Claude Code/Copilot 特有的产品级兼容性逻辑，**不能简单泛化为"自定义 env var 列表"**。参见修订后的 §2.1 `env_strategy` 字段和 §3.3 新增风险点。

2. **"认证端点转发到独立后端"会破坏代理的信任模型**
   初稿 FR-3.2 允许路由规则内联定义 `forward_base_url + forward_auth_token`。但这会把静态 JWT token 硬编码到配置文件，且代理会以透明方式转发敏感认证流，存在凭据泄漏风险。修订：路由规则的 `forward` 目标必须引用 `backends[]` 中定义的条目，不允许内联凭据。

3. **URL 路由模式的"精确匹配优先于通配符"与"按定义顺序匹配"冲突**
   §5.2 规定"精确匹配优先"又规定"按定义顺序"，自相矛盾。修订为**严格按定义顺序匹配**，首次命中即返回（与 §FR-2.3 一致）。

4. **`/teammate/*` 路径被硬编码在路由器中，路由规则无法覆盖**
   初稿未提及代理已有的 `/teammate/*` 嵌套路由（`router.rs:164`）、钩子端点（`/api/subagent-start` 等）和 `/health`。修订：新增 §2.2.4「系统保留路径」，明确 routing_rules 不能覆盖这些路径。

5. **"模型 API 请求 vs 其他请求"的区分方式没有定义，直接影响流水线**
   初稿 §8.3 作为开放问题提出但未决。实际上当前流水线的 7 阶段（thinking session、model rewrite、thinking compat）都**仅对模型 API 请求有意义**。修订：新增 §2.2.5「模型 API 端点标记」，通过 `is_model_api: true` 的 routing_rule 显式标记哪些 URL 走完整流水线。

### 🟡 重要遗漏（已补充）

6. **预设路径迁移漏过 `anyclaude → anycode`**
   `loader.rs:41` 有从 `~/.config/anyclaude/` 到 `~/.config/anycode/` 的自动迁移。初稿 §4.2 的迁移只处理了 TOML 结构迁移，没提路径迁移。修订：§4.2 补充说明迁移链路为 "旧版 anyclaude 路径 → 新版 anycode 路径 + 扁平格式 → 新版 profiles 格式"。

7. **`CliProfile` 的 `claude_settings: HashMap<String, bool>` 未在新模型中定位**
   `src/config/types.rs:17` 字段专用于 Claude Code 的 agent toggle 设置，通过 `with_settings()` 注入环境变量（`env_builder.rs:72`）。这是 Claude Code **产品特有的** feature，不适用于通用自定义智能体。修订：新增 §2.1.3「预设专属字段」保留此字段仅对 `is_preset=true` 的 profile 可见。

8. **IPC socket 路径冲突未考虑 stop/status 子命令**
   `anycode stop` 和 `anycode status` 依赖已知 IPC 套接字路径发现运行实例。多实例后，这些子命令需要 `<profile_id>` 参数。修订：§FR-4.1 补充 `anycode status [profile_id]`、`anycode stop [profile_id]`。

9. **WebUI 会话令牌（`ANTHROPIC_CUSTOM_HEADERS` + x-session-token）与"自定义 proxy_env_var"不兼容**
   `src/args/env_builder.rs:67` 通过 `ANTHROPIC_CUSTOM_HEADERS` 注入 session token。若自定义智能体的 `proxy_env_var` 不是 Anthropic 系列，CLI 无法理解 `ANTHROPIC_CUSTOM_HEADERS`。修订：§2.1.4 增加 `session_token_injection` 字段，定义如何将 token 注入到请求（如 header 预注入、URL 参数等）。

10. **`ObservabilityPlugin` 和调试日志假设单一请求流**
    当前 `ObservabilityHub` 全局单例，跨请求共享指标。多智能体场景下是否共享指标？修订：§3.3 增加架构风险点；Phase 2 增加决策"共享 observability，profile_id 作为 span 维度"。

### 🟢 次要问题（已修正）

11. **工作量估算偏低**：初稿未充分考虑测试和 CLI 子命令改造。修订：Phase 2 增加 1-2 天（URL 路由与现有流水线协同），Phase 3 增加 1 天（多实例 stop/status 子命令），Phase 5 增加 2 天（IPC 多实例集成测试）。新估算 **19-27 天**。

12. **"规则测试"前端功能过于模糊**：修订：§FR-5.2 明确为"给定 URL 路径，返回首次命中的规则及动作预览"，不需要真实发请求。

13. **Config 根字段 `default_profile` 的角色**：初稿引入但没说明与 `argv[0]` 自动检测的优先级。修订：§FR-4.1 明确：显式子命令参数 > `default_profile` > `argv[0]` 检测（保持向后兼容）。

---

## 概述

将 anycode 从"双 CLI 模式（Claude Code / Copilot）"扩展为**用户可自定义多智能体代理平台**。用户在 WebUI 中创建任意数量的自定义智能体配置文件，每个配置文件独立定义：启动命令、环境变量、多 URL 拦截转发规则、模型后端热切换列表。

---

## 1. 背景与动机

### 1.1 当前状态

| 概念 | 当前实现 |
|------|----------|
| 配置文件数量 | 2 个硬编码（`claude`、`copilot`） |
| 配置文件标识 | `CliMode` 枚举（`src/cli_mode.rs:3`） |
| 配置存储 | `Config.claude: CliProfile`、`Config.copilot: CliProfile`（`src/config/types.rs:33-36`） |
| 启动命令 | 硬编码：`claude` 或 `copilot`（`src/cli_mode.rs:26-30`） |
| 代理环境变量 | 硬编码：`ANTHROPIC_BASE_URL` 或 `COPILOT_API_URL`（`src/cli_mode.rs:34-38`） |
| URL 拦截 | 单一代理端口，所有请求通过 `proxy_handler` |
| 后端热切换 | 仅模型 API（`/v1/messages` 等），多后端热切换 |
| WebUI 管理 | 仅 claude/copilot 双 profile（`src/proxy/webui/api.rs:42-43`） |

### 1.2 局限性

1. **无法添加新智能体**：用户不能为其他 AI CLI 工具创建代理配置
2. **单一 URL 接管**：代理仅接管一个 `base_url`，某些智能体有多个后端 URL（认证、遥测、模型推理分离）
3. **无 URL 级别的路由控制**：所有请求统一转发，不能针对特定 URL 做阻断、Mock 或转发到不同后端
4. **预设不可修改**：Claude Code / Copilot 的启动命令、环境变量等核心参数不可定制

---

## 2. 功能需求

### 2.1 自定义智能体配置文件（Custom Agent Profile）

#### FR-1.1 配置文件 CRUD
用户在 WebUI 中自由创建、编辑、删除智能体配置文件。每个配置文件包含以下完整定义：

| 字段 | 类型 | 说明 | 必填 |
|------|------|------|------|
| `profile_id` | string | 唯一标识，英文小写（如 `my-custom-agent`） | 是 |
| `display_name` | string | 显示名称 | 是 |
| `description` | string | 用途描述 | 否 |
| `binary_name` | string | 要在 PTY 中启动的二进制文件名 | 是 |
| `binary_args` | string[] | 启动参数 | 否 |
| `proxy_env_var` | string | 注入代理 URL 的环境变量名 | 是 |
| `proxy_port` | number | 专用于此智能体的代理监听端口（0 = 自动分配） | 否 |
| `additional_env_vars` | map<string,string> | 额外的环境变量（自由定义） | 否 |
| `env_strategy` | enum | 认证环境变量策略（见 §2.1.2） | 是 |
| `session_token_injection` | object | 会话令牌如何注入请求（见 §2.1.4） | 否 |
| `is_preset` | bool | 是否为内置预设（预设可编辑但不可删除标识） | 否 |

#### FR-1.2 预设智能体
系统内置两个预设智能体，行为与当前一致，但用户可在 WebUI 中修改其 `base_url`、`binary_args`、`additional_env_vars`。预设的 `binary_name`、`proxy_env_var`、`env_strategy` **不可修改**（因为改动会破坏内置兼容性逻辑）：

| 预设 | binary_name | proxy_env_var | env_strategy | 说明 |
|------|-------------|---------------|--------------|------|
| `claude` | `claude` | `ANTHROPIC_BASE_URL` | `anthropic` | Claude Code |
| `copilot` | `copilot` | `COPILOT_API_URL` | `copilot` | GitHub Copilot CLI |

预设不可删除，其 `is_preset: true` 标识始终保持。

#### FR-1.2.1（新增）`env_strategy` 详解

此字段决定代理如何向被包装的 CLI 注入认证和会话相关环境变量。当前 `src/args/env_builder.rs` 中的逻辑不是通用的，必须作为"策略"枚举保留：

| 策略 | 行为 |
|------|------|
| `anthropic` | Claude Code 专用：按 `auth_type` 注入 `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY` 占位符 + 通过 `ANTHROPIC_CUSTOM_HEADERS` 注入 session token（见 `env_builder.rs:41-69`） |
| `copilot` | Copilot 专用：注入 `COPILOT_API_URL` + `ANTHROPIC_BASE_URL`（用于 sweagent-anthropic 代理） + `ANTHROPIC_API_KEY` 占位符 + `COPILOT_HOME` 隔离目录（见 `env_builder.rs:94-105`） |
| `generic` | 仅注入 `proxy_env_var=<proxy_url>` + `additional_env_vars`，不做任何产品特定处理 |

自定义智能体推荐使用 `generic`，除非它模拟 Claude Code 或 Copilot 的协议。

#### FR-1.3（新增）预设专属字段
以下字段仅对 `is_preset=true` 的 profile 可见和可编辑，对自定义智能体隐藏：

| 字段 | 用途 |
|------|------|
| `claude_settings: HashMap<String, bool>` | Claude Code 的 agent toggle 设置（通过 env vars 注入 CC） |
| `agents: AgentsConfig` | 子代理/队友路由配置（队友 shim 目前仅支持 Claude Code） |

#### FR-1.4（新增）`session_token_injection` 详解
对于不理解 `ANTHROPIC_CUSTOM_HEADERS` 的自定义智能体，定义 token 注入方式：

```toml
[profiles.my-agent.session_token_injection]
# 注入方式: env_header | url_query | disabled
mode = "env_header"
# env_header 模式：通过哪个 env var 注入 HTTP header
env_var_name = "MY_AGENT_CUSTOM_HEADERS"
# header 格式（env_var 的值填充格式）
header_format = "x-session-token:{token}"
```

若 `mode = "disabled"`，代理不为该 profile 启用 session token 校验（降低安全性，仅适用于 127.0.0.1 绑定场景）。

### 2.2 多 URL 拦截转发规则（URL Routing Rules）

#### FR-2.1 规则定义
每个智能体配置文件可定义多条 URL 拦截规则。每个规则匹配一类 URL pattern，并指定处理动作：

```toml
[[profiles.custom_agent.routing_rules]]
# URL 匹配模式（glob 风格，如 "/v1/messages*", "/auth/*", "/telemetry*"）
pattern = "/v1/messages*"
# 处理动作：forward | block | custom_response
action = "forward"
# 当 action = "forward" 时：转发到哪个后端
backend_name = "my-backend"
# 是否从 URL 中剥离匹配前缀再转发
strip_prefix = false

[[profiles.custom_agent.routing_rules]]
pattern = "/telemetry*"
action = "block"

[[profiles.custom_agent.routing_rules]]
pattern = "/.well-known/openid-configuration"
action = "custom_response"
# 自定义响应的状态码
status = 200
# 自定义响应的 Content-Type
content_type = "application/json"
# 自定义响应体
body = '{"issuer": "https://my-idp.example.com"}'
```

#### FR-2.2 处理动作详解

| 动作 | 行为 | 典型用途 |
|------|------|----------|
| `forward` | 将匹配请求转发到指定的后端 URL | 模型推理、API 调用 |
| `block` | 直接返回 403/404，不转发 | 阻断遥测、遥测上报 |
| `custom_response` | 返回用户定义的静态响应 | Mock 认证端点、假遥测 |

#### FR-2.3 规则匹配顺序
规则按定义顺序由上到下匹配，命中第一条规则后停止。未命中任何规则的请求使用默认行为（转发到当前活跃后端，并走完整的 7 阶段模型流水线）。

#### FR-2.4（新增）系统保留路径
以下路径由代理自身处理，routing_rules **不能覆盖**（即使 pattern 匹配也会被系统路径优先处理）：

| 路径 | 处理方 |
|------|--------|
| `/health` | 代理健康检查（`router.rs:146`） |
| `/api/subagent-start`、`/api/subagent-stop` | 子代理注册钩子 |
| `/api/teammate-start` | 队友注册钩子 |
| `/teammate/*` | 队友嵌套路由（`router.rs:164`） |

保存配置时，若 `routing_rules[].pattern` 与上述路径重叠，WebUI 返回错误并拒绝保存。

#### FR-2.5（新增）模型 API 端点标记
每个 `action = "forward"` 的路由规则可标记 `is_model_api: bool`（默认 `false`）。

- `is_model_api: true`：走完整 7 阶段流水线（extract → routing → thinking → transform → headers → forward → response），含模型重写、thinking 兼容、effort cap、子代理 AC 标记提取。
- `is_model_api: false`：走轻量级转发路径（仅 headers 重建 + forward），不做 JSON 解析和模型相关转换。

未定义任何 routing_rules 的 profile，默认所有请求按 `is_model_api: true` 处理（保持 Claude / Copilot 预设的当前行为）。

### 2.3 多后端热切换（保留并增强）

#### FR-3.1 模型 API 后端列表
每个智能体配置文件的 `backends` 列表与当前实现保持一致，用于模型 API 请求的热切换：

```toml
[[profiles.custom_agent.backends]]
name = "anthropic"
display_name = "Anthropic"
base_url = "https://api.anthropic.com"
auth_type = "passthrough"
# ... 模型映射、thinking 兼容等字段与当前一致
```

#### FR-3.2 非模型 URL 的独立后端
对于 `routing_rules` 中 `action = "forward"` 的规则，`backend_name` **必须**指向 `backends[]` 列表中的条目（不允许内联凭据，避免凭据泄漏和审计困难）：

```toml
# ✅ 正确：引用 backends[] 中的条目
[[profiles.custom_agent.backends]]
name = "my-auth-service"
display_name = "Auth Service"
base_url = "https://auth.my-provider.com"
auth_type = "bearer"
api_key = "..."  # 通过 WebUI 的 api_key_input 字段管理，自动脱敏

[[profiles.custom_agent.routing_rules]]
pattern = "/auth/*"
action = "forward"
backend_name = "my-auth-service"
# 可选：forward 前是否剥离 pattern 前缀
strip_prefix = false
```

后端列表中可包含多个非模型后端（如 `my-auth-service`、`my-telemetry-service`），每个独立管理凭据。

#### FR-3.3 后端的"用途"分类（新增）
`backends[]` 中的每个条目增加 `role` 字段，声明其用途：

| role | 语义 | 会被热切换影响 |
|------|------|----------------|
| `model`（默认） | 模型推理后端，参与 `defaults.active` 热切换 | 是 |
| `fixed_route` | 固定路由目标，仅被 routing_rules 引用，不参与热切换 | 否 |

`role = "model"` 的后端才会在 WebUI 的「热切换」按钮中出现。

#### FR-3.4 热切换范围
- 模型 API 后端的热切换（`/api/config/active`）**仅影响 `role = "model"` 的后端**
- `role = "fixed_route"` 的后端：改 URL 或 key 后需重启对应智能体实例生效（Phase 2 不做热更新）

### 2.4 智能体启动子命令

#### FR-4.1 命令行入口
新增 `anycode run <profile_id>` 子命令，启动指定的智能体配置文件：

```bash
# 启动预设 Claude Code
anycode run claude

# 启动自定义智能体
anycode run my-custom-agent

# 无参数默认行为（优先级）：
#   1. argv[0] 检测 → 二进制名含 "copilot" 则 run copilot（保持向后兼容）
#   2. 否则使用 config.default_profile
#   3. 都无则 run claude
anycode
```

配套的管理子命令也需加 profile 参数：

```bash
anycode status              # 列出所有运行中的智能体实例
anycode status <profile_id> # 查看指定实例的详细状态
anycode stop <profile_id>   # 停止指定实例
anycode stop --all          # 停止所有实例
anycode logs <profile_id>   # 跟踪指定实例的日志
```

PID 文件、IPC socket、日志文件的路径均由 `<profile_id>` 命名空间化：

| 资源 | 路径格式 |
|------|----------|
| PID 文件 | `~/.config/anycode/pids/{profile_id}.pid` |
| IPC socket | `$XDG_RUNTIME_DIR/anycode-{profile_id}.sock` |
| 日志文件 | `~/.config/anycode/logs/{profile_id}-debug.log` |

#### FR-4.2 智能体生命周期
每个运行的智能体实例：
- 拥有独立的代理端口
- 拥有独立的 PTY 会话
- 拥有独立的后端状态和热切换上下文
- 可同时运行多个不同智能体实例（不同端口不冲突）

### 2.5 WebUI 增强

#### FR-5.1 智能体管理页面
新增 Tab：「智能体管理」，功能包括：
- 查看所有已定义的智能体配置文件列表
- 创建新智能体（填写 FR-1.1 中所有字段）
- 编辑现有智能体
- 删除自定义智能体（预设不可删除）
- 启动/停止智能体实例

#### FR-5.2 URL 路由规则编辑器
在智能体编辑页面中：
- 可视化路由规则列表
- 添加/编辑/删除路由规则
- 规则排序（拖拽或上下移动）
- 规则测试（输入 URL，预览匹配结果）

#### FR-5.3 后端管理（与当前一致）
保留当前的后端 CRUD 和热切换功能，但上下文切换为选定智能体配置文件。

#### FR-5.4 多实例状态面板
显示所有运行中的智能体实例及其状态（端口、PID、运行时间、活跃后端）。

---

## 3. 架构影响评估

### 3.1 需要变更的核心模块

| 模块 | 文件 | 变更程度 | 说明 |
|------|------|----------|------|
| CLI 模式 | `src/cli_mode.rs` | **重构** | 从 enum 变为动态 ProfileId |
| 配置类型 | `src/config/types.rs` | **重构** | 从硬编码两个 profile 变为 `HashMap<String, CliProfile>` |
| 配置加载 | `src/config/loader.rs` | **中度** | 支持动态 profile 的序列化/反序列化 |
| PTY 管理 | `src/pty/manager.rs` | **中度** | 支持动态二进制名称和参数 |
| 代理路由器 | `src/proxy/router.rs` | **重度** | 增加 URL pattern 匹配和分级路由 |
| 代理流水线 | `src/proxy/pipeline/mod.rs` | **中度** | 增加非模型请求的 shortcut 路径 |
| WebUI API | `src/proxy/webui/api.rs` | **重度** | 从双 profile 变为多 profile CRUD |
| WebUI 前端 | `src/proxy/webui/index.html` | **重度** | 新增智能体管理 UI、路由规则编辑器 |
| 主入口 | `src/main.rs` | **中度** | 新增 `run` 子命令 |
| 后端状态 | `src/backend/state.rs` | **轻度** | 支持多实例隔离 |
| 运行时 | `src/ui/runtime.rs` | **中度** | 动态选择 profile 启动 |

### 3.2 向后兼容性

- **配置文件迁移**：旧版 `[claude]` / `[copilot]` 配置 TOML 格式自动迁移为新版 `[profiles.claude]` / `[profiles.copilot]` 格式
- **CLI 行为**：无参数的 `anycode` 命令行为不变（根据 argv[0] 自动检测模式）
- **API 兼容**：WebUI API 保留 `?profile=claude|copilot` 查询参数兼容旧版
- **环境变量**：`ANTHROPIC_BASE_URL` / `COPILOT_API_URL` 行为不变

### 3.3 风险点

1. **URL 路由性能**：每个请求需要遍历 routing_rules 进行 pattern 匹配。规则数量通常很少（< 20），使用 glob 匹配，性能影响可忽略。
2. **多实例资源**：同时运行多个智能体 = 多个代理端口 + 多个 PTY 进程 + 多个 IPC socket，需注意端口范围、文件描述符上限和内存。
3. **配置复杂度**：路由规则和自定义响应体增加了配置复杂度，需要在 WebUI 中做好引导和校验。
4. **`env_strategy` 的产品耦合不可消除**（新增）：Claude Code 和 Copilot CLI 的认证流程是产品特定的，必须作为策略枚举保留硬编码分支。新增自定义智能体若也需要复杂认证兼容，可能需要扩展 `env_strategy` 枚举（非向前兼容）。
5. **WebUI 跨实例共享 vs 隔离**（新增）：当前 WebUI 直接操作 `BackendState`（内存）+ `ConfigStore`（磁盘）。多实例下，WebUI 应作为独立守护进程，通过 IPC 与每个实例通信进行热切换，而不是直接共享内存状态。这是 Phase 4 的关键架构决策。
6. **Observability 跨实例混淆**（新增）：`ObservabilityHub` 当前为单进程全局。多实例场景下，`profile_id` 必须作为跨度维度，否则指标互相污染。
7. **队友 shim 仅支持 Claude Code**（新增）：`src/shim/tmux.rs` 硬编码了 `ANTHROPIC_BASE_URL` 和 `ANTHROPIC_CUSTOM_HEADERS` 格式。自定义智能体不支持队友 shim 功能，这是 `agents` 字段仅对预设可见的原因。

---

## 4. 配置格式设计

### 4.1 新版 `config.toml` 格式

```toml
[proxy]
bind_addr = "127.0.0.1:47190"
base_url = "http://127.0.0.1:47190"

[webui]
bind_addr = "127.0.0.1:47191"

# 默认启动的智能体（无参数时）
default_profile = "claude"

# ── 智能体配置文件列表 ──────────────────────────────────────

[profiles.claude]
display_name = "Claude Code"
description = "Anthropic 官方 Claude Code CLI"
binary_name = "claude"
proxy_env_var = "ANTHROPIC_BASE_URL"
is_preset = true

[profiles.claude.defaults]
active = "anthropic"
timeout_seconds = 30
# ... 其他 timeout 设置

[[profiles.claude.backends]]
name = "anthropic"
display_name = "Anthropic"
base_url = "https://api.anthropic.com"
auth_type = "passthrough"
# ... 模型映射等

[[profiles.claude.backends]]
name = "openrouter"
display_name = "OpenRouter"
base_url = "https://openrouter.ai/api/v1"
auth_type = "bearer"
api_key = "sk-or-..."

# Claude Code 通常不需要额外路由规则（仅模型 API）
# 但如果用户想阻断某些遥测端点，可以添加：
# [[profiles.claude.routing_rules]]
# pattern = "/telemetry*"
# action = "block"

[profiles.claude.agents]
subagent_backend = "anthropic"

# ── Copilot 预设 ────────────────────────────────────────────

[profiles.copilot]
display_name = "GitHub Copilot"
description = "GitHub Copilot CLI"
binary_name = "copilot"
proxy_env_var = "COPILOT_API_URL"
is_preset = true

[profiles.copilot.defaults]
active = "github-copilot"

[[profiles.copilot.backends]]
name = "github-copilot"
display_name = "GitHub Copilot"
base_url = "https://api.githubcopilot.com"
auth_type = "passthrough"

# ── 自定义智能体示例 ────────────────────────────────────────

[profiles.my-agent]
display_name = "My Custom AI Agent"
description = "第三方 AI CLI 工具，有独立的认证和遥测后端"
binary_name = "my-agent"
binary_args = ["--no-telemetry"]
proxy_env_var = "MY_AGENT_API_URL"
additional_env_vars = { MY_AGENT_LOG_LEVEL = "debug" }

[profiles.my-agent.defaults]
active = "my-agent-backend"

[[profiles.my-agent.backends]]
name = "my-agent-backend"
display_name = "My Agent API"
base_url = "https://api.my-agent.com"
auth_type = "bearer"
api_key = "sk-..."
model_sonnet = "my-agent-premium"

# URL 路由规则：接管多个 URL 并分流处理

# 规则 1: 模型推理 API → 转发到 my-agent-backend（支持热切换）
[[profiles.my-agent.routing_rules]]
pattern = "/v1/chat*"
action = "forward"
backend_name = "my-agent-backend"

# 规则 2: 认证端点 → 转发到独立的认证服务器
[[profiles.my-agent.routing_rules]]
pattern = "/auth/*"
action = "forward"
forward_base_url = "https://auth.my-provider.com"
forward_auth_type = "bearer"
forward_auth_token = "static-jwt-token"

# 规则 3: 遥测端点 → 阻断
[[profiles.my-agent.routing_rules]]
pattern = "/telemetry*"
action = "block"

# 规则 4: OIDC 发现 → 返回自定义响应（Mock）
[[profiles.my-agent.routing_rules]]
pattern = "/.well-known/openid-configuration"
action = "custom_response"
status = 200
content_type = "application/json"
body = '''
{
  "issuer": "https://my-idp.example.com",
  "authorization_endpoint": "https://my-idp.example.com/auth",
  "token_endpoint": "https://my-idp.example.com/token"
}
'''
```

### 4.2 旧版配置自动迁移

完整迁移链路（`Config::load()` 中依次执行）：

**步骤 1：路径迁移**（已有，保留）
- 若 `~/.config/anycode/config.toml` 不存在但 `~/.config/anyclaude/config.toml` 存在，自动复制到新路径（`loader.rs:52-61`）。

**步骤 2：v0（扁平）→ v1（双 profile）迁移**（已有，保留）
- 根级 `[defaults]` / `[[backends]]` → `[claude.defaults]` / `[[claude.backends]]`
- 由 `migrate_config_content()` 实现（`loader.rs:205`）。

**步骤 3：v1（双 profile）→ v2（多 profile）迁移**（新增）
- 旧版 `[claude]` / `[copilot]` → 新版 `[profiles.claude]` / `[profiles.copilot]`
- 为预设 profile 自动填入 `binary_name`、`proxy_env_var`、`env_strategy`、`is_preset = true`
- 触发条件：存在 `[claude]` 或 `[copilot]` 顶层 section，且不存在 `[profiles]` section

**兼容性**：上述三个步骤向前兼容，旧版配置首次启动新版 anycode 即自动升级。升级后写回磁盘，后续按新格式读取。

---

## 5. URL 路由匹配规格

### 5.1 匹配模式语法

采用简化的 glob 风格匹配：

| 模式 | 匹配 |
|------|------|
| `/v1/messages*` | 以 `/v1/messages` 开头的所有路径 |
| `/auth/*` | `/auth/` 下的所有路径 |
| `/telemetry` | 精确匹配 `/telemetry` |
| `/api/*/status` | `/api/xxx/status`（单段通配） |
| `*` | 所有剩余请求（默认规则） |

### 5.2 匹配优先级

1. **系统保留路径**（§FR-2.4）优先匹配并由代理自身处理，不进入 routing_rules
2. **routing_rules 严格按配置中定义的顺序匹配**，首次命中即返回（不做"精确优先"特殊处理，避免歧义）
3. 未命中任何规则 → 使用默认后端转发（按 `is_model_api: true` 走完整流水线）

### 5.3 规则冲突检测

保存配置时校验：
- 同一 pattern 不能在单个 profile 内定义多次
- `custom_response` 必须提供 `status` 和 `body`，且 `status` 在 [100, 599] 范围内
- `forward` 必须提供 `backend_name`，且该后端存在于 `backends[]` 中
- pattern 不能与系统保留路径重叠（`/health`、`/api/subagent-*`、`/api/teammate-start`、`/teammate/*`）

---

## 6. 实施阶段

### Phase 1: 配置层重构（4-6 天）
- `CliMode` enum 保留用于预设识别，但不再用作唯一标识 → 新增 `ProfileId(String)`
- `Config.profiles: HashMap<String, CliProfile>` 替代 `claude: CliProfile` / `copilot: CliProfile`
- 配置三级迁移链路（路径 → 扁平 → 多 profile）
- `CliProfile` 增加 `binary_name`、`proxy_env_var`、`env_strategy`、`routing_rules`、`additional_env_vars`、`session_token_injection` 字段
- `Backend` 增加 `role` 字段（`model` / `fixed_route`）
- `Config::validate_for(profile_id)` 替代 `validate_for(CliMode)`

### Phase 2: 代理层路由增强（4-5 天）
- `RouterEngine` 在 `proxy_handler` 入口增加 URL pattern 匹配层（路由规则引擎）
- 实现 `RuleAction::{Forward, Block, CustomResponse}` 三种动作
- 区分 `is_model_api: true`（走完整流水线） vs `is_model_api: false`（轻量级转发）
- 轻量级转发路径跳过 thinking session / model rewrite / thinking compat
- 系统保留路径优先于 routing_rules
- `ObservabilityHub` 增加 `profile_id` span 维度

### Phase 3: CLI 与 PTY 层适配（3-4 天）
- `anycode run <profile_id>` / `stop [profile_id]` / `status [profile_id]` / `logs [profile_id]` 子命令
- 动态二进制名称、参数、环境变量注入（`env_builder.rs` 增加 `EnvStrategy` 枚举分派）
- 多实例端口自动分配（以 `proxy.bind_addr` 为起点扫描可用端口）
- PID / IPC socket / 日志文件的路径命名空间化（按 `profile_id`）

### Phase 4: WebUI 增强（5-7 天）
- WebUI 架构决策：作为独立守护进程，通过 IPC 与每个实例通信（详见 §3.3 风险点 5）
- `/api/profiles` 升级为完整 CRUD（GET 列表、POST 创建、PUT 编辑、DELETE 删除；预设禁止 DELETE）
- 智能体管理页面（列表、创建、编辑、删除）
- URL 路由规则编辑器（列表、增删、拖拽排序、冲突校验）
- 规则匹配预览（给定 URL 路径，返回首次命中的规则及动作）
- 多实例状态面板（轮询 `status` IPC）
- `BackendDto` 增加 `role` 字段，UI 区分模型后端和固定路由后端

### Phase 5: 测试与文档（4-5 天）
- 集成测试：多 profile、多 URL 路由、热切换兼容
- IPC 多实例测试：两个 profile 并发运行不互相干扰
- 配置迁移测试：v0 / v1 / v2 三版格式均能正确读取
- 预设智能体兼容性验证：Claude Code 和 Copilot 的现有 PTY 流程无回归
- 用户文档更新（配置示例、迁移说明、常见场景）

**总估算工作量：19-27 天**

---

## 7. 未来扩展可能性

1. **路由规则中间件链**：支持在 forward 前后插入自定义处理逻辑（请求/响应修改）
2. **智能体市场**：用户可导出/导入智能体配置文件，社区共享
3. **Webhook 通知**：路由命中时触发 webhook（用于监控和告警）
4. **流量录制回放**：录制模型 API 请求/响应用于回归测试
5. **多实例负载均衡**：同一智能体运行多实例，请求分发到不同后端权重

---

## 8. 开放问题

1. ~~**多实例 IPC 隔离**~~（已在 §FR-4.1 解决：路径命名空间化 `anycode-{profile_id}.sock`）

2. ~~**WebUI 跨智能体管理**~~（已在 §3.3 风险点 5 解决：WebUI 独立守护进程架构）

3. ~~**模型 API 的 URL 识别**~~（已在 §FR-2.5 解决：显式 `is_model_api` 标记）

4. **自定义响应的模板变量**：`custom_response` 的 body 是否需要支持模板变量（如 `{{timestamp}}`、`{{request_id}}`、`{{request_header.X}}`）？建议 Phase 1 不支持，按需添加。

5. **路由规则的请求体转换**：是否允许路由规则修改请求 JSON（如字段重命名、补充字段）？这是常见的 API 适配需求，但大幅增加复杂度。建议 Phase 1 不支持；作为 §7 扩展项「路由规则中间件链」的一部分。

6. **跨 profile 的后端共享**：多个 profile 可能使用相同的后端（如都连 Anthropic）。是否允许定义"全局后端池"供多 profile 引用？当前设计每个 profile 独立 `backends[]`，有配置冗余。建议 Phase 1 保持独立，后续按需添加引用机制。

7. **WebUI 对运行中 profile 的实时影响**：当用户在 WebUI 中修改 profile 的 `routing_rules` 时，是否允许热重载？还是必须重启实例？建议 Phase 1 仅热重载 `backends[]`（保持当前行为），`routing_rules` 和 `binary_name` 等需重启实例。

8. **`proxy_env_var` 冲突**：两个 profile 使用相同的 `proxy_env_var`（如都用 `ANTHROPIC_BASE_URL`）时，能否同时运行？可以——因为每个实例有独立端口，被包装的 CLI 只看到自己的 env var。但 WebUI 应提示冲突风险（用户可能预期不同）。
