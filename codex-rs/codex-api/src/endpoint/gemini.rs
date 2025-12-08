//! Google Gemini API client.
//!
//! This module provides the client for interacting with the Google Gemini API.

use crate::auth::AuthProvider;
use crate::auth::add_auth_headers;
use crate::common::Prompt as ApiPrompt;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::GeminiRequest;
use crate::requests::GeminiRequestBuilder;
use crate::sse::spawn_gemini_stream;
use crate::telemetry::SseTelemetry;
use crate::telemetry::run_with_request_telemetry;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use http::HeaderMap;
use http::Method;
use serde_json::Value;
use std::sync::Arc;

/// Options for Gemini API requests.
#[derive(Default)]
pub struct GeminiOptions {
    /// Thinking level for reasoning ("low" or "high").
    pub thinking_level: Option<String>,
}

pub struct GeminiClient<T: HttpTransport, A: AuthProvider> {
    transport: T,
    provider: Provider,
    auth: A,
    request_telemetry: Option<Arc<dyn RequestTelemetry>>,
    sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

impl<T: HttpTransport, A: AuthProvider> GeminiClient<T, A> {
    pub fn new(transport: T, provider: Provider, auth: A) -> Self {
        Self {
            transport,
            provider,
            auth,
            request_telemetry: None,
            sse_telemetry: None,
        }
    }

    pub fn with_telemetry(
        mut self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        self.request_telemetry = request;
        self.sse_telemetry = sse;
        self
    }

    pub async fn stream_request(&self, request: GeminiRequest) -> Result<ResponseStream, ApiError> {
        // Gemini uses a custom URL with model name embedded
        self.stream_with_url(&request.url, request.body, request.headers)
            .await
    }

    pub async fn stream_prompt(
        &self,
        model: &str,
        prompt: &ApiPrompt,
        options: GeminiOptions,
    ) -> Result<ResponseStream, ApiError> {
        let GeminiOptions { thinking_level } = options;

        let mut builder = GeminiRequestBuilder::new(
            model,
            &prompt.instructions,
            &prompt.input,
            &prompt.tools,
            &self.provider.base_url,
        );

        if let Some(ref level) = thinking_level {
            builder = builder.thinking_level(Some(level.as_str()));
        }

        let request = builder.build(&self.provider)?;
        self.stream_request(request).await
    }

    /// Stream with a custom URL (Gemini embeds model name in URL).
    async fn stream_with_url(
        &self,
        url: &str,
        body: Value,
        extra_headers: HeaderMap,
    ) -> Result<ResponseStream, ApiError> {
        let builder = || {
            let mut req = self.provider.build_absolute_request(Method::POST, url);
            req.headers.extend(extra_headers.clone());
            req.headers.insert(
                http::header::ACCEPT,
                http::HeaderValue::from_static("text/event-stream"),
            );
            req.body = Some(body.clone());
            add_auth_headers(&self.auth, req, &self.provider.wire)
        };

        let stream_response = run_with_request_telemetry(
            self.provider.retry.to_policy(),
            self.request_telemetry.clone(),
            builder,
            |req| self.transport.stream(req),
        )
        .await?;

        Ok(spawn_gemini_stream(
            stream_response,
            self.provider.stream_idle_timeout,
            self.sse_telemetry.clone(),
        ))
    }
}
