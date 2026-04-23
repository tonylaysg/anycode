//! Configuration management for anycode.

mod auth;
pub mod claude_settings;
mod credentials;
mod loader;
mod store;
mod types;

pub use auth::{build_auth_header, AuthHeader};
pub use claude_settings::{
    ClaudeSettingsManager, SettingDef, SettingId, SettingSection, SettingsFieldSnapshot,
};
pub use credentials::{AuthType, CredentialStatus, SecureString};
pub use loader::{save_claude_settings, save_config, save_profile, ConfigError};
pub use store::ConfigStore;
pub use types::{
    AgentsConfig, Backend, BackendPricing, CliProfile, Config, DebugLogDestination,
    DebugLogFormat, DebugLogLevel, DebugLogRotation, DebugLogRotationMode, DebugLoggingConfig,
    Defaults, ProxyConfig, TerminalConfig, WebuiConfig,
};
