//! Environment builder — all env vars in one place.

use crate::config::ClaudeSettingsManager;
use crate::shim::TeammateShim;

/// Builder for environment variables passed to the spawned process.
#[derive(Debug, Clone)]
pub struct EnvSet {
    vars: Vec<(String, String)>,
}

impl EnvSet {
    /// Create an empty environment set.
    pub fn new() -> Self {
        Self { vars: Vec::new() }
    }

    /// Set the proxy URL using the env var name appropriate for the CLI mode.
    pub fn with_proxy_url_for_mode(mut self, url: &str, proxy_env_var: &str) -> Self {
        self.vars.push((proxy_env_var.into(), url.into()));
        self
    }

    /// Handle Anthropic auth environment variables for the child process.
    ///
    /// Claude Code checks for existing credentials (`ANTHROPIC_AUTH_TOKEN` or
    /// `ANTHROPIC_API_KEY`) before making any requests.  The strategy depends
    /// on the backend type:
    ///
    /// * **passthrough** (`is_passthrough = true`): the proxy forwards the
    ///   client's own credentials unchanged, so we leave real credentials alone.
    ///   If neither credential exists, inject a placeholder so Claude Code
    ///   won't show a login screen (it will fail later, but that's expected for
    ///   misconfigured passthrough setups).
    ///
    /// * **api_key / bearer** (`is_passthrough = false`): the proxy strips
    ///   incoming auth and injects the backend's own key.  We must:
    ///   1. Inject `ANTHROPIC_API_KEY=anycode-proxy` — keeps Claude Code happy.
    ///   2. Explicitly unset `ANTHROPIC_AUTH_TOKEN` by setting it to empty — prevents
    ///      the "MAuth conflict" warning Claude Code emits when both are set.
    pub fn with_auth_bypass(mut self, is_passthrough: bool) -> Self {
        if is_passthrough {
            // For passthrough: real credentials are forwarded by the proxy unchanged.
            // Do not inject a placeholder — if the user has no credentials, let
            // Claude Code show its normal login flow so the user can authenticate.
        } else {
            // For api_key / bearer: proxy handles all real auth — credentials in
            // ANTHROPIC_API_KEY / ANTHROPIC_AUTH_TOKEN are stripped and replaced
            // by the proxy before forwarding to the backend.
            //
            // Strategy: inject ANTHROPIC_AUTH_TOKEN with a dummy value so that
            // Claude Code enters "session token" mode and skips its login screen.
            // Auth-token mode does NOT trigger the "do you want to use this API key?"
            // confirmation dialog that ANTHROPIC_API_KEY mode does.
            //
            // Also clear ANTHROPIC_API_KEY (set to empty) so Claude Code doesn't
            // see two credential types simultaneously ("MAuth conflict").
            self.vars.push(("ANTHROPIC_AUTH_TOKEN".into(), "anycode-proxy".into()));
            self.vars.push(("ANTHROPIC_API_KEY".into(), String::new()));
        }
        self
    }

    /// Session token for proxy authentication via ANTHROPIC_CUSTOM_HEADERS.
    pub fn with_session_token(mut self, token: &str) -> Self {
        // Format: newline-separated headers as "name:value" pairs
        self.vars.push(("ANTHROPIC_CUSTOM_HEADERS".into(), format!("x-session-token:{}", token)));
        self
    }

    /// From settings manager (agent teams, etc.)
    pub fn with_settings(mut self, settings: &ClaudeSettingsManager) -> Self {
        self.vars.extend(settings.to_env_vars());
        self
    }

    /// From teammate shim (PATH override).
    pub fn with_shim(mut self, shim: Option<&TeammateShim>) -> Self {
        if let Some(s) = shim {
            self.vars.push(s.path_env());
        }
        self
    }

    /// Inject all Copilot-mode env vars:
    ///
    /// * `COPILOT_API_URL` — already set by `with_proxy_url_for_mode`; this adds the rest.
    /// * `ANTHROPIC_BASE_URL` — Copilot CLI's `sweagent-anthropic` agent reads
    ///   `ANTHROPIC_BASE_URL` (not `COPILOT_API_URL`) for its Anthropic SDK calls.
    ///   Pointing it at the same proxy lets us intercept those requests too.
    /// * `ANTHROPIC_API_KEY` — Anthropic SDK requires a non-empty key even when
    ///   using a custom base URL. A placeholder prevents "no API key" errors.
    /// * `COPILOT_HOME` — isolates Copilot data from the system-wide installation.
    pub fn with_copilot_env(mut self, proxy_url: &str) -> Self {
        // Capture Anthropic-SDK-based traffic (sweagent-anthropic agent).
        self.vars.push(("ANTHROPIC_BASE_URL".into(), proxy_url.into()));
        // Placeholder so the Anthropic SDK doesn't refuse to start.
        self.vars.push(("ANTHROPIC_API_KEY".into(), "anycode-copilot-proxy".into()));
        // Isolate Copilot home directory.
        if let Some(home) = dirs::home_dir() {
            let copilot_home = home.join(".config/anycode/copilot-home");
            self.vars.push(("COPILOT_HOME".into(), copilot_home.to_string_lossy().into_owned()));
        }
        self
    }

    /// Add arbitrary extra environment variables.
    pub fn with_extra(mut self, extra: Vec<(String, String)>) -> Self {
        self.vars.extend(extra);
        self
    }

    /// Build the final environment variable list.
    pub fn build(self) -> Vec<(String, String)> {
        self.vars
    }
}

impl Default for EnvSet {
    fn default() -> Self {
        Self::new()
    }
}
