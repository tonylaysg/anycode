# Changelog

All notable changes to anycode are documented in this file.

## [0.6.3] - 2026-04-24

### Fixes

- **tui**: defensive hardening around terminal setup to prevent silent crashes on exotic terminal emulators. The panic hook is now installed BEFORE any alt-screen / raw-mode side effects, so if `Terminal::new` panics (e.g. size-query failure) the terminal is still restored. Startup now probes terminal size up-front and returns a readable error (`terminal reports zero size …`) instead of silently panicking inside ratatui on first draw — this was the most likely root cause behind "anycopilot 启动后立即退出，终端回到 shell 提示符" in some shells. Errors are also explicitly printed to stderr after the terminal is restored, so diagnostic output is never eaten by a lingering alternate screen.

## [0.6.2] - 2026-04-26

### Features

- **proxy**: new per-backend `strip_request_prefix` config field for backends that expose the API without the `/v1` segment entirely (e.g. `POST /chat/completions`, `GET /models`). Copilot CLI always emits `/v1/...` — setting `strip_request_prefix = "/v1"` removes the matching leading segment before concatenation. Combined with the smart `/vN` dedup from 0.6.1 this covers all four common URL shapes:
  * `base=api.anthropic.com`              → `/v1/messages` kept
  * `base=api.openai.com/v1`               → `/v1/messages` deduped to `/v1/messages`
  * `base=api.foo.com`, strip=`/v1`        → `/v1/chat/completions` → `/chat/completions`
  * `base=api.foo.com/openai`, strip=`/v1` → `/v1/messages` → `/openai/messages`
- **webui**: expose `strip_request_prefix` as an inline input on the backend edit modal (label `请求路径前缀剥离`, placeholder `/v1`). Empty = automatic dedup only.

## [0.6.1] - 2026-04-26

### Bug Fixes

- **copilot**: fix `anycopilot` regression where the Copilot CLI child received `COPILOT_PROVIDER_API_KEY=` (empty) and refused to start, complaining that a Copilot token was required. `src/ui/runtime.rs` had a stale pre-BYOK branch that zeroed the session token for Copilot mode; under BYOK that token is consumed as both the proxy auth and the CLI's provider key, so an empty value fails authentication at launch. The token is now an unconditional per-session UUID.
- **proxy**: smarter upstream URL construction. Copilot CLI always emits paths prefixed with `/v1` (e.g. `/v1/messages`, `/v1/chat/completions`, `/v1/responses`), but many backends already embed `/v1` in their base URL (OpenAI, DeepSeek, `https://api.openai.com/v1`). Naïve concatenation produced `/v1/v1/...` and returned 404. The new `build_upstream_url` helper dedupes identical trailing/leading `/vN` segments while leaving non-matching versions (`/v2` vs `/v1`) and lookalikes (`/v1beta`, `/v10`) alone. No configuration required — works with existing backend entries.

### Features

- **copilot**: expanded `wire_api` to cover the three wire formats Copilot CLI speaks natively. New values: `anthropic` (default), `openai` (OpenAI Chat Completions), `openai-responses` (OpenAI `/v1/responses`, needed for GPT-5-class models), `azure`, `azure-responses`. Anycode now omits `COPILOT_PROVIDER_WIRE_API` entirely when `wire_api = anthropic`, silencing the Copilot CLI warning `Provider 'wireApi' option is ignored for Anthropic provider`. Exposed in the webui as a 5-option dropdown with inline help text.

## [0.6.0] - 2026-04-25

### Features

- **copilot**: first-class BYOK (bring-your-own-key) support for the GitHub Copilot CLI. `anycopilot` now injects `COPILOT_OFFLINE=true` + `COPILOT_PROVIDER_*` env vars so the upstream CLI skips the GitHub OAuth device flow entirely and speaks directly to anycode's proxy using the session-local Bearer token. No GitHub account or `gh auth login` required.
- **proxy**: new `GET /v1/models` (and `/models`) endpoint. Aggregates the active backend's model list with a 30-minute in-memory cache keyed by `(backend.name, base_url)`; auto-probes `/v1/models` then `/models` when no explicit path is configured. Lets Copilot CLI's `/model` slash command and Claude Code's model picker discover all backend models dynamically. Configurable per backend via the new `models_path` field.
- **proxy**: OpenAI-wire pass-through. `POST /v1/chat/completions` now flows through the same routing / model-rewrite / backend-forward pipeline as `/v1/messages`. The reverse-model-name SSE rewriter was extended to handle OpenAI `chat.completion.chunk` events so the client never sees the backend's raw model id. Enables pairing anycopilot with any OpenAI-compatible backend (DeepSeek, OpenRouter, LiteLLM, vLLM) via a single `wire_api = "openai"` knob.
- **config**: new `Backend.wire_api` field (`"anthropic"` | `"openai"`, default Anthropic). Drives `COPILOT_PROVIDER_TYPE` at Copilot CLI launch. Exposed in the webui as a dropdown.
- **proxy**: `Authorization: Bearer <session_token>` is now accepted by the middleware alongside the existing `x-session-token` header (required because Copilot CLI's `COPILOT_PROVIDER_API_KEY` is sent as Bearer).

## [0.5.5] - 2026-04-24

### Bug Fixes

- **copilot**: `anycopilot` failed at startup with `error: unknown option '--session-id'` from the GitHub Copilot CLI. anycode was unconditionally injecting Claude-Code-specific flags (`--session-id <id>`, `--teammate-mode tmux`, `--settings <hooks-json>`) regardless of `CliMode`. Copilot CLI rejects unknown flags and exits immediately. Added a `copilot_mode` flag on `ArgAssembler` that:
    - skips `--session-id` injection on initial spawn (Copilot auto-assigns its own session ID);
    - emits `--resume=<id>` (the `=` form Copilot requires) for resume/restart;
    - no-ops `with_teammate_mode` and `with_subagent_hooks` (both are Claude-only);
    - strips any stray `--session-id <id>` that leaks through user passthrough.

  Wired through `build_spawn_params`, `build_restart_params`, and both `runtime.rs` assembler call sites. Added `CliMode::is_copilot()` helper. Regression tests in `tests/copilot_mode_args.rs` pin the contract (5 cases) and verify the Claude codepath is unchanged.

## [0.5.4] - 2026-04-24

### Bug Fixes

- **pty**: `anycopilot` / `anycode` panicked at startup with `index out of bounds: the len is 0 but the index is 18446744073709551615` when the host terminal reported a 0×0 initial size. `alacritty_terminal::Term::new` underflows its grid storage on zero dimensions. Reproducible under fresh `pty.fork()` children, certain SSH clients, and nested tmux sessions where the WINCH that carries the real size has not yet been delivered by the time we construct the emulator. Added a 1×1 floor in all four PTY init/resize sites (`AlacrittyEmulator::new`, `AlacrittyEmulator::set_size`, `PtyManager::{new,run_command}`, `PtySession::new`) and skip bogus 0×0 SIGWINCH events in `ResizeWatcher`. Added regression tests in `tests/pty_emulator_zero_size.rs`.

## [0.5.3] - 2026-04-24

### Features

- **webui**: copy/clone backend across profiles — new `POST /api/config/backends/{name}/copy` endpoint preserves the full backend definition including `api_key` (which is never exposed to the browser). The UI adds a 复制 button per backend row with a modal for choosing target profile and new name; supports both same-profile cloning and cross-profile copy (claude ↔ copilot)

### Bug Fixes

- **webui**: saving the copilot profile failed with `PUT /api/config?profile=copilot 400 Bad Request` because `Defaults::default()` sets `active = "claude"` while user-added copilot backends have different names. `put_config` now self-heals by adopting the first backend as active when the submitted `active` doesn't exist
- **webui**: same self-heal applied to `post_copy_backend` — copying the first backend into an empty copilot profile would previously fail validation for the same reason

## [0.5.2] - 2026-04-24

### Bug Fixes

- **proxy**: strip entire thinking history when filter removes any block — prevents recurring `400 content[].thinking must be passed back to the API` on Anthropic-compatible backends (DeepSeek, etc.) after a mid-session backend switch
- **proxy**: don't apply request-total timeout to streaming retries (was causing spurious timeouts on long SSE streams)
- **config**: auto-detect `thinking_compat` for non-Anthropic backends when unset (was relying on explicit config even when the docstring implied auto)
- **pty**: switch alacritty screen handle to `Arc<Mutex>` for thread-safety (fixes pre-existing build error in tests)
- **uninstall**: clean up the `anycopilot` symlink and any legacy `anyclaude` binary alongside the main binary (prevents dangling symlinks after uninstall)

### CI / Release

- **release.yml**: fix binary name after the `anyclaude` → `anycode` rename (the `cp` step was still referencing the old name, which would have broken every tag-triggered release)
- **release.yml**: switch Linux targets to `*-unknown-linux-musl` (fully-static) so prebuilt binaries run on any glibc version (Ubuntu 22.04 / Debian 12 / RHEL 9 and older). Previous glibc build on `ubuntu-latest` required GLIBC 2.38+ and failed on most user systems.
- **install.sh**: point at the `tonylaysg/anycode` repo so prebuilt assets are downloaded directly instead of silently falling back to source compilation

### Documentation

- Document Copilot CLI mode: `anycopilot` symlink, profile selection via `argv[0]`, separate `[claude]` / `[copilot]` config sections
- Document previously undocumented commands: `reset`, `webui --stop`

## [0.5.1] - 2026-04-23

### Bug Fixes

- Fix passthrough backends injecting fake `ANTHROPIC_API_KEY=anycode-proxy` when no credentials present — previously prevented Claude Code from showing its login screen on fresh installs

### Features

- Add `anycode reset` command to clear stale Claude Code auth state from previous sessions
- Install script now detects existing installations and performs update-only (skips config wizard, preserves existing config)

## [0.5.0] - 2026-03-13

### Bug Fixes

- Update stale shim tests to match current injection logic
- Skip AC marker parsing when subagent registry is empty
- Detect teammate spawns by --agent-id flag instead of binary path
- Always set CLAUDE_CODE_SUBAGENT_MODEL env var
- Remove dead code and add subagent backend validation in UI
- Patch dependency vulnerabilities and optimize release profile
- Inject shim PATH into initial spawn env
- Disable session token check for teammate pipeline
- Inject session token into tmux shim for teammate auth
- Update spawn env with actual proxy port after try_bind
- Resolve 3 critical pipeline bugs found in review
- Use append mode instead of truncate for log files
- Ctrl+R now resumes current session via --resume
- --continue now uses --resume for existing sessions
- Replace fragile sed with bash regex in tmux shim

### Documentation

- Update subagent backend spec and add session affinity spec
- Add pluggable feature architecture design and implementation plan
- Add subagent backend selection specification
- Add GPU terminal architecture specification
- Update README to match current architecture
- Add pipeline unification analysis from 7 review rounds
- Clarify thinking middleware skipping for teammates
- Update README with reverse model mapping and text selection

### Features

- Unified agent routing with teammate pipeline
- Subagent registry with session affinity via AC markers
- Subagent session affinity via CC hooks
- **ui:** Subagent backend selection in backend popup
- **proxy:** Subagent backend runtime state and routing
- **config:** Add subagent_backend to AgentTeamsConfig
- Add session token handshake via ANTHROPIC_CUSTOM_HEADERS
- Stamp dev builds with git hash, add --version flag
- Add unified 7-stage linear pipeline behind feature flag
- Cleanup old per-session log files on startup
- Use per-session log file names to isolate instance logs
- Add Ctrl+R to restart Claude Code (continues session)

### Refactor

- Harden AC marker parsing against false positives
- Use agent_id instead of session_id for subagent routing
- Remove CLAUDE_CODE_SUBAGENT_MODEL in favor of AC marker routing
- Rename AgentTeamsConfig to AgentsConfig
- Add SAFETY comments, reduce cloning, deduplicate formatting
- Remove legacy pipeline and unified-pipeline feature flag

### Testing

- Add corner-case tests for Ctrl+R restart
- Add tests for Ctrl+R restart feature

## [0.4.0] - 2026-02-15

### Bug Fixes

- Generate realistic SSE format in MockResponse::sse()
- Align body content with header/footer by adding 1-column side padding
- Forward Ctrl+V to CC instead of intercepting clipboard images
- Respect debug logging config in tmux shim
- Propagate routing decision to upstream forwarding
- Detect Shift+Enter via macOS CGEvent for newline insertion

### Chore

- Release v0.4.0

### Documentation

- Add design document for reverse model mapping
- Add commit rules to AGENTS.md
- Add argument handling redesign documentation
- Update shim doc comment to reference args pipeline
- Update README with agent teams, model mapping, and CLI options
- Design thinking pipeline isolation for multi-agent sessions
- Update design doc with Phase 1b completion and model map
- Add Phase 1b/1c design for smart and synthetic tmux shims
- Update routing design with empirical tmux findings
- Revise routing design — generic routing layer + simple config
- Add per-agent backend routing design for Agent Teams

### Features

- Add double-click word selection
- Wire reverse model mapping into upstream proxy pipeline
- Add model_rewrite module for reverse model mapping
- Add ChunkRewriter to ObservedStream for response transformation
- Add args pipeline module for declarative argument handling
- Separate main and teammate pipelines via axum nest
- Add per-backend model family mapping (model_opus/sonnet/haiku)
- Smart tmux shim injects ANTHROPIC_BASE_URL for teammates
- Wire shims and --teammate-mode tmux into PTY spawn
- Add PATH shims for teammate routing (claude + tmux)
- Add proxy routing layer for path-based backend selection
- Add AgentTeamsConfig with teammate_backend validation

### Refactor

- Remove dead code replaced by args pipeline
- Integrate args pipeline into runtime
- Remove dead image paste code
- Remove redundant claude PATH shim
- Remove debug logging env var overrides
- Wire new pipeline architecture into server and IPC
- Extract ThinkingSession from request extensions in upstream
- Introduce ThinkingSession as per-request handle
- Remove dead code from thinking pipeline
- Make encode_project_path pub, restore tests

### Testing

- Add 32 tests for reverse model mapping
- Add integration tests for args pipeline
- Add pipeline isolation and ThinkingSession tests

## [0.3.1] - 2026-02-11

### Bug Fixes

- Detect Option key via macOS CGEvent for Warp terminal
- Propagate arrow keys with Control and Alt/Option modifiers
- Improve session flag handling, warnings, and clippy compliance
- Apply saved settings env vars on initial PTY spawn
- Use narrow centered scrollbar char with distinct color
- Add gap between content and scrollbar
- Move scrollbar inside dialog border
- Use █ for scrollbar thumb to contrast with border
- Replace ratatui Scrollbar with manual draw for constant thumb size
- Scrollbar thumb not reaching bottom of track
- Scrollbar not reaching bottom with small scroll range
- History dialog time alignment with multi-byte chars

### Chore

- Release v0.3.1
- Enforce no inline #[cfg(test)] in src/ via lint
- Remove .DS_Store from tracking

### Documentation

- Add design docs for settings menu and agent teams integration
- Consolidate test convention to single tests/ directory
- Add testing rules to AGENTS.md
- Add missing config fields to README Full Example
- Remove Ctrl+V from hotkeys table

### Features

- Add term_input crate for lossless raw byte terminal input
- Add mouse text selection and input improvements
- Save pasted images to temp files instead of data URIs
- Replace --continue with --session-id/--resume UUID targeting
- Add Settings Menu (Ctrl+E) with PTY restart
- Add settings configuration layer and PtySpawnConfig
- Buffer user input during PTY startup and flush on ready
- Migrate terminal emulator from vt100 to alacritty_terminal
- Always show Esc/Ctrl+H in history dialog footer
- Unified centered footer in PopupDialog, fix scrollbar
- Add legend and scrollbar to history dialog

### Refactor

- Replace crossterm event parsing with term_input
- Centralize PTY lifecycle with Restarting state and generation counter
- Unify logging by removing tracing in favor of DebugLogger
- Abstract terminal emulator behind TerminalEmulator trait
- Move scrollbar into PopupDialog component
- Extract unified PopupDialog into ui/components

### Testing

- Migrate all inline tests from src/ to tests/
- Add PTY lifecycle and startup readiness tests

## [0.3.0] - 2026-02-07

### Bug Fixes

- Resolve all clippy warnings, remove dead code, add workspace lints
- Prevent haiku sub-requests from evicting confirmed thinking blocks
- Resolve 7 concurrency issues from analysis
- Skip reqwest timeout for SSE streaming requests
- Structural SSE thinking event detection, replace naive text search
- Shared SSE parser and thinking cache eviction
- Require explicit thinking_compat=true, no auto-detect
- Fail fast on invalid config instead of silent fallback to defaults
- Respect thinking_budget_tokens config over max_tokens
- Serialize body after adaptive thinking conversion
- Log thinking compat events to debug.log via DebugLogger
- Properly accumulate SSE thinking deltas before registering blocks
- Call on_backend_switch for all modes and remove outdated comment
- Add error logging for filter serialization failure

### Chore

- Release v0.3.0
- Add justfile with release and check commands
- Add git-cliff config and generate CHANGELOG
- Add .DS_Store to gitignore

### Documentation

- Fix README config errors, add installation and development sections
- Add verification instructions to AGENTS.md
- Update README to reflect current architecture
- Document side-effect pattern in cancel/complete_summarization
- Add terminal emulator crate comparison analysis
- Add thinking blocks architecture documentation
- Add research findings to tool-context-preservation design

### Features

- Add backend history dialog (Ctrl+H)
- Convert adaptive thinking to enabled for non-Anthropic backends
- **ui:** Display thinking mode in header and status popup
- **thinking:** Implement NativeTransformer for passthrough mode
- **thinking:** Improve hash reliability with prefix + suffix
- **thinking:** Add confirmed flag and timestamp-based cleanup
- **thinking:** Add ThinkingRegistry for session-based thinking block tracking
- **debug:** Improve debug logging with full body capture and SSE summaries

### Refactor

- Remove dead code, fix bug, delete outdated docs post-cleanup
- Remove summarize and strip thinking modes, keep only native
- Remove dead SummarizeIntent::Success variant
- Add dispatch_mvi! macro and comprehensive MVI tests
- Unify history dialog visibility with focus management
- Eliminate RetrySummarization from InputAction, consolidate retry logic
- Embed button selection into SummarizeDialogState::Failed
- Remove dead MVI code (popup.rs, CancelSummarization, dead intents, Success state)
- Migrate PtyState to full MVI pattern
- Remove legacy retry-on-400 thinking block handling

### Testing

- Add unit tests for thinking compat functions

## [0.2.0] - 2026-02-03

### Bug Fixes

- Only save messages from chat completion requests
- **pty:** Buffer stdin input during Claude Code startup
- **config:** Remove ConfigWatcher to fix backend override race condition
- **ui:** Prevent rendering artifacts in terminal display
- **logging:** Disable logging by default in TUI mode
- **proxy:** Resolve 400 errors when switching backends
- **auth:** Replace AuthType::None with Passthrough for OAuth support
- **clipboard:** Inline image paste data URIs
- **clipboard:** Handle Ctrl+V for image paste
- **ui:** Header bar style matches footer bar formatting
- **pty:** Enable clipboard shortcuts passthrough (Ctrl+C/Ctrl+V)
- **ui:** Header bar style matches footer bar formatting
- **ui:** Remove tracing that corrupts TUI header display
- **ui:** Add arrow indicator for backend selector keyboard navigation
- **ui:** Apply highlight to spans for keyboard navigation
- **ui:** Improve backend selector popup layout and visibility
- Improve backend selector popup
- **config:** Restore ~/.config path fix that was lost in merge
- **proxy:** Strip auth headers before forwarding to upstream
- **config:** Use ~/.config path on all Unix platforms
- **ipc:** Add Display/Error traits, trace logging, and timeout test
- **metrics:** Improve timeout tracking and percentile calculation
- Consolidate upstream request timeout
- Address code review feedback
- **ui:** Polish spacing and clear scrollback
- **ui:** Add header borders
- **ui:** Size PTY to body
- **deps:** Restore portable-pty, crossterm, io-util; organize deps
- Inherit cwd for spawned claude

### Chore

- ClaudeWrapper → AnyClaude
- **deps:** Update major versions - thiserror, dirs, notify
- **deps:** Update crossterm 0.27 → 0.29, ratatui 0.26 → 0.30
- **deps:** Update portable-pty, axum, reqwest
- **deps:** Update toml, tower, signal-hook
- **deps:** Update uuid 1.12 → 1.20
- Drop refactor plan doc

### Documentation

- Remove hardcoded vendor references from architecture doc
- Fix motivation - focus on Anthropic-compatible backends
- Add goal section explaining the motivation
- Update README with current project state
- Simplify AGENTS.md to reference ARCHITECTURE.md
- Align observability design with implementation
- Remove temporary design doc
- Refresh agent instructions
- Add ARCHITECTURE.md and update AGENTS.md

### Features

- **Debug Mode & Request Logging:** Final implementation
- **proxy:** Add dynamic port allocation with fallback
- Add --backend CLI argument
- **terminal:** Migrate from termwiz to vt100 for scrollback support
- **mouse:** Implement proper mouse event forwarding to Claude Code
- **error:** Add centralized error registry and UI display
- **pty:** Auto-shutdown when Claude process terminates
- **shutdown:** Add graceful shutdown handling
- **clipboard:** Add image and file paste support
- **ui:** Add backend selector popup behavior
- **ui:** Center popups by content size
- **thinking:** Add convert_to_tags mode for thinking blocks
- **Add convert_to_tags mode for thinking blocks:** Final implementation
- **config:** Remove models field from backend config
- **config:** Drop auth_env_var in favor of api_key
- **config:** Support direct api_key fallback
- **Remove models field from backend config:** Final implementation
- **config:** Add api_key field to Backend for direct key storage
- **Wire all components together:** Final implementation
- **proxy:** Add session auth and env injection
- **IPC layer for TUI communication:** Final implementation
- **metrics:** Add observability pipeline
- **proxy:** Implement connection pooling and retry with exponential backoff
- **backend:** Implement hot-swap routing for backend switching
- Implement error handling and timeouts
- **Config integration for upstream:** Final implementation
- Add SSE streaming support to proxy
- Add structured logging with tracing
- Implement graceful shutdown handling
- **config:** Add hot-reload with file watching and debouncing
- **config:** Add environment variable resolution for API keys
- **config:** Implement TOML config loader with validation
- **config:** Define Config, Defaults, and Backend structs with serde
- Route keyboard hotkeys
- **ui:** Render pty output
- Add hotkey footer hints
- **ui:** Render status header bar
- Compute body layout rect
- **ui:** Add color theme palette
- **ui:** Scaffold ratatui app runtime
- Handle PTY resize events
- **pty:** Restore PTY runner in module architecture
- Add termwiz vt parser wrapper
- Scaffold module layout
- **Implement input passthrough to PTY:** Final implementation
- **Proxy command-line arguments to claude process:** Final implementation
- **pty:** Enable interactive claude sessions
- **Initialize Rust project with core dependencies:** Final implementation

### Refactor

- Split proxy, metrics, and ipc modules for maintainability
- **config:** Remove unused models field from Backend
- **proxy:** Remove session auth, add passthrough mode
- Migrate proxy to axum and reqwest
- **ui,pty:** Split monolithic modules

### Testing

- Add comprehensive e2e testing suite
- Add CLI argument tests
- **thinking:** Add tests for backend switch scenarios
- Remove useless test_connection_tracking
- Add PTY passthrough integration tests


