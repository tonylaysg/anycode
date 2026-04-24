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

    /// Inject all Copilot-mode env vars for **BYOK (Bring-Your-Own-Key)** operation.
    ///
    /// GitHub Copilot CLI natively supports a BYOK / offline mode via the
    /// `COPILOT_PROVIDER_*` environment variables. When `COPILOT_OFFLINE=true`
    /// is set together with a `COPILOT_PROVIDER_BASE_URL`, the CLI:
    ///
    /// * **Skips the GitHub device-code OAuth flow entirely** — no login screen,
    ///   no `github.com/login/device` prompt;
    /// * Disables all GitHub network access (telemetry, web tools, GitHub MCP
    ///   server, auto-update) — the CLI becomes a pure model client;
    /// * Routes **every** model call to `COPILOT_PROVIDER_BASE_URL`, sending
    ///   `Authorization: Bearer <COPILOT_PROVIDER_API_KEY>`.
    ///
    /// This is exactly the integration point anycode needs: point the provider
    /// base URL at our proxy, hand it the session token, and the proxy then
    /// forwards the request to the active backend (anthropic / openai / …) with
    /// the backend's real credentials.
    ///
    /// Variables injected:
    /// * `COPILOT_OFFLINE=true` — disables GitHub auth/telemetry.
    /// * `COPILOT_PROVIDER_BASE_URL=<proxy>` — the anycode proxy's address.
    /// * `COPILOT_PROVIDER_TYPE=<type>` — wire format ("anthropic" | "openai").
    /// * `COPILOT_PROVIDER_API_KEY=<session_token>` — proxy authenticates the
    ///   incoming request against this (also accepted as `Authorization: Bearer`
    ///   by the proxy's auth middleware).
    /// * `COPILOT_PROVIDER_WIRE_API=<wire>` — "completions" (default) or
    ///   "responses". anycode always ships "completions".
    /// * `COPILOT_HOME=<isolated dir>` — keeps Copilot CLI data out of the
    ///   system-wide `~/.copilot` directory so BYOK sessions don't collide with
    ///   an OAuth-authenticated installation.
    ///
    /// `provider_type` controls which wire format the CLI emits and thus which
    /// proxy inbound endpoint it hits (`/v1/messages` for anthropic,
    /// `/v1/chat/completions` for openai).
    pub fn with_copilot_env(
        mut self,
        proxy_url: &str,
        session_token: &str,
        provider_type: &str,
    ) -> Self {
        self.vars.push(("COPILOT_OFFLINE".into(), "true".into()));
        self.vars.push(("COPILOT_PROVIDER_BASE_URL".into(), proxy_url.into()));
        self.vars.push(("COPILOT_PROVIDER_TYPE".into(), provider_type.into()));
        self.vars.push(("COPILOT_PROVIDER_API_KEY".into(), session_token.into()));
        self.vars.push(("COPILOT_PROVIDER_WIRE_API".into(), "completions".into()));

        // Isolate Copilot home directory so BYOK-driven sessions don't share
        // state (credentials, MCP config, session history) with an OAuth-logged-in
        // Copilot installation on the same machine.
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
