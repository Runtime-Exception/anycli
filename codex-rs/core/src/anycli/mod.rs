//! AnyCLI multi-provider configuration system.
//!
//! This module provides configuration types and utilities for managing
//! multiple AI provider configurations (OpenAI, Anthropic, Google, NewAPI).
//!
//! Configuration is stored in `~/.anycli/config.toml` separately from the
//! standard Codex configuration at `~/.codex/config.toml`.

pub mod config;
pub mod migrate;
pub mod models;

pub use config::AnycliConfig;
pub use config::ConfigEntry;
pub use config::ProviderType;
pub use migrate::migrate_from_codex;
pub use models::ModelInfo;
pub use models::models_for_provider;

use std::path::PathBuf;

/// Returns the AnyCLI configuration directory path.
/// Default: `~/.anycli/`
pub fn anycli_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".anycli"))
}

/// Returns the AnyCLI configuration file path.
/// Default: `~/.anycli/config.toml`
pub fn anycli_config_path() -> Option<PathBuf> {
    anycli_config_dir().map(|d| d.join("config.toml"))
}

/// Returns true if AnyCLI mode is enabled.
///
/// AnyCLI mode is detected by:
/// 1. The `ANYCLI_MODE` environment variable being set, OR
/// 2. A valid config file exists at `~/.anycli/config.toml` with at least one config entry
///
/// Setting `ANYCLI_MODE=0` or `ANYCLI_MODE=false` will force disable AnyCLI mode.
pub fn is_anycli_mode() -> bool {
    // Check environment variable first
    if let Ok(val) = std::env::var("ANYCLI_MODE") {
        // Allow explicit disable with "0" or "false"
        let disabled = val == "0" || val.eq_ignore_ascii_case("false");
        return !disabled;
    }

    // Check for a valid AnyCLI config file with at least one entry
    if let Some(config_path) = anycli_config_path()
        && config_path.exists()
    {
        // Try to load and validate the config
        if let Ok(config) = config::AnycliConfig::load() {
            return !config.configs.is_empty();
        }
    }

    false
}

/// Ensures the AnyCLI configuration directory exists.
pub fn ensure_anycli_dir() -> std::io::Result<PathBuf> {
    let dir = anycli_config_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not determine home directory",
        )
    })?;

    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }

    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anycli_config_path_structure() {
        let path = anycli_config_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains(".anycli"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }
}
