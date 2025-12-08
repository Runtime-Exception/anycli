//! Google Gemini API SSE parser.
//!
//! This module provides functionality to parse Server-Sent Events from
//! the Google Gemini API streaming endpoint.

use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::StreamResponse;
use codex_client::TransportError;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::Stream;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::debug;
use tracing::warn;

/// Spawn a response stream from a Gemini SSE stream.
pub fn spawn_gemini_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    _telemetry: Option<Arc<dyn SseTelemetry>>,
) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        process_gemini_sse(stream_response.bytes, tx_event, idle_timeout).await;
    });

    ResponseStream { rx_event }
}

/// Process Gemini SSE events and emit ResponseEvents.
pub async fn process_gemini_sse<S>(
    stream: S,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
) where
    S: Stream<Item = Result<bytes::Bytes, TransportError>> + Send + Unpin + 'static,
{
    let mut sse_stream = stream.eventsource();
    let mut response_id = String::new();
    let mut input_tokens: i64 = 0;
    let mut output_tokens: i64 = 0;
    let mut thought_tokens: i64 = 0;
    let mut function_call_accumulator: Option<FunctionCallAccumulator> = None;

    // Send Created event
    let _ = tx_event.send(Ok(ResponseEvent::Created)).await;

    loop {
        let timeout = tokio::time::timeout(idle_timeout, sse_stream.next());
        match timeout.await {
            Ok(Some(Ok(event))) => {
                // Gemini uses "data" events only
                if event.event.is_empty() || event.event == "message" {
                    if let Err(e) = process_gemini_chunk(
                        &event.data,
                        &tx_event,
                        &mut response_id,
                        &mut input_tokens,
                        &mut output_tokens,
                        &mut thought_tokens,
                        &mut function_call_accumulator,
                    )
                    .await
                    {
                        let _ = tx_event.send(Err(e)).await;
                        break;
                    }
                }
            }
            Ok(Some(Err(e))) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(format!("SSE error: {e}"))))
                    .await;
                break;
            }
            Ok(None) => {
                // Stream ended - send completed event
                let token_usage = if input_tokens > 0 || output_tokens > 0 || thought_tokens > 0 {
                    Some(TokenUsage {
                        input_tokens,
                        output_tokens,
                        cached_input_tokens: 0,
                        reasoning_output_tokens: thought_tokens,
                        total_tokens: input_tokens + output_tokens,
                    })
                } else {
                    None
                };

                let _ = tx_event
                    .send(Ok(ResponseEvent::Completed {
                        response_id: response_id.clone(),
                        token_usage,
                    }))
                    .await;
                break;
            }
            Err(_) => {
                // Timeout
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "Gemini SSE stream timed out".to_string(),
                    )))
                    .await;
                break;
            }
        }
    }
}

/// Accumulator for function call data.
struct FunctionCallAccumulator {
    name: String,
    args: Value,
}

/// Process a single Gemini SSE data chunk.
async fn process_gemini_chunk(
    data: &str,
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    response_id: &mut String,
    input_tokens: &mut i64,
    output_tokens: &mut i64,
    thought_tokens: &mut i64,
    function_call_accumulator: &mut Option<FunctionCallAccumulator>,
) -> Result<(), ApiError> {
    debug!("Gemini SSE chunk: {}", data);

    // Parse the JSON chunk
    let chunk: GeminiStreamChunk = match serde_json::from_str(data) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to parse Gemini chunk: {}", e);
            return Ok(());
        }
    };

    // Update usage metadata if present
    if let Some(metadata) = &chunk.usage_metadata {
        *input_tokens = metadata.prompt_token_count.unwrap_or(0);
        *output_tokens = metadata.candidates_token_count.unwrap_or(0);
        *thought_tokens = metadata.thoughts_token_count.unwrap_or(0);
    }

    // Process candidates
    if let Some(candidates) = &chunk.candidates {
        for candidate in candidates {
            // Update response ID from candidate index
            if response_id.is_empty() {
                *response_id = format!("gemini-{}", candidate.index.unwrap_or(0));
            }

            if let Some(content) = &candidate.content {
                for part in &content.parts {
                    // Check if this is a thought/reasoning part
                    if part.thought.unwrap_or(false) {
                        // Emit reasoning content delta
                        if let Some(text) = &part.text {
                            let _ = tx_event
                                .send(Ok(ResponseEvent::ReasoningContentDelta {
                                    delta: text.clone(),
                                    content_index: 0,
                                }))
                                .await;
                        }
                    } else if let Some(text) = &part.text {
                        // Regular text output
                        let _ = tx_event
                            .send(Ok(ResponseEvent::OutputTextDelta(text.clone())))
                            .await;
                    }

                    // Check for function call
                    if let Some(function_call) = &part.function_call {
                        *function_call_accumulator = Some(FunctionCallAccumulator {
                            name: function_call.name.clone(),
                            args: function_call.args.clone(),
                        });
                    }
                }
            }

            // Check finish reason
            if let Some(finish_reason) = &candidate.finish_reason {
                // If we have accumulated function call, emit it
                if let Some(fc) = function_call_accumulator.take() {
                    let arguments =
                        serde_json::to_string(&fc.args).unwrap_or_else(|_| "{}".to_string());
                    let call_id = format!("fc_{}", fc.name);

                    let item = ResponseItem::FunctionCall {
                        id: None,
                        call_id,
                        name: fc.name,
                        arguments,
                    };

                    let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                }

                // Handle specific finish reasons
                match finish_reason.as_str() {
                    "STOP" => {
                        // Normal completion, handled by stream end
                    }
                    "SAFETY" | "RECITATION" | "OTHER" => {
                        warn!("Gemini response finished with reason: {}", finish_reason);
                    }
                    "MAX_TOKENS" => {
                        warn!("Gemini response truncated due to max tokens");
                    }
                    _ => {
                        debug!("Unknown finish reason: {}", finish_reason);
                    }
                }
            }
        }
    }

    // Check for errors in the response
    if let Some(error) = &chunk.error {
        return Err(ApiError::Stream(format!(
            "Gemini error: {} - {}",
            error.code.unwrap_or(0),
            error.message.as_deref().unwrap_or("Unknown error")
        )));
    }

    Ok(())
}

// Gemini SSE response structures

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiStreamChunk {
    #[serde(default)]
    candidates: Option<Vec<GeminiCandidate>>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default)]
    error: Option<GeminiError>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    #[serde(default)]
    index: Option<i64>,
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
    #[serde(default)]
    role: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thought: Option<bool>,
    #[serde(default)]
    function_call: Option<GeminiFunctionCall>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionCall {
    name: String,
    args: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    #[serde(default)]
    prompt_token_count: Option<i64>,
    #[serde(default)]
    candidates_token_count: Option<i64>,
    #[serde(default)]
    thoughts_token_count: Option<i64>,
    #[serde(default)]
    total_token_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiError {
    #[serde(default)]
    code: Option<i32>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_process_text_chunk() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut response_id = String::new();
        let mut input_tokens = 0;
        let mut output_tokens = 0;
        let mut thought_tokens = 0;
        let mut function_call_accumulator = None;

        let data =
            r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"},"index":0}]}"#;

        process_gemini_chunk(
            data,
            &tx,
            &mut response_id,
            &mut input_tokens,
            &mut output_tokens,
            &mut thought_tokens,
            &mut function_call_accumulator,
        )
        .await
        .unwrap();

        if let Ok(result) = rx.try_recv() {
            match result {
                Ok(ResponseEvent::OutputTextDelta(text)) => {
                    assert_eq!(text, "Hello");
                }
                _ => panic!("Expected OutputTextDelta event"),
            }
        }
    }

    #[tokio::test]
    async fn test_process_thought_chunk() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut response_id = String::new();
        let mut input_tokens = 0;
        let mut output_tokens = 0;
        let mut thought_tokens = 0;
        let mut function_call_accumulator = None;

        let data = r#"{"candidates":[{"content":{"parts":[{"text":"thinking...","thought":true}],"role":"model"},"index":0}]}"#;

        process_gemini_chunk(
            data,
            &tx,
            &mut response_id,
            &mut input_tokens,
            &mut output_tokens,
            &mut thought_tokens,
            &mut function_call_accumulator,
        )
        .await
        .unwrap();

        if let Ok(result) = rx.try_recv() {
            match result {
                Ok(ResponseEvent::ReasoningContentDelta { delta, .. }) => {
                    assert_eq!(delta, "thinking...");
                }
                _ => panic!("Expected ReasoningContentDelta event"),
            }
        }
    }
}
