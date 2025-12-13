//! Model registry for AnyCLI providers.
//!
//! This module provides model information for each supported provider,
//! including model IDs, display names, and default selections.

use super::config::ProviderType;

/// Information about a model available from a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// The model ID used in API requests.
    pub id: &'static str,
    /// Human-readable display name.
    pub name: &'static str,
    /// Brief description of the model.
    pub description: &'static str,
    /// Whether this is the default model for the provider.
    pub default: bool,
}

impl ModelInfo {
    const fn new(
        id: &'static str,
        name: &'static str,
        description: &'static str,
        default: bool,
    ) -> Self {
        Self {
            id,
            name,
            description,
            default,
        }
    }
}

/// Returns the available models for a given provider type.
pub fn models_for_provider(provider: ProviderType) -> Vec<ModelInfo> {
    match provider {
        ProviderType::OpenAI => openai_models(),
        ProviderType::Anthropic => anthropic_models(),
        ProviderType::Google => google_models(),
        ProviderType::NewAPI => vec![], // Manual entry required
    }
}

/// Returns the default model ID for a given provider type.
pub fn default_model_for_provider(provider: ProviderType) -> Option<&'static str> {
    models_for_provider(provider)
        .iter()
        .find(|m| m.default)
        .map(|m| m.id)
}

/// OpenAI models.
fn openai_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo::new(
            "gpt-5.1-codex-max",
            "gpt-5.1-codex-max",
            "Latest Codex-optimized flagship for deep and fast reasoning.",
            true,
        ),
        ModelInfo::new(
            "gpt-5.1-codex",
            "gpt-5.1-codex",
            "Optimized for codex.",
            false,
        ),
        ModelInfo::new(
            "gpt-5.1-codex-mini",
            "gpt-5.1-codex-mini",
            "Optimized for codex. Cheaper, faster, but less capable.",
            false,
        ),
        ModelInfo::new(
            "gpt-5.2",
            "gpt-5.2",
            "Latest frontier model with improvements across knowledge, reasoning and coding",
            false,
        ),
    ]
}

/// Anthropic Claude models.
fn anthropic_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo::new(
            "claude-sonnet-4-5-20250929",
            "Claude Sonnet 4.5",
            "Best model for real-world agents and coding",
            true,
        ),
        ModelInfo::new(
            "claude-opus-4-5-20251101",
            "Claude Opus 4.5",
            "Premium model with maximum intelligence",
            false,
        ),
        ModelInfo::new(
            "claude-haiku-4-5-20251001",
            "Claude Haiku 4.5",
            "Fastest model for near-instant responses",
            false,
        ),
    ]
}

/// Google Gemini models.
fn google_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo::new(
            "gemini-3-pro-preview",
            "Gemini 3 Pro",
            "Latest generation with advanced thinking",
            true,
        ),
        ModelInfo::new(
            "gemini-2.5-flash",
            "Gemini 2.5 Flash",
            "Fast model with thinking enabled",
            false,
        ),
        ModelInfo::new(
            "gemini-2.5-pro",
            "Gemini 2.5 Pro",
            "High capability multimodal model",
            false,
        ),
    ]
}

/// Reasoning effort levels for providers that support them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEffort {
    Low,
    High,
}

impl ReasoningEffort {
    /// Returns the API value for this reasoning effort.
    pub fn api_value(&self) -> &'static str {
        match self {
            ReasoningEffort::Low => "low",
            ReasoningEffort::High => "high",
        }
    }

    /// Returns the display name for this reasoning effort.
    pub fn display_name(&self) -> &'static str {
        match self {
            ReasoningEffort::Low => "Low",
            ReasoningEffort::High => "High",
        }
    }

    /// Returns the description for this reasoning effort.
    pub fn description(&self) -> &'static str {
        match self {
            ReasoningEffort::Low => "Faster responses, less deep thinking",
            ReasoningEffort::High => "Deeper reasoning, more thorough analysis",
        }
    }

    /// Parse from a string value.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "low" => Some(ReasoningEffort::Low),
            "high" => Some(ReasoningEffort::High),
            _ => None,
        }
    }
}

/// Returns whether a provider supports reasoning effort configuration.
pub fn supports_reasoning_effort(provider: ProviderType) -> bool {
    matches!(provider, ProviderType::Google)
}

/// Returns the available reasoning efforts for a provider.
pub fn reasoning_efforts_for_provider(provider: ProviderType) -> Vec<ReasoningEffort> {
    if supports_reasoning_effort(provider) {
        vec![ReasoningEffort::Low, ReasoningEffort::High]
    } else {
        vec![]
    }
}

/// Returns the default reasoning effort for a provider.
pub fn default_reasoning_effort(provider: ProviderType) -> Option<ReasoningEffort> {
    if supports_reasoning_effort(provider) {
        Some(ReasoningEffort::High)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_openai_models() {
        let models = models_for_provider(ProviderType::OpenAI);
        assert!(!models.is_empty());

        // Check default model
        let default = models.iter().find(|m| m.default);
        assert!(default.is_some());
        assert_eq!(default.unwrap().id, "gpt-5.1-codex-max");
    }

    #[test]
    fn test_anthropic_models() {
        let models = models_for_provider(ProviderType::Anthropic);
        assert_eq!(models.len(), 3);

        // Check default model
        let default = models.iter().find(|m| m.default);
        assert!(default.is_some());
        assert_eq!(default.unwrap().id, "claude-sonnet-4-5-20250929");

        // Check all expected models are present
        let ids: Vec<_> = models.iter().map(|m| m.id).collect();
        assert!(ids.contains(&"claude-sonnet-4-5-20250929"));
        assert!(ids.contains(&"claude-opus-4-5-20251101"));
        assert!(ids.contains(&"claude-haiku-4-5-20251001"));
    }

    #[test]
    fn test_google_models() {
        let models = models_for_provider(ProviderType::Google);
        assert!(!models.is_empty());

        // Check default model
        let default = models.iter().find(|m| m.default);
        assert!(default.is_some());
        assert_eq!(default.unwrap().id, "gemini-3-pro-preview");
    }

    #[test]
    fn test_newapi_models_empty() {
        let models = models_for_provider(ProviderType::NewAPI);
        assert!(models.is_empty(), "NewAPI should have no predefined models");
    }

    #[test]
    fn test_default_model_for_provider() {
        assert_eq!(
            default_model_for_provider(ProviderType::OpenAI),
            Some("gpt-5.1-codex-max")
        );
        assert_eq!(
            default_model_for_provider(ProviderType::Anthropic),
            Some("claude-sonnet-4-5-20250929")
        );
        assert_eq!(
            default_model_for_provider(ProviderType::Google),
            Some("gemini-3-pro-preview")
        );
        assert_eq!(default_model_for_provider(ProviderType::NewAPI), None);
    }

    #[test]
    fn test_reasoning_effort() {
        // Google supports reasoning effort
        assert!(supports_reasoning_effort(ProviderType::Google));
        let efforts = reasoning_efforts_for_provider(ProviderType::Google);
        assert_eq!(efforts.len(), 2);

        // Others don't
        assert!(!supports_reasoning_effort(ProviderType::OpenAI));
        assert!(!supports_reasoning_effort(ProviderType::Anthropic));
        assert!(!supports_reasoning_effort(ProviderType::NewAPI));
    }

    #[test]
    fn test_reasoning_effort_parsing() {
        assert_eq!(ReasoningEffort::from_str("low"), Some(ReasoningEffort::Low));
        assert_eq!(
            ReasoningEffort::from_str("HIGH"),
            Some(ReasoningEffort::High)
        );
        assert_eq!(ReasoningEffort::from_str("invalid"), None);
    }
}
