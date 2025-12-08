//! Anthropic Messages API SSE parser.
//!
//! This module provides functionality to parse Server-Sent Events from
//! the Anthropic Messages API streaming endpoint.

use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use codex_client::StreamResponse;
use codex_client::TransportError;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Event;
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

use crate::telemetry::SseTelemetry;

/// Spawn a response stream from an Anthropic SSE stream.
pub fn spawn_anthropic_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    _telemetry: Option<Arc<dyn SseTelemetry>>,
) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        process_anthropic_sse(stream_response.bytes, tx_event, idle_timeout).await;
    });

    ResponseStream { rx_event }
}

/// Process Anthropic SSE events and emit ResponseEvents.
pub async fn process_anthropic_sse<S>(
    stream: S,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
) where
    S: Stream<Item = Result<bytes::Bytes, TransportError>> + Send + Unpin + 'static,
{
    let mut sse_stream = stream.eventsource();
    let mut response_id = String::new();
    let mut current_tool_use: Option<ToolUseAccumulator> = None;
    let mut input_tokens: Option<i64> = None;
    let mut output_tokens: Option<i64> = None;
    let mut message_item_emitted = false;

    // Send Created event
    let _ = tx_event.send(Ok(ResponseEvent::Created)).await;

    loop {
        let timeout = tokio::time::timeout(idle_timeout, sse_stream.next());
        match timeout.await {
            Ok(Some(Ok(event))) => {
                if let Err(e) = process_event(
                    &event,
                    &tx_event,
                    &mut response_id,
                    &mut current_tool_use,
                    &mut input_tokens,
                    &mut output_tokens,
                    &mut message_item_emitted,
                )
                .await
                {
                    let _ = tx_event.send(Err(e)).await;
                    break;
                }

                // Check for message_stop event
                if event.event == "message_stop" {
                    // If we had a text message, emit OutputItemDone for it
                    if message_item_emitted {
                        let _ = tx_event
                            .send(Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
                                id: Some(format!("{}-msg", response_id)),
                                role: "assistant".to_string(),
                                content: vec![], // Content was streamed via deltas
                            })))
                            .await;
                    }

                    // Send Completed event
                    let token_usage = if input_tokens.is_some() || output_tokens.is_some() {
                        let in_tokens = input_tokens.unwrap_or(0);
                        let out_tokens = output_tokens.unwrap_or(0);
                        Some(TokenUsage {
                            input_tokens: in_tokens,
                            output_tokens: out_tokens,
                            cached_input_tokens: 0,
                            reasoning_output_tokens: 0,
                            total_tokens: in_tokens + out_tokens,
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
            }
            Ok(Some(Err(e))) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(format!("SSE error: {e}"))))
                    .await;
                break;
            }
            Ok(None) => {
                // Stream ended
                break;
            }
            Err(_) => {
                // Timeout
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "Anthropic SSE stream timed out".to_string(),
                    )))
                    .await;
                break;
            }
        }
    }
}

/// Accumulator for tool use input JSON.
struct ToolUseAccumulator {
    id: String,
    name: String,
    input_json: String,
}

/// Process a single SSE event.
async fn process_event(
    event: &Event,
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    response_id: &mut String,
    current_tool_use: &mut Option<ToolUseAccumulator>,
    input_tokens: &mut Option<i64>,
    output_tokens: &mut Option<i64>,
    message_item_emitted: &mut bool,
) -> Result<(), ApiError> {
    let event_type = &event.event;
    let data = &event.data;

    debug!("Anthropic SSE event: {} = {}", event_type, data);

    match event_type.as_str() {
        "message_start" => {
            // Parse message_start to get response ID and initial usage
            if let Ok(parsed) = serde_json::from_str::<MessageStartEvent>(data) {
                *response_id = parsed.message.id;
                if let Some(usage) = parsed.message.usage {
                    *input_tokens = Some(usage.input_tokens);
                }
            }
        }
        "content_block_start" => {
            // Parse content_block_start
            if let Ok(parsed) = serde_json::from_str::<ContentBlockStartEvent>(data) {
                match parsed.content_block.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        // Text content block started - emit OutputItemAdded if not already done
                        if !*message_item_emitted {
                            *message_item_emitted = true;
                            let _ = tx_event
                                .send(Ok(ResponseEvent::OutputItemAdded(ResponseItem::Message {
                                    id: Some(format!("{}-msg", response_id)),
                                    role: "assistant".to_string(),
                                    content: vec![],
                                })))
                                .await;
                        }
                    }
                    Some("tool_use") => {
                        // Start accumulating tool use
                        let id = parsed
                            .content_block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = parsed
                            .content_block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        *current_tool_use = Some(ToolUseAccumulator {
                            id,
                            name,
                            input_json: String::new(),
                        });
                    }
                    Some("thinking") => {
                        // Thinking block started - emit reasoning summary part added
                        let _ = tx_event
                            .send(Ok(ResponseEvent::ReasoningSummaryPartAdded {
                                summary_index: parsed.index,
                            }))
                            .await;
                    }
                    _ => {}
                }
            }
        }
        "content_block_delta" => {
            // Parse content_block_delta
            if let Ok(parsed) = serde_json::from_str::<ContentBlockDeltaEvent>(data) {
                match parsed.delta.get("type").and_then(|v| v.as_str()) {
                    Some("text_delta") => {
                        // Emit OutputItemAdded if not already done (fallback for missing content_block_start)
                        if !*message_item_emitted {
                            *message_item_emitted = true;
                            let _ = tx_event
                                .send(Ok(ResponseEvent::OutputItemAdded(ResponseItem::Message {
                                    id: Some(format!("{}-msg", response_id)),
                                    role: "assistant".to_string(),
                                    content: vec![],
                                })))
                                .await;
                        }
                        if let Some(text) = parsed.delta.get("text").and_then(|v| v.as_str()) {
                            let _ = tx_event
                                .send(Ok(ResponseEvent::OutputTextDelta(text.to_string())))
                                .await;
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(thinking) =
                            parsed.delta.get("thinking").and_then(|v| v.as_str())
                        {
                            let _ = tx_event
                                .send(Ok(ResponseEvent::ReasoningContentDelta {
                                    delta: thinking.to_string(),
                                    content_index: parsed.index,
                                }))
                                .await;
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(partial) =
                            parsed.delta.get("partial_json").and_then(|v| v.as_str())
                        {
                            if let Some(tool_use) = current_tool_use {
                                tool_use.input_json.push_str(partial);
                            }
                        }
                    }
                    Some("signature_delta") => {
                        // Signature for thinking block verification - ignore for now
                    }
                    _ => {}
                }
            }
        }
        "content_block_stop" => {
            // Finish any accumulated tool use
            if let Some(tool_use) = current_tool_use.take() {
                let arguments = if tool_use.input_json.is_empty() {
                    "{}".to_string()
                } else {
                    tool_use.input_json
                };

                let item = ResponseItem::FunctionCall {
                    id: None,
                    call_id: tool_use.id,
                    name: tool_use.name,
                    arguments,
                };

                let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
            }
        }
        "message_delta" => {
            // Parse message_delta for usage info
            if let Ok(parsed) = serde_json::from_str::<MessageDeltaEvent>(data) {
                if let Some(usage) = parsed.usage {
                    *output_tokens = Some(usage.output_tokens);
                }
            }
        }
        "message_stop" => {
            // Handled in main loop
        }
        "ping" => {
            // Keepalive, ignore
        }
        "error" => {
            // Parse error event
            if let Ok(parsed) = serde_json::from_str::<ErrorEvent>(data) {
                return Err(ApiError::Stream(format!(
                    "Anthropic error: {} - {}",
                    parsed.error.r#type, parsed.error.message
                )));
            }
        }
        _ => {
            // Unknown event type
            warn!("Unknown Anthropic SSE event type: {}", event_type);
        }
    }

    Ok(())
}

// SSE event structures for Anthropic

#[derive(Debug, Deserialize)]
struct MessageStartEvent {
    message: MessageInfo,
}

#[derive(Debug, Deserialize)]
struct MessageInfo {
    id: String,
    #[serde(default)]
    usage: Option<InputUsage>,
}

#[derive(Debug, Deserialize)]
struct InputUsage {
    input_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct ContentBlockStartEvent {
    index: i64,
    content_block: Value,
}

#[derive(Debug, Deserialize)]
struct ContentBlockDeltaEvent {
    index: i64,
    delta: Value,
}

#[derive(Debug, Deserialize)]
struct MessageDeltaEvent {
    #[serde(default)]
    usage: Option<OutputUsage>,
}

#[derive(Debug, Deserialize)]
struct OutputUsage {
    output_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct ErrorEvent {
    error: AnthropicError,
}

#[derive(Debug, Deserialize)]
struct AnthropicError {
    r#type: String,
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_process_message_start() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut response_id = String::new();
        let mut current_tool_use = None;
        let mut input_tokens = None;
        let mut output_tokens = None;

        let event = Event {
            event: "message_start".to_string(),
            data: r#"{"type":"message_start","message":{"id":"msg_123","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":25}}}"#.to_string(),
            ..Default::default()
        };

        process_event(
            &event,
            &tx,
            &mut response_id,
            &mut current_tool_use,
            &mut input_tokens,
            &mut output_tokens,
        )
        .await
        .unwrap();

        assert_eq!(response_id, "msg_123");
        assert_eq!(input_tokens, Some(25));
    }

    #[tokio::test]
    async fn test_process_text_delta() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut response_id = String::new();
        let mut current_tool_use = None;
        let mut input_tokens = None;
        let mut output_tokens = None;

        let event = Event {
            event: "content_block_delta".to_string(),
            data: r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#.to_string(),
            ..Default::default()
        };

        process_event(
            &event,
            &tx,
            &mut response_id,
            &mut current_tool_use,
            &mut input_tokens,
            &mut output_tokens,
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
}
