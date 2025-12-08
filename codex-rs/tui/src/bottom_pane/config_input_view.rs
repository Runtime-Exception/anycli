//! Config input view for creating AnyCLI provider configurations.
//!
//! This view provides a multi-step text input interface for collecting
//! configuration details like API keys and endpoints.

use std::cell::RefCell;

use codex_core::anycli::config::ProviderType;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::StatefulWidgetRef;
use ratatui::widgets::Widget;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::render::renderable::Renderable;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;
use super::popup_consts::standard_popup_hint_line;
use super::textarea::TextArea;
use super::textarea::TextAreaState;

/// The current step in the config creation flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigInputStep {
    /// Enter the configuration name.
    ConfigName,
    /// Enter the API key.
    ApiKey,
    /// Enter the endpoint URL (optional for most, required for NewAPI).
    Endpoint,
    /// Enter the model name (for NewAPI only).
    ModelName,
}

/// Input view for collecting AnyCLI configuration details.
pub(crate) struct ConfigInputView {
    provider: ProviderType,
    app_event_tx: AppEventSender,

    // Current step
    step: ConfigInputStep,

    // Collected values
    config_name: String,
    api_key: String,
    endpoint: String,

    // UI state
    textarea: TextArea,
    textarea_state: RefCell<TextAreaState>,
    complete: bool,
}

impl ConfigInputView {
    pub(crate) fn new(provider: ProviderType, app_event_tx: AppEventSender) -> Self {
        let default_name = match &provider {
            ProviderType::OpenAI => "openai",
            ProviderType::Anthropic => "anthropic",
            ProviderType::Google => "gemini",
            ProviderType::NewAPI => "newapi",
        };

        let mut textarea = TextArea::new();
        textarea.set_text(default_name);

        Self {
            provider,
            app_event_tx,
            step: ConfigInputStep::ConfigName,
            config_name: default_name.to_string(),
            api_key: String::new(),
            endpoint: String::new(),
            textarea,
            textarea_state: RefCell::new(TextAreaState::default()),
            complete: false,
        }
    }

    fn title(&self) -> &'static str {
        match self.step {
            ConfigInputStep::ConfigName => "Enter configuration name:",
            ConfigInputStep::ApiKey => "Enter API key:",
            ConfigInputStep::Endpoint => match self.provider {
                ProviderType::NewAPI => "Enter API endpoint URL (required):",
                _ => "Enter API endpoint URL (optional, press Enter to skip):",
            },
            ConfigInputStep::ModelName => "Enter model name:",
        }
    }

    fn placeholder(&self) -> &'static str {
        match self.step {
            ConfigInputStep::ConfigName => "e.g., my-anthropic-config",
            ConfigInputStep::ApiKey => "sk-... or your API key",
            ConfigInputStep::Endpoint => "https://api.example.com",
            ConfigInputStep::ModelName => "e.g., claude-sonnet-4-5-20250929",
        }
    }

    fn provider_name(&self) -> &'static str {
        match self.provider {
            ProviderType::OpenAI => "OpenAI",
            ProviderType::Anthropic => "Anthropic",
            ProviderType::Google => "Google Gemini",
            ProviderType::NewAPI => "New-API",
        }
    }

    fn advance_step(&mut self) {
        // Save current value
        let current_text = self.textarea.text().trim().to_string();

        match self.step {
            ConfigInputStep::ConfigName => {
                if current_text.is_empty() {
                    return; // Name is required
                }
                self.config_name = current_text;
                self.step = ConfigInputStep::ApiKey;
                self.textarea = TextArea::new();
                self.textarea_state = RefCell::new(TextAreaState::default());
            }
            ConfigInputStep::ApiKey => {
                if current_text.is_empty() {
                    return; // API key is required
                }
                self.api_key = current_text;
                self.step = ConfigInputStep::Endpoint;
                self.textarea = TextArea::new();
                self.textarea_state = RefCell::new(TextAreaState::default());
            }
            ConfigInputStep::Endpoint => {
                // Endpoint is required for NewAPI, optional for others
                if matches!(self.provider, ProviderType::NewAPI) && current_text.is_empty() {
                    return; // Required for NewAPI
                }
                self.endpoint = current_text;

                // For NewAPI, we need model name input
                if matches!(self.provider, ProviderType::NewAPI) {
                    self.step = ConfigInputStep::ModelName;
                    self.textarea = TextArea::new();
                    self.textarea_state = RefCell::new(TextAreaState::default());
                } else {
                    // For other providers, go to model selection popup
                    self.save_config_with_model_selection();
                }
            }
            ConfigInputStep::ModelName => {
                if current_text.is_empty() {
                    return; // Model name is required for NewAPI
                }
                self.save_config_with_model(current_text);
            }
        }
    }

    fn save_config_with_model_selection(&mut self) {
        // Send event to open model selection popup
        self.app_event_tx
            .send(AppEvent::OpenConfigModelSelectWithKey {
                provider: self.provider.clone(),
                config_name: self.config_name.clone(),
                api_key: self.api_key.clone(),
                endpoint: if self.endpoint.is_empty() {
                    None
                } else {
                    Some(self.endpoint.clone())
                },
            });
        self.complete = true;
    }

    fn save_config_with_model(&mut self, model: String) {
        self.app_event_tx
            .send(AppEvent::SaveNewAnycliConfigComplete {
                provider: self.provider.clone(),
                config_name: self.config_name.clone(),
                api_key: self.api_key.clone(),
                endpoint: if self.endpoint.is_empty() {
                    None
                } else {
                    Some(self.endpoint.clone())
                },
                model,
            });
        self.complete = true;
    }

    fn input_height(&self, _width: u16) -> u16 {
        3 // Title + input + blank line
    }

    /// Returns the display text for the current input, masking API keys.
    fn display_text(&self) -> String {
        let text = self.textarea.text();
        if matches!(self.step, ConfigInputStep::ApiKey) && !text.is_empty() {
            // Mask API key, showing only last 4 chars
            let len = text.len();
            if len > 4 {
                format!("{}...{}", "*".repeat(len.min(8) - 4), &text[len - 4..])
            } else {
                "*".repeat(len)
            }
        } else {
            text.to_string()
        }
    }
}

fn gutter() -> Span<'static> {
    Span::from("› ")
}

impl BottomPaneView for ConfigInputView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.on_ctrl_c();
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.advance_step();
            }
            other => {
                self.textarea.input(other);
            }
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        if pasted.is_empty() {
            return false;
        }
        self.textarea.insert_str(&pasted);
        true
    }
}

impl Renderable for ConfigInputView {
    fn desired_height(&self, width: u16) -> u16 {
        2u16 + self.input_height(width) + 2u16
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if area.height < 2 || area.width <= 2 {
            return None;
        }
        let text_area_height = self.input_height(area.width).saturating_sub(1);
        if text_area_height == 0 {
            return None;
        }
        let top_line_count = 2u16; // provider + title
        let textarea_rect = Rect {
            x: area.x.saturating_add(2),
            y: area.y.saturating_add(top_line_count).saturating_add(1),
            width: area.width.saturating_sub(2),
            height: text_area_height,
        };
        let state = *self.textarea_state.borrow();
        self.textarea.cursor_pos_with_state(textarea_rect, state)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let input_height = self.input_height(area.width);

        // Provider line
        let provider_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        let provider_line = format!("Add {} Configuration", self.provider_name());
        Paragraph::new(Line::from(vec![gutter(), provider_line.bold().cyan()]))
            .render(provider_area, buf);

        // Title line
        let title_area = Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: 1,
        };
        Paragraph::new(Line::from(vec![gutter(), Span::from(self.title())]))
            .render(title_area, buf);

        // Input area
        let input_area = Rect {
            x: area.x,
            y: area.y.saturating_add(2),
            width: area.width,
            height: input_height,
        };

        if input_area.width >= 2 {
            for row in 0..input_area.height {
                Paragraph::new(Line::from(vec![gutter()])).render(
                    Rect {
                        x: input_area.x,
                        y: input_area.y.saturating_add(row),
                        width: 2,
                        height: 1,
                    },
                    buf,
                );
            }

            let text_area_height = input_area.height.saturating_sub(1);
            if text_area_height > 0 && input_area.width > 2 {
                let blank_rect = Rect {
                    x: input_area.x.saturating_add(2),
                    y: input_area.y,
                    width: input_area.width.saturating_sub(2),
                    height: 1,
                };
                Clear.render(blank_rect, buf);

                let textarea_rect = Rect {
                    x: input_area.x.saturating_add(2),
                    y: input_area.y.saturating_add(1),
                    width: input_area.width.saturating_sub(2),
                    height: text_area_height,
                };

                // For API key step, we show masked text but still use the textarea for input
                if matches!(self.step, ConfigInputStep::ApiKey) {
                    let display = self.display_text();
                    if display.is_empty() {
                        Paragraph::new(Line::from(self.placeholder().dim()))
                            .render(textarea_rect, buf);
                    } else {
                        Paragraph::new(Line::from(display)).render(textarea_rect, buf);
                    }
                } else {
                    let mut state = self.textarea_state.borrow_mut();
                    StatefulWidgetRef::render_ref(
                        &(&self.textarea),
                        textarea_rect,
                        buf,
                        &mut state,
                    );
                    if self.textarea.text().is_empty() {
                        Paragraph::new(Line::from(self.placeholder().dim()))
                            .render(textarea_rect, buf);
                    }
                }
            }
        }

        // Hint line
        let hint_y = input_area.y.saturating_add(input_height).saturating_add(1);
        if hint_y < area.y.saturating_add(area.height) {
            Paragraph::new(standard_popup_hint_line()).render(
                Rect {
                    x: area.x,
                    y: hint_y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }
    }
}
