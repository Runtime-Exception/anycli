//! Anthropic Messages API client.
//!
//! This module provides the client for interacting with the Anthropic Messages API.

use crate::auth::AuthProvider;
use crate::common::Prompt as ApiPrompt;
use crate::common::ResponseStream;
use crate::endpoint::streaming::StreamingClient;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::AnthropicRequest;
use crate::requests::AnthropicRequestBuilder;
use crate::sse::spawn_anthropic_stream;
use crate::telemetry::SseTelemetry;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use http::HeaderMap;
use serde_json::Value;
use std::sync::Arc;

/// Options for Anthropic API requests.
#[derive(Default)]
pub struct AnthropicOptions {
    pub max_tokens: Option<i32>,
    pub thinking_enabled: bool,
    pub thinking_budget: Option<i32>,
}

pub struct AnthropicClient<T: HttpTransport, A: AuthProvider> {
    streaming: StreamingClient<T, A>,
}

impl<T: HttpTransport, A: AuthProvider> AnthropicClient<T, A> {
    pub fn new(transport: T, provider: Provider, auth: A) -> Self {
        Self {
            streaming: StreamingClient::new(transport, provider, auth),
        }
    }

    pub fn with_telemetry(
        self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        Self {
            streaming: self.streaming.with_telemetry(request, sse),
        }
    }

    pub async fn stream_request(
        &self,
        request: AnthropicRequest,
    ) -> Result<ResponseStream, ApiError> {
        self.stream(request.body, request.headers).await
    }

    pub async fn stream_prompt(
        &self,
        model: &str,
        prompt: &ApiPrompt,
        options: AnthropicOptions,
    ) -> Result<ResponseStream, ApiError> {
        let AnthropicOptions {
            max_tokens,
            thinking_enabled,
            thinking_budget,
        } = options;

        let mut builder =
            AnthropicRequestBuilder::new(model, &prompt.instructions, &prompt.input, &prompt.tools);

        if let Some(tokens) = max_tokens {
            builder = builder.max_tokens(tokens);
        }

        builder = builder
            .thinking_enabled(thinking_enabled)
            .thinking_budget(thinking_budget);

        let request = builder.build(self.streaming.provider())?;
        self.stream_request(request).await
    }

    /// Anthropic API uses "v1/messages" path.
    fn path(&self) -> &'static str {
        "v1/messages"
    }

    pub async fn stream(
        &self,
        body: Value,
        extra_headers: HeaderMap,
    ) -> Result<ResponseStream, ApiError> {
        self.streaming
            .stream(self.path(), body, extra_headers, spawn_anthropic_stream)
            .await
    }
}
