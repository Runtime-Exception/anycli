//! AnyCLI configuration setup widget for first-run onboarding.
//!
//! This widget guides users through creating their first AnyCLI configuration.

use codex_core::anycli::config::AnycliConfig;
use codex_core::anycli::config::ConfigEntry;
use codex_core::anycli::config::ProviderType;
use codex_core::anycli::models::models_for_provider;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepState;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::tui::FrameRequester;

#[derive(Clone)]
pub(crate) enum ConfigSetupState {
    PickProvider,
    EnterName,
    EnterEndpoint,
    EnterApiKey,
    SelectModel,
    Confirmation,
    Complete,
}

#[derive(Clone)]
pub(crate) struct ConfigSetupWidget {
    pub request_frame: FrameRequester,
    pub state: ConfigSetupState,
    pub error: Option<String>,
    // Provider selection
    pub provider_index: usize,
    pub providers: Vec<(ProviderType, &'static str)>,
    // Config name
    pub config_name: String,
    // Endpoint (for New-API)
    pub endpoint: String,
    // API key / env var
    pub api_key: String,
    pub use_env_var: bool,
    pub env_var_name: String,
    // Model selection
    pub model_index: usize,
    pub available_models: Vec<String>,
}

impl ConfigSetupWidget {
    pub fn new(request_frame: FrameRequester) -> Self {
        let providers = vec![
            (ProviderType::OpenAI, "OpenAI"),
            (ProviderType::Anthropic, "Anthropic Claude"),
            (ProviderType::Google, "Google Gemini"),
            (ProviderType::NewAPI, "New-API (Custom)"),
        ];

        Self {
            request_frame,
            state: ConfigSetupState::PickProvider,
            error: None,
            provider_index: 0,
            providers,
            config_name: String::new(),
            endpoint: String::new(),
            api_key: String::new(),
            use_env_var: true,
            env_var_name: String::new(),
            model_index: 0,
            available_models: Vec::new(),
        }
    }

    fn selected_provider(&self) -> ProviderType {
        self.providers[self.provider_index].0.clone()
    }

    fn default_env_var_for_provider(&self, provider: &ProviderType) -> String {
        match provider {
            ProviderType::OpenAI => "OPENAI_API_KEY".to_string(),
            ProviderType::Anthropic => "ANTHROPIC_API_KEY".to_string(),
            ProviderType::Google => "GOOGLE_API_KEY".to_string(),
            ProviderType::NewAPI => "NEWAPI_API_KEY".to_string(),
        }
    }

    fn default_config_name(&self) -> String {
        let provider = self.selected_provider();
        match provider {
            ProviderType::OpenAI => "openai-default".to_string(),
            ProviderType::Anthropic => "anthropic-default".to_string(),
            ProviderType::Google => "gemini-default".to_string(),
            ProviderType::NewAPI => "newapi-custom".to_string(),
        }
    }

    fn move_to_next_state(&mut self) {
        let provider = self.selected_provider();
        self.state = match &self.state {
            ConfigSetupState::PickProvider => {
                // Set default config name and env var
                if self.config_name.is_empty() {
                    self.config_name = self.default_config_name();
                }
                if self.env_var_name.is_empty() {
                    self.env_var_name = self.default_env_var_for_provider(&provider);
                }
                ConfigSetupState::EnterName
            }
            ConfigSetupState::EnterName => {
                if matches!(provider, ProviderType::NewAPI) {
                    ConfigSetupState::EnterEndpoint
                } else {
                    ConfigSetupState::EnterApiKey
                }
            }
            ConfigSetupState::EnterEndpoint => ConfigSetupState::EnterApiKey,
            ConfigSetupState::EnterApiKey => {
                // Load models for the selected provider
                self.available_models = models_for_provider(provider.clone())
                    .into_iter()
                    .map(|m| m.id.to_string())
                    .collect();
                if self.available_models.is_empty() {
                    // For New-API, we may need manual entry
                    self.available_models = vec!["custom-model".to_string()];
                }
                ConfigSetupState::SelectModel
            }
            ConfigSetupState::SelectModel => ConfigSetupState::Confirmation,
            ConfigSetupState::Confirmation => {
                // Save the configuration
                self.save_config();
                ConfigSetupState::Complete
            }
            ConfigSetupState::Complete => ConfigSetupState::Complete,
        };
        self.error = None;
    }

    fn move_to_prev_state(&mut self) {
        let provider = self.selected_provider();
        self.state = match &self.state {
            ConfigSetupState::PickProvider => ConfigSetupState::PickProvider,
            ConfigSetupState::EnterName => ConfigSetupState::PickProvider,
            ConfigSetupState::EnterEndpoint => ConfigSetupState::EnterName,
            ConfigSetupState::EnterApiKey => {
                if matches!(provider, ProviderType::NewAPI) {
                    ConfigSetupState::EnterEndpoint
                } else {
                    ConfigSetupState::EnterName
                }
            }
            ConfigSetupState::SelectModel => ConfigSetupState::EnterApiKey,
            ConfigSetupState::Confirmation => ConfigSetupState::SelectModel,
            ConfigSetupState::Complete => ConfigSetupState::Complete,
        };
        self.error = None;
    }

    fn save_config(&mut self) {
        let provider = self.selected_provider();
        let model = self
            .available_models
            .get(self.model_index)
            .cloned()
            .unwrap_or_else(|| "custom-model".to_string());

        let entry = ConfigEntry {
            provider_type: provider.clone(),
            endpoint: if matches!(provider, ProviderType::NewAPI) {
                Some(self.endpoint.clone())
            } else {
                None
            },
            env_key: Some(if self.use_env_var {
                self.env_var_name.clone()
            } else {
                // Store API key directly (not recommended but supported)
                format!("direct:{}", self.api_key)
            }),
            model,
            reasoning_effort: None,
            enabled: true,
        };

        let mut config = AnycliConfig::load().unwrap_or_default();
        config.configs.insert(self.config_name.clone(), entry);
        config.active_config = self.config_name.clone();

        if let Err(e) = config.save() {
            self.error = Some(format!("Failed to save config: {}", e));
        }
    }

    fn handle_text_input(&mut self, key_event: &KeyEvent, text: &mut String) -> bool {
        match key_event {
            KeyEvent {
                code: KeyCode::Char(c),
                kind: KeyEventKind::Press,
                modifiers,
                ..
            } if !modifiers.contains(KeyModifiers::CONTROL) => {
                text.push(*c);
                true
            }
            KeyEvent {
                code: KeyCode::Backspace,
                kind: KeyEventKind::Press,
                ..
            } => {
                text.pop();
                true
            }
            _ => false,
        }
    }
}

impl KeyboardHandler for ConfigSetupWidget {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match &self.state {
            ConfigSetupState::PickProvider => match key_event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.provider_index > 0 {
                        self.provider_index -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.provider_index < self.providers.len() - 1 {
                        self.provider_index += 1;
                    }
                }
                KeyCode::Enter => {
                    self.move_to_next_state();
                }
                _ => {}
            },
            ConfigSetupState::EnterName => {
                let mut text = self.config_name.clone();
                if self.handle_text_input(&key_event, &mut text) {
                    self.config_name = text;
                } else if key_event.code == KeyCode::Enter && !self.config_name.is_empty() {
                    self.move_to_next_state();
                } else if key_event.code == KeyCode::Esc {
                    self.move_to_prev_state();
                }
            }
            ConfigSetupState::EnterEndpoint => {
                let mut text = self.endpoint.clone();
                if self.handle_text_input(&key_event, &mut text) {
                    self.endpoint = text;
                } else if key_event.code == KeyCode::Enter && !self.endpoint.is_empty() {
                    self.move_to_next_state();
                } else if key_event.code == KeyCode::Esc {
                    self.move_to_prev_state();
                }
            }
            ConfigSetupState::EnterApiKey => match key_event.code {
                KeyCode::Tab => {
                    self.use_env_var = !self.use_env_var;
                }
                KeyCode::Enter => {
                    let has_value = if self.use_env_var {
                        !self.env_var_name.is_empty()
                    } else {
                        !self.api_key.is_empty()
                    };
                    if has_value {
                        self.move_to_next_state();
                    }
                }
                KeyCode::Esc => {
                    self.move_to_prev_state();
                }
                _ => {
                    if self.use_env_var {
                        let mut text = self.env_var_name.clone();
                        if self.handle_text_input(&key_event, &mut text) {
                            self.env_var_name = text;
                        }
                    } else {
                        let mut text = self.api_key.clone();
                        if self.handle_text_input(&key_event, &mut text) {
                            self.api_key = text;
                        }
                    }
                }
            },
            ConfigSetupState::SelectModel => match key_event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.model_index > 0 {
                        self.model_index -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.model_index < self.available_models.len().saturating_sub(1) {
                        self.model_index += 1;
                    }
                }
                KeyCode::Enter => {
                    self.move_to_next_state();
                }
                KeyCode::Esc => {
                    self.move_to_prev_state();
                }
                _ => {}
            },
            ConfigSetupState::Confirmation => match key_event.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.move_to_next_state();
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.move_to_prev_state();
                }
                _ => {}
            },
            ConfigSetupState::Complete => {}
        }
        self.request_frame.schedule_frame();
    }

    fn handle_paste(&mut self, pasted: String) {
        match &self.state {
            ConfigSetupState::EnterName => {
                self.config_name.push_str(&pasted);
            }
            ConfigSetupState::EnterEndpoint => {
                self.endpoint.push_str(&pasted);
            }
            ConfigSetupState::EnterApiKey => {
                if self.use_env_var {
                    self.env_var_name.push_str(&pasted);
                } else {
                    self.api_key.push_str(&pasted);
                }
            }
            _ => {}
        }
        self.request_frame.schedule_frame();
    }
}

impl StepStateProvider for ConfigSetupWidget {
    fn get_step_state(&self) -> StepState {
        match self.state {
            ConfigSetupState::Complete => StepState::Complete,
            _ => StepState::InProgress,
        }
    }
}

impl WidgetRef for ConfigSetupWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Title
        lines.push(Line::from("").into());
        lines.push(Line::from("AnyCLI Configuration Setup").bold().cyan());
        lines.push(Line::from("").into());

        match &self.state {
            ConfigSetupState::PickProvider => {
                lines.push(Line::from("Select your AI provider:").into());
                lines.push(Line::from("").into());

                for (i, (_, name)) in self.providers.iter().enumerate() {
                    let prefix = if i == self.provider_index {
                        "› "
                    } else {
                        "  "
                    };
                    let line = format!("{}{}", prefix, name);
                    if i == self.provider_index {
                        lines.push(Line::from(line).cyan().bold());
                    } else {
                        lines.push(Line::from(line).into());
                    }
                }

                lines.push(Line::from("").into());
                lines.push(Line::from("Use ↑/↓ to navigate, Enter to select").dim());
            }
            ConfigSetupState::EnterName => {
                lines.push(Line::from("Enter a name for this configuration:").into());
                lines.push(Line::from("").into());
                let display = if self.config_name.is_empty() {
                    "_".to_string()
                } else {
                    format!("{}_", self.config_name)
                };
                lines.push(Line::from(display).cyan());
                lines.push(Line::from("").into());
                lines.push(Line::from("Press Enter to continue, Esc to go back").dim());
            }
            ConfigSetupState::EnterEndpoint => {
                lines.push(Line::from("Enter the API endpoint URL:").into());
                lines.push(Line::from("(e.g., https://api.newapi.cc)").dim());
                lines.push(Line::from("").into());
                let display = if self.endpoint.is_empty() {
                    "https://_".to_string()
                } else {
                    format!("{}_", self.endpoint)
                };
                lines.push(Line::from(display).cyan());
                lines.push(Line::from("").into());
                lines.push(Line::from("Press Enter to continue, Esc to go back").dim());
            }
            ConfigSetupState::EnterApiKey => {
                lines.push(Line::from("Configure API authentication:").into());
                lines.push(Line::from("").into());

                let env_prefix = if self.use_env_var { "› " } else { "  " };
                let direct_prefix = if !self.use_env_var { "› " } else { "  " };

                let env_line = format!("{}Environment variable: {}", env_prefix, self.env_var_name);
                let direct_line = format!(
                    "{}Direct API key: {}",
                    direct_prefix,
                    "*".repeat(self.api_key.len().min(20))
                );

                if self.use_env_var {
                    lines.push(Line::from(env_line).cyan());
                    lines.push(Line::from(direct_line).dim());
                } else {
                    lines.push(Line::from(env_line).dim());
                    lines.push(Line::from(direct_line).cyan());
                }

                lines.push(Line::from("").into());
                lines.push(
                    Line::from("Press Tab to switch, Enter to continue, Esc to go back").dim(),
                );
            }
            ConfigSetupState::SelectModel => {
                lines.push(Line::from("Select a model:").into());
                lines.push(Line::from("").into());

                for (i, model) in self.available_models.iter().enumerate() {
                    let prefix = if i == self.model_index { "› " } else { "  " };
                    let line = format!("{}{}", prefix, model);
                    if i == self.model_index {
                        lines.push(Line::from(line).cyan().bold());
                    } else {
                        lines.push(Line::from(line).into());
                    }
                }

                lines.push(Line::from("").into());
                lines
                    .push(Line::from("Use ↑/↓ to navigate, Enter to select, Esc to go back").dim());
            }
            ConfigSetupState::Confirmation => {
                let provider = self.selected_provider();
                let provider_name = match provider {
                    ProviderType::OpenAI => "OpenAI",
                    ProviderType::Anthropic => "Anthropic",
                    ProviderType::Google => "Google",
                    ProviderType::NewAPI => "New-API",
                };
                let model = self
                    .available_models
                    .get(self.model_index)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());

                lines.push(Line::from("Review your configuration:").into());
                lines.push(Line::from("").into());
                lines.push(Line::from(format!("  Name: {}", self.config_name)).into());
                lines.push(Line::from(format!("  Provider: {}", provider_name)).into());
                if matches!(provider, ProviderType::NewAPI) {
                    lines.push(Line::from(format!("  Endpoint: {}", self.endpoint)).into());
                }
                if self.use_env_var {
                    lines.push(Line::from(format!("  Auth: ${}", self.env_var_name)).into());
                } else {
                    lines.push(Line::from("  Auth: Direct API key").into());
                }
                lines.push(Line::from(format!("  Model: {}", model)).into());
                lines.push(Line::from("").into());
                lines.push(Line::from("Press Enter or Y to save, Esc or N to go back").dim());
            }
            ConfigSetupState::Complete => {
                lines.push(Line::from("Configuration saved!").green().bold());
                lines.push(Line::from("").into());
                lines
                    .push(Line::from(format!("Active configuration: {}", self.config_name)).into());
                lines.push(Line::from("").into());
                lines.push(Line::from("Press any key to continue...").dim());
            }
        }

        // Show error if any
        if let Some(ref error) = self.error {
            lines.push(Line::from("").into());
            lines.push(Line::from(format!("Error: {}", error)).fg(Color::Red));
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}
