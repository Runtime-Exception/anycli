//! Migration utilities for importing Codex configuration into AnyCLI.
//!
//! This module provides functionality to detect and import existing Codex
//! configurations, allowing users to seamlessly transition to AnyCLI.

use super::config::AnycliConfig;
use super::config::ConfigEntry;
use super::config::ProviderType;
use crate::error::CodexErr;
use crate::error::Result;
use std::path::PathBuf;

/// Returns the Codex configuration directory path.
fn codex_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex"))
}

/// Returns the Codex configuration file path.
fn codex_config_path() -> Option<PathBuf> {
    codex_config_dir().map(|d| d.join("config.toml"))
}

/// Returns true if a Codex configuration exists that can be migrated.
pub fn has_codex_config() -> bool {
    codex_config_path().map(|p| p.exists()).unwrap_or(false)
}

/// Migrates existing Codex configuration to AnyCLI format.
///
/// This function:
/// 1. Checks for existing `~/.codex/config.toml`
/// 2. Reads the OpenAI model and provider settings
/// 3. Creates an AnyCLI configuration with an "openai-default" entry
/// 4. Saves the new configuration to `~/.anycli/config.toml`
///
/// Returns the created AnyCLI configuration, or None if no Codex config exists.
pub fn migrate_from_codex() -> Result<Option<AnycliConfig>> {
    let codex_path = match codex_config_path() {
        Some(p) if p.exists() => p,
        _ => return Ok(None),
    };

    // Read the Codex config file
    let content = std::fs::read_to_string(&codex_path)
        .map_err(|e| CodexErr::Fatal(format!("Failed to read Codex config: {e}")))?;

    // Parse as TOML to extract model information
    let codex_toml: toml::Value = toml::from_str(&content)
        .map_err(|e| CodexErr::Fatal(format!("Failed to parse Codex config: {e}")))?;

    // Extract model from Codex config
    let model = codex_toml
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("gpt-5.1-codex-max")
        .to_string();

    // Create the OpenAI config entry
    let mut openai_entry = ConfigEntry::new(ProviderType::OpenAI, model);

    // Check for API key environment variable customization
    if let Some(providers) = codex_toml.get("model_providers")
        && let Some(openai) = providers.get("openai")
    {
        if let Some(env_key) = openai.get("env_key").and_then(|v| v.as_str()) {
            openai_entry.env_key = Some(env_key.to_string());
        }
        if let Some(base_url) = openai.get("base_url").and_then(|v| v.as_str()) {
            openai_entry.endpoint = Some(base_url.to_string());
        }
    }

    // Create the AnyCLI config
    let mut anycli_config = AnycliConfig::new();
    anycli_config.add_config("openai-default".to_string(), openai_entry);
    anycli_config.active_config = "openai-default".to_string();

    // Ensure the AnyCLI directory exists
    super::ensure_anycli_dir()?;

    // Save the new configuration
    anycli_config.save()?;

    tracing::info!(
        "Migrated Codex configuration to AnyCLI: {:?}",
        super::anycli_config_path()
    );

    Ok(Some(anycli_config))
}

/// Performs migration if needed and returns the AnyCLI configuration.
///
/// This function:
/// 1. Checks if AnyCLI config already exists - if so, loads it
/// 2. If not, checks for Codex config and migrates it
/// 3. If neither exists, returns an empty configuration
pub fn ensure_anycli_config() -> Result<AnycliConfig> {
    // First, try to load existing AnyCLI config
    let anycli_path = super::anycli_config_path();
    if let Some(path) = &anycli_path
        && path.exists()
    {
        return AnycliConfig::load_from_path(path);
    }

    // No AnyCLI config exists, try to migrate from Codex
    if let Some(config) = migrate_from_codex()? {
        return Ok(config);
    }

    // No config exists at all, return empty config
    Ok(AnycliConfig::new())
}

/// Returns migration status information.
#[derive(Debug, Clone)]
pub struct MigrationStatus {
    /// Whether a Codex configuration exists.
    pub has_codex_config: bool,
    /// Whether an AnyCLI configuration exists.
    pub has_anycli_config: bool,
    /// Whether migration is needed (Codex exists but AnyCLI doesn't).
    pub migration_needed: bool,
}

impl MigrationStatus {
    /// Checks the current migration status.
    pub fn check() -> Self {
        let has_codex = has_codex_config();
        let has_anycli = super::anycli_config_path()
            .map(|p| p.exists())
            .unwrap_or(false);

        Self {
            has_codex_config: has_codex,
            has_anycli_config: has_anycli,
            migration_needed: has_codex && !has_anycli,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_status_empty() {
        // In a clean test environment, neither config should exist
        // (unless the test is run on a machine with actual configs)
        let status = MigrationStatus::check();
        // We can't assert specific values since it depends on the test environment
        // but we can verify the logic is consistent
        assert_eq!(
            status.migration_needed,
            status.has_codex_config && !status.has_anycli_config
        );
    }

    #[test]
    fn test_parse_codex_config_extract_model() {
        let codex_toml = r#"
model = "gpt-4o"
approval_policy = "on-request"

[model_providers.openai]
name = "OpenAI"
env_key = "MY_OPENAI_KEY"
base_url = "https://custom.api.com/v1"
"#;

        let parsed: toml::Value = toml::from_str(codex_toml).unwrap();

        // Extract model
        let model = parsed
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        assert_eq!(model, "gpt-4o");

        // Extract custom env_key
        let env_key = parsed
            .get("model_providers")
            .and_then(|p| p.get("openai"))
            .and_then(|o| o.get("env_key"))
            .and_then(|v| v.as_str());
        assert_eq!(env_key, Some("MY_OPENAI_KEY"));

        // Extract custom base_url
        let base_url = parsed
            .get("model_providers")
            .and_then(|p| p.get("openai"))
            .and_then(|o| o.get("base_url"))
            .and_then(|v| v.as_str());
        assert_eq!(base_url, Some("https://custom.api.com/v1"));
    }

    #[test]
    fn test_config_entry_from_codex() {
        // Simulate creating a config entry from Codex values
        let mut entry = ConfigEntry::new(ProviderType::OpenAI, "gpt-4o".to_string());
        entry.env_key = Some("MY_OPENAI_KEY".to_string());
        entry.endpoint = Some("https://custom.api.com/v1".to_string());

        assert_eq!(entry.provider_type, ProviderType::OpenAI);
        assert_eq!(entry.model, "gpt-4o");
        assert_eq!(entry.env_key, Some("MY_OPENAI_KEY".to_string()));
        assert_eq!(
            entry.endpoint,
            Some("https://custom.api.com/v1".to_string())
        );
    }
}
