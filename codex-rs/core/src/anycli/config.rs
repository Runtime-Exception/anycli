//! AnyCLI configuration types and loading.
//!
//! This module defines the configuration structure for AnyCLI, supporting
//! multiple AI provider configurations that can be switched at runtime.

use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

use crate::error::CodexErr;
use crate::error::Result;
use crate::model_provider_info::ModelProviderInfo;
use crate::model_provider_info::WireApi;

/// Agent execution mode for AnyCLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// Classic single-agent execution (current default behavior).
    #[default]
    Classic,
    /// Two-phase Alloy Agent (analyze + implement with different models).
    Alloy,
}

impl AgentMode {
    /// Returns the display name for this agent mode.
    pub fn display_name(&self) -> &'static str {
        match self {
            AgentMode::Classic => "Classic Agent",
            AgentMode::Alloy => "Alloy Agent",
        }
    }

    /// Returns a description of this agent mode.
    pub fn description(&self) -> &'static str {
        match self {
            AgentMode::Classic => "Single model handles all tasks",
            AgentMode::Alloy => "Two-phase: Analyze (GPT) → Implement (Claude)",
        }
    }
}

impl std::fmt::Display for AgentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Configuration for the Alloy Agent two-phase execution mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AlloyConfig {
    /// The configuration name for the Analyze Agent.
    /// Should reference an existing config in the `configs` HashMap.
    /// Recommended: GPT 5.2 (400k context) for detailed analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyze_config: Option<String>,

    /// The configuration name for the Implementation Agent.
    /// Should reference an existing config in the `configs` HashMap.
    /// Recommended: Claude Opus 4.5 (200k context) for precise execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implement_config: Option<String>,
}

impl AlloyConfig {
    /// Returns true if the Alloy configuration is complete (both agents configured).
    pub fn is_configured(&self) -> bool {
        self.analyze_config.is_some() && self.implement_config.is_some()
    }

    /// Validates the Alloy configuration against available configs.
    pub fn validate(&self, available_configs: &HashMap<String, ConfigEntry>) -> Result<()> {
        if let Some(ref analyze) = self.analyze_config
            && !available_configs.contains_key(analyze)
        {
            return Err(CodexErr::Fatal(format!(
                "Alloy analyze config '{analyze}' does not exist"
            )));
        }
        if let Some(ref implement) = self.implement_config
            && !available_configs.contains_key(implement)
        {
            return Err(CodexErr::Fatal(format!(
                "Alloy implement config '{implement}' does not exist"
            )));
        }
        Ok(())
    }
}

/// The type of AI provider for a configuration entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    /// OpenAI API (supports ChatGPT login and API key)
    OpenAI,
    /// Anthropic Claude API
    Anthropic,
    /// Google Gemini API
    Google,
    /// New-API (Anthropic-compatible with custom endpoint)
    NewAPI,
}

impl ProviderType {
    /// Returns the display name for this provider type.
    pub fn display_name(&self) -> &'static str {
        match self {
            ProviderType::OpenAI => "OpenAI",
            ProviderType::Anthropic => "Anthropic Claude",
            ProviderType::Google => "Google Gemini",
            ProviderType::NewAPI => "New-API",
        }
    }

    /// Returns the default API endpoint for this provider.
    pub fn default_endpoint(&self) -> Option<&'static str> {
        match self {
            ProviderType::OpenAI => Some("https://api.openai.com/v1"),
            ProviderType::Anthropic => Some("https://api.anthropic.com"),
            ProviderType::Google => Some("https://generativelanguage.googleapis.com/v1beta"),
            ProviderType::NewAPI => None, // Must be specified by user
        }
    }

    /// Returns the default environment variable name for the API key.
    pub fn default_env_key(&self) -> &'static str {
        match self {
            ProviderType::OpenAI => "OPENAI_API_KEY",
            ProviderType::Anthropic => "ANTHROPIC_API_KEY",
            ProviderType::Google => "GOOGLE_API_KEY",
            ProviderType::NewAPI => "NEWAPI_API_KEY",
        }
    }

    /// Returns true if this provider requires a custom endpoint.
    pub fn requires_endpoint(&self) -> bool {
        matches!(self, ProviderType::NewAPI)
    }

    /// Returns the WireApi protocol for this provider type.
    pub fn wire_api(&self) -> WireApi {
        match self {
            ProviderType::OpenAI => WireApi::Responses,
            ProviderType::Anthropic => WireApi::Anthropic,
            ProviderType::Google => WireApi::Gemini,
            ProviderType::NewAPI => WireApi::Anthropic, // NewAPI uses Anthropic-compatible protocol
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// A single configuration entry for an AI provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigEntry {
    /// The type of provider (OpenAI, Anthropic, Google, NewAPI).
    pub provider_type: ProviderType,

    /// Custom API endpoint. Required for NewAPI, optional for others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Environment variable name for the API key.
    /// Defaults to provider-specific value (e.g., "ANTHROPIC_API_KEY").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_key: Option<String>,

    /// The current model ID for this configuration.
    pub model: String,

    /// Reasoning effort level (for Gemini: "low" or "high").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,

    /// Whether this configuration is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Whether to use ChatGPT account authentication (OpenAI only).
    /// When true, uses the logged-in ChatGPT account instead of an API key.
    #[serde(default)]
    pub use_account_auth: bool,

    /// Wire API format override for NewAPI providers.
    /// Supported values: "openai_chat", "anthropic", "gemini".
    /// If not specified, defaults based on provider_type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_api: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl ConfigEntry {
    /// Creates a new configuration entry with the given parameters.
    pub fn new(provider_type: ProviderType, model: String) -> Self {
        Self {
            provider_type,
            endpoint: None,
            env_key: None,
            model,
            reasoning_effort: None,
            enabled: true,
            use_account_auth: false,
            wire_api: None,
        }
    }

    /// Returns the effective API endpoint for this configuration.
    pub fn effective_endpoint(&self) -> Option<String> {
        self.endpoint
            .clone()
            .or_else(|| self.provider_type.default_endpoint().map(String::from))
    }

    /// Returns the effective environment variable name for the API key.
    pub fn effective_env_key(&self) -> String {
        self.env_key
            .clone()
            .unwrap_or_else(|| self.provider_type.default_env_key().to_string())
    }

    /// Attempts to read the API key from the environment.
    pub fn api_key(&self) -> Option<String> {
        let env_key = self.effective_env_key();
        std::env::var(&env_key)
            .ok()
            .filter(|v| !v.trim().is_empty())
    }

    /// Validates this configuration entry.
    pub fn validate(&self) -> Result<()> {
        // NewAPI requires an endpoint
        if self.provider_type.requires_endpoint() && self.endpoint.is_none() {
            return Err(CodexErr::Fatal(
                "NewAPI provider requires an endpoint to be specified".into(),
            ));
        }

        // Model must not be empty
        if self.model.trim().is_empty() {
            return Err(CodexErr::Fatal("Model name cannot be empty".into()));
        }

        Ok(())
    }

    /// Converts this configuration entry to a ModelProviderInfo.
    ///
    /// This allows the AnyCLI config to be used with the existing Codex
    /// model provider infrastructure.
    pub fn to_model_provider_info(&self, name: &str) -> ModelProviderInfo {
        let is_openai = matches!(self.provider_type, ProviderType::OpenAI);

        // For OpenAI with account auth, don't set env_key so it uses ChatGPT account
        let env_key = if is_openai && self.use_account_auth {
            None
        } else {
            Some(self.effective_env_key())
        };

        let base_url = if is_openai {
            // Mirror upstream OpenAI provider behavior:
            // - default to `None` so the runtime can choose ChatGPT vs API base URL
            //   depending on auth mode
            // - allow overriding via `OPENAI_BASE_URL` or explicit AnyCLI endpoint.
            self.endpoint.clone().or_else(|| {
                std::env::var("OPENAI_BASE_URL")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
            })
        } else {
            self.effective_endpoint()
        };

        let http_headers = is_openai.then(|| {
            [("version".to_string(), env!("CARGO_PKG_VERSION").to_string())]
                .into_iter()
                .collect()
        });
        let env_http_headers = is_openai.then(|| {
            [
                (
                    "OpenAI-Organization".to_string(),
                    "OPENAI_ORGANIZATION".to_string(),
                ),
                ("OpenAI-Project".to_string(), "OPENAI_PROJECT".to_string()),
            ]
            .into_iter()
            .collect()
        });

        // For OpenAI: requires_openai_auth is true if using account auth
        // (otherwise it will use the API key from env_key)
        let requires_openai_auth = is_openai && self.use_account_auth;

        // Determine wire API: use explicit override if provided, otherwise default for provider
        let wire_api = self
            .wire_api
            .as_ref()
            .and_then(|w| match w.as_str() {
                "openai_chat" | "chat" => Some(WireApi::Chat),
                "openai_responses" | "responses" => Some(WireApi::Responses),
                "anthropic" => Some(WireApi::Anthropic),
                "gemini" | "google" => Some(WireApi::Gemini),
                _ => None,
            })
            .unwrap_or_else(|| self.provider_type.wire_api());

        ModelProviderInfo {
            name: name.to_string(),
            base_url,
            env_key,
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api,
            query_params: None,
            http_headers,
            env_http_headers,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth,
        }
    }
}

/// Root configuration structure for AnyCLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AnycliConfig {
    /// The name of the currently active configuration.
    pub active_config: String,

    /// Map of configuration names to their entries.
    #[serde(default)]
    pub configs: HashMap<String, ConfigEntry>,

    /// The current agent execution mode (Classic or Alloy).
    #[serde(default)]
    pub agent_mode: AgentMode,

    /// Alloy Agent specific configuration (analyze + implement models).
    #[serde(default)]
    pub alloy: AlloyConfig,
}

impl AnycliConfig {
    /// Creates a new empty configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads the configuration from the default AnyCLI config path.
    pub fn load() -> Result<Self> {
        let path = super::anycli_config_path()
            .ok_or_else(|| CodexErr::Fatal("Could not determine AnyCLI config path".into()))?;

        Self::load_from_path(&path)
    }

    /// Loads the configuration from a specific path.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| CodexErr::Fatal(format!("Failed to read config file: {e}")))?;

        let config: Self = toml::from_str(&content)
            .map_err(|e| CodexErr::Fatal(format!("Failed to parse config file: {e}")))?;

        Ok(config)
    }

    /// Saves the configuration to the default AnyCLI config path.
    pub fn save(&self) -> Result<()> {
        let path = super::anycli_config_path()
            .ok_or_else(|| CodexErr::Fatal("Could not determine AnyCLI config path".into()))?;

        self.save_to_path(&path)
    }

    /// Saves the configuration to a specific path.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CodexErr::Fatal(format!("Failed to create config directory: {e}")))?;
        }

        let content = toml::to_string_pretty(self)
            .map_err(|e| CodexErr::Fatal(format!("Failed to serialize config: {e}")))?;

        std::fs::write(path, content)
            .map_err(|e| CodexErr::Fatal(format!("Failed to write config file: {e}")))?;

        Ok(())
    }

    /// Returns the currently active configuration entry.
    pub fn active_entry(&self) -> Option<&ConfigEntry> {
        self.configs.get(&self.active_config)
    }

    /// Returns a mutable reference to the currently active configuration entry.
    pub fn active_entry_mut(&mut self) -> Option<&mut ConfigEntry> {
        self.configs.get_mut(&self.active_config)
    }

    /// Adds a new configuration entry.
    pub fn add_config(&mut self, name: String, entry: ConfigEntry) {
        self.configs.insert(name, entry);
    }

    /// Removes a configuration entry.
    pub fn remove_config(&mut self, name: &str) -> Option<ConfigEntry> {
        self.configs.remove(name)
    }

    /// Sets the active configuration by name.
    pub fn set_active(&mut self, name: &str) -> Result<()> {
        if !self.configs.contains_key(name) {
            return Err(CodexErr::Fatal(format!("Configuration '{name}' not found")));
        }
        self.active_config = name.to_string();
        Ok(())
    }

    /// Returns a list of all configuration names.
    pub fn config_names(&self) -> Vec<&str> {
        self.configs
            .keys()
            .map(std::string::String::as_str)
            .collect()
    }

    /// Returns true if this configuration is empty (no configs defined).
    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    /// Returns true if Alloy Agent mode is active.
    pub fn is_alloy_mode(&self) -> bool {
        self.agent_mode == AgentMode::Alloy
    }

    /// Sets the agent execution mode.
    pub fn set_agent_mode(&mut self, mode: AgentMode) {
        self.agent_mode = mode;
    }

    /// Sets the Alloy Agent configuration.
    pub fn set_alloy_config(&mut self, analyze_config: String, implement_config: String) {
        self.alloy.analyze_config = Some(analyze_config);
        self.alloy.implement_config = Some(implement_config);
    }

    /// Returns the Alloy analyze configuration entry, if configured.
    pub fn alloy_analyze_entry(&self) -> Option<&ConfigEntry> {
        self.alloy
            .analyze_config
            .as_ref()
            .and_then(|name| self.configs.get(name))
    }

    /// Returns the Alloy implement configuration entry, if configured.
    pub fn alloy_implement_entry(&self) -> Option<&ConfigEntry> {
        self.alloy
            .implement_config
            .as_ref()
            .and_then(|name| self.configs.get(name))
    }

    /// Validates all configuration entries.
    pub fn validate(&self) -> Result<()> {
        for (name, entry) in &self.configs {
            entry
                .validate()
                .map_err(|e| CodexErr::Fatal(format!("Invalid config '{name}': {e}")))?;
        }

        // Ensure active_config references a valid entry (if not empty)
        if !self.configs.is_empty() && !self.configs.contains_key(&self.active_config) {
            return Err(CodexErr::Fatal(format!(
                "Active config '{}' does not exist",
                self.active_config
            )));
        }

        // Validate Alloy configuration if in Alloy mode
        if self.agent_mode == AgentMode::Alloy {
            self.alloy.validate(&self.configs)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_provider_type_display() {
        assert_eq!(ProviderType::OpenAI.display_name(), "OpenAI");
        assert_eq!(ProviderType::Anthropic.display_name(), "Anthropic Claude");
        assert_eq!(ProviderType::Google.display_name(), "Google Gemini");
        assert_eq!(ProviderType::NewAPI.display_name(), "New-API");
    }

    #[test]
    fn test_provider_type_default_endpoint() {
        assert!(ProviderType::OpenAI.default_endpoint().is_some());
        assert!(ProviderType::Anthropic.default_endpoint().is_some());
        assert!(ProviderType::Google.default_endpoint().is_some());
        assert!(ProviderType::NewAPI.default_endpoint().is_none());
    }

    #[test]
    fn test_config_entry_validation() {
        // Valid OpenAI config
        let entry = ConfigEntry::new(ProviderType::OpenAI, "gpt-4".to_string());
        assert!(entry.validate().is_ok());

        // Valid Anthropic config
        let entry = ConfigEntry::new(
            ProviderType::Anthropic,
            "claude-sonnet-4-5-20250929".to_string(),
        );
        assert!(entry.validate().is_ok());

        // Invalid NewAPI config (missing endpoint)
        let entry = ConfigEntry::new(ProviderType::NewAPI, "custom-model".to_string());
        assert!(entry.validate().is_err());

        // Valid NewAPI config (with endpoint)
        let mut entry = ConfigEntry::new(ProviderType::NewAPI, "custom-model".to_string());
        entry.endpoint = Some("https://api.newapi.cc".to_string());
        assert!(entry.validate().is_ok());

        // Invalid config (empty model)
        let entry = ConfigEntry::new(ProviderType::OpenAI, "".to_string());
        assert!(entry.validate().is_err());
    }

    #[test]
    fn test_anycli_config_serialization() {
        let mut config = AnycliConfig::new();

        let openai_entry = ConfigEntry::new(ProviderType::OpenAI, "gpt-4".to_string());
        config.add_config("openai-default".to_string(), openai_entry);

        let mut claude_entry = ConfigEntry::new(
            ProviderType::Anthropic,
            "claude-sonnet-4-5-20250929".to_string(),
        );
        claude_entry.env_key = Some("ANTHROPIC_API_KEY".to_string());
        config.add_config("claude-work".to_string(), claude_entry);

        config.active_config = "claude-work".to_string();

        let toml_str = toml::to_string_pretty(&config).expect("serialization failed");
        let parsed: AnycliConfig = toml::from_str(&toml_str).expect("deserialization failed");

        assert_eq!(parsed.active_config, "claude-work");
        assert_eq!(parsed.configs.len(), 2);
        assert!(parsed.configs.contains_key("openai-default"));
        assert!(parsed.configs.contains_key("claude-work"));
    }

    #[test]
    fn test_config_toml_format() {
        let toml_str = r#"
active_config = "claude-work"

[configs.openai-default]
provider_type = "openai"
model = "gpt-5.1-codex-max"

[configs.claude-work]
provider_type = "anthropic"
model = "claude-sonnet-4-5-20250929"
env_key = "ANTHROPIC_API_KEY"

[configs.gemini-pro]
provider_type = "google"
model = "gemini-3-pro-preview"
reasoning_effort = "high"

[configs.newapi-custom]
provider_type = "newapi"
endpoint = "https://api.newapi.cc"
model = "custom-model-name"
"#;

        let config: AnycliConfig = toml::from_str(toml_str).expect("deserialization failed");

        assert_eq!(config.active_config, "claude-work");
        assert_eq!(config.configs.len(), 4);

        let openai = config.configs.get("openai-default").unwrap();
        assert_eq!(openai.provider_type, ProviderType::OpenAI);
        assert_eq!(openai.model, "gpt-5.1-codex-max");

        let claude = config.configs.get("claude-work").unwrap();
        assert_eq!(claude.provider_type, ProviderType::Anthropic);
        assert_eq!(claude.env_key, Some("ANTHROPIC_API_KEY".to_string()));

        let gemini = config.configs.get("gemini-pro").unwrap();
        assert_eq!(gemini.provider_type, ProviderType::Google);
        assert_eq!(gemini.reasoning_effort, Some("high".to_string()));

        let newapi = config.configs.get("newapi-custom").unwrap();
        assert_eq!(newapi.provider_type, ProviderType::NewAPI);
        assert_eq!(newapi.endpoint, Some("https://api.newapi.cc".to_string()));
    }
}
