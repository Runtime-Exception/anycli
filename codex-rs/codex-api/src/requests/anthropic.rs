//! Anthropic Messages API request builder.
//!
//! This module provides functionality to build requests compatible with
//! the Anthropic Messages API (https://api.anthropic.com/v1/messages).

use crate::error::ApiError;
use crate::provider::Provider;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use http::HeaderMap;
use http::header::HeaderName;
use http::header::HeaderValue;
use serde_json::Value;
use serde_json::json;

/// The Anthropic API version header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Maximum tokens for Anthropic models (default).
pub const DEFAULT_MAX_TOKENS: i32 = 8192;

/// Assembled request body plus headers for Anthropic streaming calls.
pub struct AnthropicRequest {
    pub body: Value,
    pub headers: HeaderMap,
}

pub struct AnthropicRequestBuilder<'a> {
    model: &'a str,
    instructions: &'a str,
    input: &'a [ResponseItem],
    tools: &'a [Value],
    max_tokens: i32,
    thinking_enabled: bool,
    thinking_budget: Option<i32>,
}

impl<'a> AnthropicRequestBuilder<'a> {
    pub fn new(
        model: &'a str,
        instructions: &'a str,
        input: &'a [ResponseItem],
        tools: &'a [Value],
    ) -> Self {
        Self {
            model,
            instructions,
            input,
            tools,
            max_tokens: DEFAULT_MAX_TOKENS,
            thinking_enabled: false,
            thinking_budget: None,
        }
    }

    pub fn max_tokens(mut self, tokens: i32) -> Self {
        self.max_tokens = tokens;
        self
    }

    pub fn thinking_enabled(mut self, enabled: bool) -> Self {
        self.thinking_enabled = enabled;
        self
    }

    pub fn thinking_budget(mut self, budget: Option<i32>) -> Self {
        self.thinking_budget = budget;
        self
    }

    pub fn build(self, _provider: &Provider) -> Result<AnthropicRequest, ApiError> {
        // Build messages array from input
        let messages = self.build_messages();

        // Build tools array in Anthropic format
        let tools = self.build_tools();

        // Build the request body
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "stream": true,
            "messages": messages,
        });

        // Add system instruction if present
        if !self.instructions.is_empty() {
            body["system"] = json!(self.instructions);
        }

        // Add tools if present
        if !tools.is_empty() {
            body["tools"] = json!(tools);
            body["tool_choice"] = json!({"type": "auto"});
        }

        // Add thinking configuration if enabled
        if self.thinking_enabled {
            let mut thinking = json!({"type": "enabled"});
            if let Some(budget) = self.thinking_budget {
                thinking["budget_tokens"] = json!(budget);
            }
            body["thinking"] = thinking;
        }

        // Build headers
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );

        Ok(AnthropicRequest { body, headers })
    }

    /// Convert ResponseItems to Anthropic message format.
    fn build_messages(&self) -> Vec<Value> {
        let mut messages = Vec::new();
        let mut pending_tool_results: Vec<Value> = Vec::new();

        for item in self.input {
            match item {
                ResponseItem::Message { role, content, .. } => {
                    // Flush any pending tool results before a new message
                    if !pending_tool_results.is_empty() {
                        messages.push(json!({
                            "role": "user",
                            "content": pending_tool_results.clone()
                        }));
                        pending_tool_results.clear();
                    }

                    let anthropic_role = match role.as_str() {
                        "assistant" => "assistant",
                        _ => "user",
                    };

                    let content_blocks = self.convert_content(content);
                    if !content_blocks.is_empty() {
                        messages.push(json!({
                            "role": anthropic_role,
                            "content": content_blocks
                        }));
                    }
                }
                ResponseItem::FunctionCall {
                    call_id,
                    name,
                    arguments,
                    ..
                } => {
                    // Flush any pending tool results
                    if !pending_tool_results.is_empty() {
                        messages.push(json!({
                            "role": "user",
                            "content": pending_tool_results.clone()
                        }));
                        pending_tool_results.clear();
                    }

                    // Add assistant message with tool_use
                    messages.push(json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": call_id,
                            "name": name,
                            "input": serde_json::from_str::<Value>(arguments).unwrap_or(json!({}))
                        }]
                    }));
                }
                ResponseItem::FunctionCallOutput { call_id, output } => {
                    // Accumulate tool results - use the content string directly
                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": output.content
                    }));
                }
                ResponseItem::LocalShellCall {
                    call_id, action, ..
                } => {
                    // Flush any pending tool results
                    if !pending_tool_results.is_empty() {
                        messages.push(json!({
                            "role": "user",
                            "content": pending_tool_results.clone()
                        }));
                        pending_tool_results.clear();
                    }

                    // Convert shell call to tool use
                    let command = match action {
                        codex_protocol::models::LocalShellAction::Exec(exec_action) => {
                            exec_action.command.join(" ")
                        }
                    };
                    // Use call_id if available, otherwise generate a placeholder
                    let tool_id = call_id.clone().unwrap_or_else(|| "shell_call".to_string());
                    messages.push(json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": tool_id,
                            "name": "shell",
                            "input": {"command": command}
                        }]
                    }));
                }
                // Skip reasoning, compaction summaries, and other internal items
                ResponseItem::Reasoning { .. }
                | ResponseItem::CompactionSummary { .. }
                | ResponseItem::GhostSnapshot { .. }
                | ResponseItem::CustomToolCall { .. }
                | ResponseItem::CustomToolCallOutput { .. }
                | ResponseItem::WebSearchCall { .. }
                | ResponseItem::Other => {}
            }
        }

        // Flush any remaining tool results
        if !pending_tool_results.is_empty() {
            messages.push(json!({
                "role": "user",
                "content": pending_tool_results
            }));
        }

        messages
    }

    /// Convert ContentItem to Anthropic content block format.
    fn convert_content(&self, content: &[ContentItem]) -> Vec<Value> {
        content
            .iter()
            .filter_map(|item| match item {
                ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                    Some(json!({"type": "text", "text": text}))
                }
                ContentItem::InputImage { image_url } => {
                    // Anthropic uses base64 image data
                    if image_url.starts_with("data:") {
                        // Parse data URL: data:image/png;base64,<data>
                        if let Some(comma_pos) = image_url.find(',') {
                            let media_type = image_url[5..comma_pos]
                                .split(';')
                                .next()
                                .unwrap_or("image/png");
                            let data = &image_url[comma_pos + 1..];
                            return Some(json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": data
                                }
                            }));
                        }
                    }
                    None
                }
            })
            .collect()
    }

    /// Convert tools to Anthropic format.
    fn build_tools(&self) -> Vec<Value> {
        self.tools
            .iter()
            .filter_map(|tool| {
                // OpenAI tool format -> Anthropic tool format
                if let Some(function) = tool.get("function") {
                    let name = function.get("name")?.as_str()?;
                    let description = function
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let parameters = function.get("parameters").cloned().unwrap_or(json!({}));

                    Some(json!({
                        "name": name,
                        "description": description,
                        "input_schema": parameters
                    }))
                } else {
                    // Already in Anthropic format or unknown
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::ContentItem;
    use pretty_assertions::assert_eq;

    fn mock_provider() -> Provider {
        Provider {
            name: "anthropic".to_string(),
            base_url: "https://api.anthropic.com/v1/messages".to_string(),
            query_params: None,
            wire: crate::provider::WireApi::Anthropic,
            headers: HeaderMap::new(),
            retry: crate::provider::RetryConfig {
                max_attempts: 3,
                base_delay: std::time::Duration::from_millis(200),
                retry_429: true,
                retry_5xx: true,
                retry_transport: true,
            },
            stream_idle_timeout: std::time::Duration::from_secs(300),
        }
    }

    #[test]
    fn test_basic_request() {
        let input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Hello".to_string(),
            }],
        }];

        let builder = AnthropicRequestBuilder::new(
            "claude-sonnet-4-5-20250929",
            "You are a helpful assistant.",
            &input,
            &[],
        );

        let request = builder.build(&mock_provider()).unwrap();

        assert_eq!(request.body["model"], "claude-sonnet-4-5-20250929");
        assert_eq!(request.body["stream"], true);
        assert_eq!(request.body["system"], "You are a helpful assistant.");

        let messages = request.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn test_headers() {
        let builder = AnthropicRequestBuilder::new("claude-sonnet-4-5-20250929", "", &[], &[]);

        let request = builder.build(&mock_provider()).unwrap();

        assert_eq!(
            request.headers.get("anthropic-version").unwrap(),
            ANTHROPIC_VERSION
        );
        assert_eq!(
            request.headers.get("content-type").unwrap(),
            "application/json"
        );
    }
}
