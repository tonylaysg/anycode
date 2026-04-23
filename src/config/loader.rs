use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::cli_mode::CliMode;
use crate::config::credentials::CredentialStatus;
use crate::config::types::{Backend, CliProfile, Config};

/// Errors that can occur when loading configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file '{path}': {source}")]
    ReadError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse config file '{path}': {source}")]
    ParseError {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("Config validation failed: {message}")]
    ValidationError { message: String },
}

impl Config {
    /// Returns the path to the configuration file (`~/.config/anycode/config.toml`).
    pub fn config_path() -> PathBuf {
        let config_dir = dirs::home_dir()
            .map(|h| h.join(".config"))
            .unwrap_or_else(|| PathBuf::from("."));
        config_dir.join("anycode").join("config.toml")
    }

    /// Returns the legacy path used by the old `anyclaude` binary.
    pub fn legacy_config_path() -> PathBuf {
        let config_dir = dirs::home_dir()
            .map(|h| h.join(".config"))
            .unwrap_or_else(|| PathBuf::from("."));
        config_dir.join("anyclaude").join("config.toml")
    }

    /// Loads configuration from the default config file.
    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::config_path();
        // Auto-migrate from legacy anyclaude directory if new path doesn't exist yet
        if !path.exists() {
            let legacy = Self::legacy_config_path();
            if legacy.exists() {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::copy(&legacy, &path);
            }
        }
        Self::load_from(&path)
    }

    /// Loads configuration from a specific path, with automatic old-format migration.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Config::default());
        }

        let file = File::open(path).map_err(|e| ConfigError::ReadError {
            path: path.to_path_buf(),
            source: e,
        })?;

        fs2::FileExt::lock_shared(&file).map_err(|e| ConfigError::ReadError {
            path: path.to_path_buf(),
            source: e,
        })?;

        let mut content = String::new();
        (&file)
            .read_to_string(&mut content)
            .map_err(|e| ConfigError::ReadError {
                path: path.to_path_buf(),
                source: e,
            })?;

        // Attempt migration from old flat format before deserializing
        let content = migrate_config_content(&content);

        let config: Config = toml::from_str(&content).map_err(|e| ConfigError::ParseError {
            path: path.to_path_buf(),
            source: e,
        })?;

        Ok(config)
    }

    /// Validates the profile for a given CLI mode.
    pub fn validate_for(&self, mode: CliMode) -> Result<(), ConfigError> {
        let profile = self.profile(mode);
        if profile.backends.is_empty() {
            // Empty copilot profile is OK if not configured
            if mode == CliMode::Copilot {
                return Ok(());
            }
            return Err(ConfigError::ValidationError {
                message: format!(
                    "No backends configured for {} profile",
                    mode.profile_key()
                ),
            });
        }

        let active = &profile.defaults.active;
        let active_backend = profile.backends.iter().find(|b| &b.name == active);

        match active_backend {
            None => {
                return Err(ConfigError::ValidationError {
                    message: format!(
                        "[{}] Active backend '{}' not found in configured backends",
                        mode.profile_key(),
                        active
                    ),
                });
            }
            Some(backend) => {
                if !backend.is_configured() {
                    return Err(ConfigError::ValidationError {
                        message: format!(
                            "[{}] Active backend '{}' is not configured — set api_key in config",
                            mode.profile_key(),
                            backend.name
                        ),
                    });
                }
            }
        }

        if let Some(ref at) = profile.agents {
            if !profile.backends.iter().any(|b| b.name == at.teammate_backend) {
                return Err(ConfigError::ValidationError {
                    message: format!(
                        "[{}] agents.teammate_backend '{}' not found in configured backends",
                        mode.profile_key(),
                        at.teammate_backend
                    ),
                });
            }
            if let Some(ref sb) = at.subagent_backend {
                if !profile.backends.iter().any(|b| b.name == *sb) {
                    return Err(ConfigError::ValidationError {
                        message: format!(
                            "[{}] agents.subagent_backend '{}' not found in configured backends",
                            mode.profile_key(),
                            sb
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    /// Backward-compat validation (validates the Claude profile).
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_for(CliMode::Claude)
    }

    pub fn log_backend_status(&self) {
        for backend in &self.claude.backends {
            match backend.resolve_credential() {
                CredentialStatus::Unconfigured { reason } => {
                    eprintln!("Warning: Backend '{}' is unconfigured - {}", backend.name, reason);
                }
                CredentialStatus::Configured(_) => {
                    eprintln!("Backend '{}' configured", backend.name);
                }
                CredentialStatus::NoAuth => {
                    eprintln!("Backend '{}' configured (no auth required)", backend.name);
                }
            }
        }
    }

    pub fn configured_backends(&self) -> Vec<&Backend> {
        self.claude.backends.iter().filter(|b| b.is_configured()).collect()
    }

    pub fn active_backend(&self) -> Option<&Backend> {
        self.claude
            .backends
            .iter()
            .find(|b| b.name == self.claude.defaults.active && b.is_configured())
    }
}

/// Migrate old flat config format to new dual-profile format.
///
/// Old format has root-level `[defaults]` and `[[backends]]`.
/// New format has `[claude.defaults]` and `[[claude.backends]]`.
fn migrate_config_content(content: &str) -> String {
    let mut value: toml::Value = match toml::from_str(content) {
        Ok(v) => v,
        Err(_) => return content.to_string(),
    };

    if !migrate_toml_value(&mut value) {
        return content.to_string();
    }

    toml::to_string_pretty(&value).unwrap_or_else(|_| content.to_string())
}

/// Returns true if migration was performed.
fn migrate_toml_value(value: &mut toml::Value) -> bool {
    let table = match value.as_table_mut() {
        Some(t) => t,
        None => return false,
    };

    // Already in new format if 'claude' section exists
    if table.contains_key("claude") {
        return false;
    }

    // Old format has root-level 'backends' or 'defaults'
    let has_backends = table.contains_key("backends");
    let has_defaults = table.contains_key("defaults");
    if !has_backends && !has_defaults {
        return false;
    }

    let backends = table.remove("backends");
    let defaults = table.remove("defaults");
    let agents = table.remove("agents");
    let claude_settings = table.remove("claude_settings");

    let mut claude_table = toml::value::Table::new();
    if let Some(d) = defaults {
        claude_table.insert("defaults".to_string(), d);
    }
    if let Some(b) = backends {
        claude_table.insert("backends".to_string(), b);
    }
    if let Some(a) = agents {
        claude_table.insert("agents".to_string(), a);
    }
    if let Some(cs) = claude_settings {
        claude_table.insert("claude_settings".to_string(), cs);
    }

    table.insert("claude".to_string(), toml::Value::Table(claude_table));
    true
}

/// Save a full Config to the given path.
pub fn save_config(path: &Path, config: &Config) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::ReadError {
            path: path.to_path_buf(),
            source: e,
        })?;
    }

    let content = toml::to_string_pretty(config).map_err(|e| ConfigError::ValidationError {
        message: format!("Failed to serialize config: {}", e),
    })?;

    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|e| ConfigError::ReadError {
            path: path.to_path_buf(),
            source: e,
        })?;

    fs2::FileExt::lock_exclusive(&file).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;

    use std::io::Write;
    (&file)
        .write_all(content.as_bytes())
        .map_err(|e| ConfigError::ReadError {
            path: path.to_path_buf(),
            source: e,
        })?;

    Ok(())
}

/// Save claude_settings for the Claude profile.
pub fn save_claude_settings(
    path: &Path,
    settings: &HashMap<String, bool>,
) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::ReadError {
            path: path.to_path_buf(),
            source: e,
        })?;
    }

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|e| ConfigError::ReadError {
            path: path.to_path_buf(),
            source: e,
        })?;

    fs2::FileExt::lock_exclusive(&file).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;

    use std::io::{Read, Seek, SeekFrom, Write};

    let mut raw = String::new();
    (&file).read_to_string(&mut raw).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;

    let mut config: Config = if raw.trim().is_empty() {
        Config::default()
    } else {
        let migrated = migrate_config_content(&raw);
        toml::from_str(&migrated).map_err(|e| ConfigError::ValidationError {
            message: format!("Failed to parse config: {}", e),
        })?
    };
    config.claude.claude_settings = settings.clone();

    let content = toml::to_string_pretty(&config).map_err(|e| ConfigError::ValidationError {
        message: format!("Failed to serialize config: {}", e),
    })?;

    (&file).seek(SeekFrom::Start(0)).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;
    file.set_len(0).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;
    (&file).write_all(content.as_bytes()).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;

    Ok(())
}

/// Save a CliProfile for the given mode back into the config file.
pub fn save_profile(path: &Path, mode: CliMode, profile: &CliProfile) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::ReadError {
            path: path.to_path_buf(),
            source: e,
        })?;
    }

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|e| ConfigError::ReadError {
            path: path.to_path_buf(),
            source: e,
        })?;

    fs2::FileExt::lock_exclusive(&file).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;

    use std::io::{Read, Seek, SeekFrom, Write};

    let mut raw = String::new();
    (&file).read_to_string(&mut raw).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;

    let mut config: Config = if raw.trim().is_empty() {
        Config::default()
    } else {
        let migrated = migrate_config_content(&raw);
        toml::from_str(&migrated).map_err(|e| ConfigError::ValidationError {
            message: format!("Failed to parse config: {}", e),
        })?
    };

    *config.profile_mut(mode) = profile.clone();

    let content = toml::to_string_pretty(&config).map_err(|e| ConfigError::ValidationError {
        message: format!("Failed to serialize config: {}", e),
    })?;

    (&file).seek(SeekFrom::Start(0)).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;
    file.set_len(0).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;
    (&file).write_all(content.as_bytes()).map_err(|e| ConfigError::ReadError {
        path: path.to_path_buf(),
        source: e,
    })?;

    Ok(())
}
