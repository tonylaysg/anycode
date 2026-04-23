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

    /// Always-present: proxy URL for Claude API.
    pub fn with_proxy_url(mut self, url: &str) -> Self {
        self.vars.push(("ANTHROPIC_BASE_URL".into(), url.into()));
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
    ///   1. Inject `ANTHROPIC_API_KEY=anyclaude-proxy` — keeps Claude Code happy.
    ///   2. Explicitly unset `ANTHROPIC_AUTH_TOKEN` by setting it to empty — prevents
    ///      the "MAuth conflict" warning Claude Code emits when both are set.
    pub fn with_auth_bypass(mut self, is_passthrough: bool) -> Self {
        let has_auth_token = std::env::var("ANTHROPIC_AUTH_TOKEN")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let has_api_key = std::env::var("ANTHROPIC_API_KEY")
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        if is_passthrough {
            // For passthrough: real credentials are forwarded by the proxy.
            // Only inject a placeholder if the user has no real credentials at all.
            if !has_auth_token && !has_api_key {
                self.vars.push(("ANTHROPIC_API_KEY".into(), "anyclaude-proxy".into()));
            }
        } else {
            // For api_key / bearer: proxy handles all auth.
            // Inject placeholder key so Claude Code skips login check.
            self.vars.push(("ANTHROPIC_API_KEY".into(), "anyclaude-proxy".into()));
            // If a real auth token is present (e.g., from ~/.bashrc), clear it to
            // avoid Claude Code's "MAuth conflict" error.
            if has_auth_token {
                self.vars.push(("ANTHROPIC_AUTH_TOKEN".into(), String::new()));
            }
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
