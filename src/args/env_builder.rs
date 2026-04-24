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
    /// * `COPILOT_PROVIDER_TYPE=<type>` — provider family ("anthropic" |
    ///   "openai" | "azure"). Drives which endpoint the CLI posts to.
    /// * `COPILOT_PROVIDER_API_KEY=<session_token>` — proxy authenticates the
    ///   incoming request against this (also accepted as `Authorization: Bearer`
    ///   by the proxy's auth middleware).
    /// * `COPILOT_PROVIDER_WIRE_API=<wire>` — "completions" (`/chat/completions`,
    ///   default) or "responses" (`/v1/responses`, required for GPT-5 series).
    ///   Only emitted for openai/azure; Copilot CLI logs a warning and ignores
    ///   the value for `type=anthropic`.
    /// * `COPILOT_HOME=<isolated dir>` — keeps Copilot CLI data out of the
    ///   system-wide `~/.copilot` directory so BYOK sessions don't collide with
    ///   an OAuth-authenticated installation.
    ///
    /// `provider_type` is parsed as one of:
    /// * `"anthropic"`                → TYPE=anthropic (CLI posts /v1/messages)
    /// * `"openai"` / `"openai-completions"` → TYPE=openai, WIRE=completions
    ///   (CLI posts /chat/completions)
    /// * `"openai-responses"`         → TYPE=openai, WIRE=responses (CLI posts
    ///   /v1/responses — GPT-5 series)
    /// * `"azure"` / `"azure-completions"` / `"azure-responses"` — same shape
    ///   as openai with provider type=azure.
    ///
    /// Any unrecognized value falls back to `anthropic`.
    pub fn with_copilot_env(
        mut self,
        proxy_url: &str,
        session_token: &str,
        provider_type: &str,
    ) -> Self {
        let (ptype, wire) = match provider_type {
            "anthropic" => ("anthropic", None),
            "openai" | "openai-completions" => ("openai", Some("completions")),
            "openai-responses" => ("openai", Some("responses")),
            "azure" | "azure-completions" => ("azure", Some("completions")),
            "azure-responses" => ("azure", Some("responses")),
            _ => ("anthropic", None),
        };
        self.vars.push(("COPILOT_OFFLINE".into(), "true".into()));
        self.vars.push(("COPILOT_PROVIDER_BASE_URL".into(), proxy_url.into()));
        self.vars.push(("COPILOT_PROVIDER_TYPE".into(), ptype.into()));
        self.vars.push(("COPILOT_PROVIDER_API_KEY".into(), session_token.into()));
        if let Some(w) = wire {
            self.vars.push(("COPILOT_PROVIDER_WIRE_API".into(), w.into()));
        }

        // Copilot CLI ≥1.0.35 refuses to start under BYOK unless a model is
        // explicitly configured — it prints
        //   "BYOK providers require an explicit model. Run `copilot help
        //    providers` for configuration details."
        // and exits within ~1s, before writing its own `process-*.log`.
        //
        // Copilot accepts any of `COPILOT_MODEL`, `COPILOT_PROVIDER_MODEL_ID`,
        // or `--model <name>`. We honour the user's environment first: if any
        // of the three are already set in the parent env, we don't override
        // them. Otherwise we inject a sensible default whose family matches
        // the active wire protocol, since anycode's proxy re-maps model names
        // to the backend's configured `model_opus` / `model_sonnet` / …
        // before forwarding.
        let user_has_model = std::env::var_os("COPILOT_MODEL").is_some_and(|v| !v.is_empty())
            || std::env::var_os("COPILOT_PROVIDER_MODEL_ID").is_some_and(|v| !v.is_empty());
        if !user_has_model {
            let default_model = match (ptype, wire) {
                ("anthropic", _) => "claude-sonnet-4-5",
                (_, Some("responses")) => "gpt-5",
                _ => "gpt-4o",
            };
            self.vars.push(("COPILOT_MODEL".into(), default_model.into()));
        }

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
