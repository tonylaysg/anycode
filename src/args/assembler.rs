//! Argument assembler — all CLI args in one place.

use crate::args::classifier::ClassifiedArg;
use crate::args::session::SessionResolution;
use crate::args::SessionMode;
use crate::config::ClaudeSettingsManager;
use crate::shim::TeammateShim;

/// Builder for CLI arguments passed to the spawned claude process.
#[derive(Debug, Clone)]
pub struct ArgAssembler {
    args: Vec<String>,
    copilot_mode: bool,
}

impl ArgAssembler {
    /// Start with passthrough args (filtered from classified args).
    ///
    /// Wrapper-owned and intercepted flags are excluded — they've been consumed.
    pub fn from_passthrough(classified: &[ClassifiedArg]) -> Self {
        let args = classified
            .iter()
            .filter_map(|a| match a {
                ClassifiedArg::KnownPassthrough { flag, value } => {
                    let mut v = vec![flag.clone()];
                    if let Some(val) = value {
                        v.push(val.clone());
                    }
                    Some(v)
                }
                ClassifiedArg::UnknownPassthrough(s) => Some(vec![s.clone()]),
                ClassifiedArg::Positional(s) => Some(vec![s.clone()]),
                // Wrapper-owned and Intercepted are consumed
                ClassifiedArg::WrapperOwned { .. } | ClassifiedArg::Intercepted { .. } => None,
            })
            .flatten()
            .collect();
        Self { args, copilot_mode: false }
    }

    /// Start with an empty arg list.
    pub fn new() -> Self {
        Self { args: Vec::new(), copilot_mode: false }
    }

    /// Enable Copilot-CLI mode — subsequent `with_*` calls emit Copilot-compatible
    /// flags (or become no-ops for Claude-only features like teammate/subagent hooks).
    pub fn copilot_mode(mut self, enabled: bool) -> Self {
        self.copilot_mode = enabled;
        // Strip Claude-only session flags that may have leaked through passthrough.
        if enabled {
            self.strip_claude_only_flags();
        }
        self
    }

    fn strip_claude_only_flags(&mut self) {
        // `--session-id <id>` is Claude-only; Copilot CLI rejects it.
        let mut i = 0;
        while i < self.args.len() {
            if self.args[i] == "--session-id" {
                let end = (i + 2).min(self.args.len());
                self.args.drain(i..end);
            } else {
                i += 1;
            }
        }
    }

    /// Inject session flag based on mode.
    pub fn with_session(mut self, session: &SessionResolution, mode: SessionMode) -> Self {
        if self.copilot_mode {
            // Copilot CLI has no `--session-id`. For fresh spawns we let
            // Copilot auto-assign its own session ID. For resume we use
            // `--resume=<id>` (the `=` form, which the Copilot CLI requires
            // since `--resume` has an optional value).
            if matches!(mode, SessionMode::Resume) {
                self.args.push(format!("--resume={}", session.session_id));
            }
            return self;
        }
        match mode {
            SessionMode::Initial => {
                self.args.push("--session-id".into());
                self.args.push(session.session_id.clone());
            }
            SessionMode::Resume => {
                self.args.push("--resume".into());
                self.args.push(session.session_id.clone());
            }
        }
        self
    }

    /// Force `--resume <id>` (used by Ctrl+R restart).
    pub fn with_session_resume(mut self, session_id: &str) -> Self {
        if self.copilot_mode {
            self.args.push(format!("--resume={}", session_id));
        } else {
            self.args.push("--resume".into());
            self.args.push(session_id.into());
        }
        self
    }

    /// From settings manager (CLI flags from registry).
    pub fn with_settings(mut self, settings: &ClaudeSettingsManager) -> Self {
        self.args.extend(settings.to_cli_args());
        self
    }

    /// From teammate shim (--teammate-mode tmux).
    ///
    /// Claude-specific — no-op in Copilot mode.
    pub fn with_teammate_mode(mut self, shim: Option<&TeammateShim>) -> Self {
        if self.copilot_mode {
            return self;
        }
        if shim.is_some() {
            self.args.push("--teammate-mode".into());
            self.args.push("tmux".into());
        }
        self
    }

    /// Inject SubagentStart/SubagentStop hooks via `--settings` CLI flag.
    ///
    /// Claude-specific — no-op in Copilot mode (Copilot CLI has no
    /// `--settings` JSON flag and no SubagentStart hook concept).
    ///
    /// CC merges these with user settings at runtime. No user files are modified.
    /// The hooks use curl to POST to the proxy, which returns `additionalContext`
    /// with a backend marker for session affinity.
    pub fn with_subagent_hooks(mut self, proxy_port: u16) -> Self {
        if self.copilot_mode {
            return self;
        }
        let hooks_json = format!(
            r#"{{"hooks":{{"SubagentStart":[{{"matcher":"","hooks":[{{"type":"command","command":"curl -s -m 5 -X POST http://127.0.0.1:{port}/api/subagent-start -d @- -H 'Content-Type: application/json'"}}]}}],"SubagentStop":[{{"matcher":"","hooks":[{{"type":"command","command":"curl -s -m 5 -X POST http://127.0.0.1:{port}/api/subagent-stop -d @- -H 'Content-Type: application/json'"}}]}}]}}}}"#,
            port = proxy_port
        );
        self.args.push("--settings".into());
        self.args.push(hooks_json);
        self
    }

    /// Add arbitrary extra arguments.
    pub fn with_extra(mut self, extra: Vec<String>) -> Self {
        self.args.extend(extra);
        self
    }

    /// Build the final argument list.
    pub fn build(self) -> Vec<String> {
        self.args
    }
}

impl Default for ArgAssembler {
    fn default() -> Self {
        Self::new()
    }
}
