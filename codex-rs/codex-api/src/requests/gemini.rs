//! Google Gemini API request builder.
//!
//! This module provides functionality to build requests compatible with
//! the Google Gemini API (https://generativelanguage.googleapis.com/v1beta).

use crate::error::ApiError;
use crate::provider::Provider;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use http::HeaderMap;
use http::header::HeaderName;
use http::header::HeaderValue;
use serde_json::Value;
use serde_json::json;

/// Build the Gemini streaming endpoint URL.
pub fn gemini_streaming_url(base_url: &str, model: &str) -> String {
    // Expected format: https://generativelanguage.googleapis.com/v1beta
    // Resulting URL: {base_url}/models/{model}:streamGenerateContent?alt=sse
    format!(
        "{}/models/{}:streamGenerateContent?alt=sse",
        base_url, model
    )
}

/// Assembled request body plus headers for Gemini streaming calls.
pub struct GeminiRequest {
    pub body: Value,
    pub headers: HeaderMap,
    pub url: String,
}

pub struct GeminiRequestBuilder<'a> {
    model: &'a str,
    instructions: &'a str,
    input: &'a [ResponseItem],
    tools: &'a [Value],
    base_url: &'a str,
    thinking_level: Option<&'a str>,
}

impl<'a> GeminiRequestBuilder<'a> {
    pub fn new(
        model: &'a str,
        instructions: &'a str,
        input: &'a [ResponseItem],
        tools: &'a [Value],
        base_url: &'a str,
    ) -> Self {
        Self {
            model,
            instructions,
            input,
            tools,
            base_url,
            thinking_level: None,
        }
    }

    /// Set the thinking level ("low" or "high") for reasoning models.
    pub fn thinking_level(mut self, level: Option<&'a str>) -> Self {
        self.thinking_level = level;
        self
    }

    pub fn build(self, _provider: &Provider) -> Result<GeminiRequest, ApiError> {
        // Build contents array from input
        let contents = self.build_contents();

        // Build tools array in Gemini format
        let tools = self.build_tools();

        // Build the request body
        let mut body = json!({
            "contents": contents,
        });

        // Add system instruction if present
        if !self.instructions.is_empty() {
            body["systemInstruction"] = json!({
                "parts": [{
                    "text": self.instructions
                }]
            });
        }

        // Add tools if present
        if !tools.is_empty() {
            body["tools"] = json!([{
                "functionDeclarations": tools
            }]);
        }

        // Add generation config with thinking if specified
        if let Some(level) = self.thinking_level {
            body["generationConfig"] = json!({
                "thinkingConfig": {
                    "thinkingLevel": level.to_uppercase()
                }
            });
        }

        // Build headers
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );

        // Build the URL with model name
        let url = gemini_streaming_url(self.base_url, self.model);

        Ok(GeminiRequest { body, headers, url })
    }

    /// Convert ResponseItems to Gemini content format.
    fn build_contents(&self) -> Vec<Value> {
        let mut contents = Vec::new();
        let mut current_parts: Vec<Value> = Vec::new();
        let mut current_role: Option<&str> = None;

        for item in self.input {
            match item {
                ResponseItem::Message { role, content, .. } => {
                    // Flush previous content if role changes
                    let gemini_role = match role.as_str() {
                        "assistant" => "model",
                        _ => "user",
                    };

                    if let Some(prev_role) = current_role {
                        if prev_role != gemini_role && !current_parts.is_empty() {
                            contents.push(json!({
                                "role": prev_role,
                                "parts": current_parts.clone()
                            }));
                            current_parts.clear();
                        }
                    }

                    current_role = Some(gemini_role);

                    // Add content parts
                    for content_item in content {
                        match content_item {
                            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                                current_parts.push(json!({"text": text}));
                            }
                            ContentItem::InputImage { image_url } => {
                                // Gemini uses inline_data for images
                                if image_url.starts_with("data:") {
                                    if let Some(comma_pos) = image_url.find(',') {
                                        let media_type = image_url[5..comma_pos]
                                            .split(';')
                                            .next()
                                            .unwrap_or("image/png");
                                        let data = &image_url[comma_pos + 1..];
                                        current_parts.push(json!({
                                            "inlineData": {
                                                "mimeType": media_type,
                                                "data": data
                                            }
                                        }));
                                    }
                                }
                            }
                        }
                    }
                }
                ResponseItem::FunctionCall {
                    call_id,
                    name,
                    arguments,
                    ..
                } => {
                    // Flush any pending user content
                    if let Some(prev_role) = current_role {
                        if !current_parts.is_empty() {
                            contents.push(json!({
                                "role": prev_role,
                                "parts": current_parts.clone()
                            }));
                            current_parts.clear();
                        }
                    }

                    // Add model message with function call
                    let args: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));
                    contents.push(json!({
                        "role": "model",
                        "parts": [{
                            "functionCall": {
                                "name": name,
                                "args": args
                            }
                        }]
                    }));

                    current_role = None;

                    // Store call_id for matching with output
                    let _ = call_id; // Used for matching
                }
                ResponseItem::FunctionCallOutput { call_id, output } => {
                    // Add function response
                    // Try to find the function name from previous items
                    let function_name = self
                        .input
                        .iter()
                        .find_map(|item| match item {
                            ResponseItem::FunctionCall {
                                call_id: fc_call_id,
                                name,
                                ..
                            } if fc_call_id == call_id => Some(name.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "unknown".to_string());

                    contents.push(json!({
                        "role": "user",
                        "parts": [{
                            "functionResponse": {
                                "name": function_name,
                                "response": {
                                    "content": output.content
                                }
                            }
                        }]
                    }));

                    current_role = None;
                }
                ResponseItem::LocalShellCall {
                    call_id, action, ..
                } => {
                    // Flush any pending content
                    if let Some(prev_role) = current_role {
                        if !current_parts.is_empty() {
                            contents.push(json!({
                                "role": prev_role,
                                "parts": current_parts.clone()
                            }));
                            current_parts.clear();
                        }
                    }

                    // Convert shell call to function call
                    let command = match action {
                        codex_protocol::models::LocalShellAction::Exec(exec_action) => {
                            exec_action.command.join(" ")
                        }
                    };
                    contents.push(json!({
                        "role": "model",
                        "parts": [{
                            "functionCall": {
                                "name": "shell",
                                "args": {"command": command}
                            }
                        }]
                    }));

                    current_role = None;
                    let _ = call_id; // Used for matching
                }
                // Skip internal items
                ResponseItem::Reasoning { .. }
                | ResponseItem::CompactionSummary { .. }
                | ResponseItem::GhostSnapshot { .. }
                | ResponseItem::CustomToolCall { .. }
                | ResponseItem::CustomToolCallOutput { .. }
                | ResponseItem::WebSearchCall { .. }
                | ResponseItem::Other => {}
            }
        }

        // Flush remaining content
        if let Some(role) = current_role {
            if !current_parts.is_empty() {
                contents.push(json!({
                    "role": role,
                    "parts": current_parts
                }));
            }
        }

        contents
    }

    /// Convert tools to Gemini functionDeclarations format.
    fn build_tools(&self) -> Vec<Value> {
        self.tools
            .iter()
            .filter_map(|tool| {
                // OpenAI tool format -> Gemini functionDeclarations format
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
                        "parameters": parameters
                    }))
                } else {
                    // Already in Gemini format or unknown
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
    use http::HeaderMap;
    use pretty_assertions::assert_eq;

    fn mock_provider() -> Provider {
        Provider {
            name: "google".to_string(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            query_params: None,
            wire: crate::provider::WireApi::Gemini,
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

        let builder = GeminiRequestBuilder::new(
            "gemini-3-pro-preview",
            "You are a helpful assistant.",
            &input,
            &[],
            "https://generativelanguage.googleapis.com/v1beta",
        );

        let request = builder.build(&mock_provider()).unwrap();

        assert!(request.url.contains("gemini-3-pro-preview"));
        assert!(request.url.contains("streamGenerateContent"));

        let system = &request.body["systemInstruction"];
        assert_eq!(system["parts"][0]["text"], "You are a helpful assistant.");

        let contents = request.body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
    }

    #[test]
    fn test_thinking_config() {
        let builder = GeminiRequestBuilder::new(
            "gemini-3-pro-preview",
            "",
            &[],
            &[],
            "https://generativelanguage.googleapis.com/v1beta",
        )
        .thinking_level(Some("high"));

        let request = builder.build(&mock_provider()).unwrap();

        let thinking_config = &request.body["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking_config["thinkingLevel"], "HIGH");
    }

    #[test]
    fn test_streaming_url() {
        let url = gemini_streaming_url(
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-3-pro-preview",
        );
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3-pro-preview:streamGenerateContent?alt=sse"
        );
    }
}
