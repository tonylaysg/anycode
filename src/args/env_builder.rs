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
    ///
    /// If no real Anthropic credentials exist in the current environment
    /// (`ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_API_KEY` are both absent/empty),
    /// injects a placeholder `ANTHROPIC_API_KEY` so Claude Code skips its own
    /// login check — actual authentication is handled entirely by the proxy.
    ///
    /// When real credentials ARE present we leave them untouched, avoiding the
    /// "MAuth conflict" error Claude Code emits when both token types are set.
    pub fn with_proxy_url(mut self, url: &str) -> Self {
        self.vars.push(("ANTHROPIC_BASE_URL".into(), url.into()));

        let has_auth_token = std::env::var("ANTHROPIC_AUTH_TOKEN")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let has_api_key = std::env::var("ANTHROPIC_API_KEY")
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        if !has_auth_token && !has_api_key {
            // No real credentials → inject placeholder so Claude Code won't
            // show the login screen.  The proxy replaces auth on every request.
            self.vars.push(("ANTHROPIC_API_KEY".into(), "anyclaude-proxy".into()));
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
