/// Which CLI tool this anycode instance is wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliMode {
    /// Wrapping Claude Code (`claude` binary).
    Claude,
    /// Wrapping GitHub Copilot CLI (`copilot` binary).
    Copilot,
}

impl CliMode {
    /// Detect mode from argv[0]: if the binary name contains "copilot", use Copilot mode.
    pub fn detect() -> Self {
        let exe = std::env::args().next().unwrap_or_default();
        let stem = std::path::Path::new(&exe)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if stem.contains("copilot") {
            CliMode::Copilot
        } else {
            CliMode::Claude
        }
    }

    /// The binary to spawn inside the PTY.
    pub fn binary(self) -> &'static str {
        match self {
            CliMode::Claude => "claude",
            CliMode::Copilot => "copilot",
        }
    }

    /// The environment variable used to inject the proxy URL into the spawned CLI.
    pub fn proxy_env_var(self) -> &'static str {
        match self {
            CliMode::Claude => "ANTHROPIC_BASE_URL",
            CliMode::Copilot => "COPILOT_API_URL",
        }
    }

    /// The config section name used in TOML and log messages.
    pub fn profile_key(self) -> &'static str {
        match self {
            CliMode::Claude => "claude",
            CliMode::Copilot => "copilot",
        }
    }

    /// Whether to apply Anthropic-specific auth bypass logic.
    pub fn is_claude(self) -> bool {
        matches!(self, CliMode::Claude)
    }
}
