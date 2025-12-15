//! Alloy Agent task implementation.
//!
//! The Alloy Agent uses a two-phase execution model:
//! 1. Analyze Phase: Send user request to analyze model to research and create detailed specs
//! 2. Implement Phase: Send specs to implementation model to execute with full tool access
//!
//! The key difference from Classic Agent is that Alloy separates research/planning
//! from implementation, potentially using different models for each phase.

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::anycli::config::AnycliConfig;
use crate::codex::Codex;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::codex_delegate::run_codex_conversation_one_shot;
use crate::prompts::ALLOY_ANALYZE_PROMPT;
use crate::prompts::ALLOY_IMPLEMENT_PROMPT;
use crate::state::TaskKind;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::user_input::UserInput;

use super::SessionTask;
use super::SessionTaskContext;

/// Timeout configuration for Alloy phases
const INITIAL_TIMEOUT_SECS: u64 = 300; // 5 minutes for first response
const EVENT_TIMEOUT_SECS: u64 = 120; // 2 minutes between events

/// Configuration for the Alloy Agent task.
#[derive(Clone)]
pub(crate) struct AlloyTask {
    /// Name of the analyze configuration (references AnyCLI config).
    pub analyze_config: String,
    /// Name of the implementation configuration (references AnyCLI config).
    pub implement_config: String,
}

impl AlloyTask {
    /// Creates a new AlloyTask with the given configuration names.
    pub fn new(analyze_config: String, implement_config: String) -> Self {
        Self {
            analyze_config,
            implement_config,
        }
    }
}

#[async_trait]
impl SessionTask for AlloyTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Alloy
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        let sess = session.clone_session();

        tracing::info!(
            analyze_config = %self.analyze_config,
            implement_config = %self.implement_config,
            "Starting Alloy Agent two-phase execution"
        );

        // Load AnyCLI config to get model settings
        let anycli_config = match AnycliConfig::load() {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::error!("Failed to load AnyCLI config: {e}");
                send_error(
                    &sess,
                    &ctx,
                    format!("Alloy Agent failed: could not load config: {e}"),
                )
                .await;
                return None;
            }
        };

        // Validate analyze model config exists
        let analyze_entry = match anycli_config.configs.get(&self.analyze_config) {
            Some(entry) => entry.clone(),
            None => {
                tracing::error!("Analyze config '{}' not found", self.analyze_config);
                send_error(
                    &sess,
                    &ctx,
                    format!(
                        "Alloy Agent failed: analyze config '{}' not found",
                        self.analyze_config
                    ),
                )
                .await;
                return None;
            }
        };

        // Validate implement model config exists
        let implement_entry = match anycli_config.configs.get(&self.implement_config) {
            Some(entry) => entry.clone(),
            None => {
                tracing::error!("Implement config '{}' not found", self.implement_config);
                send_error(
                    &sess,
                    &ctx,
                    format!(
                        "Alloy Agent failed: implement config '{}' not found",
                        self.implement_config
                    ),
                )
                .await;
                return None;
            }
        };

        // Log the loaded config entries for debugging
        tracing::info!(
            analyze_config = %self.analyze_config,
            analyze_provider_type = ?analyze_entry.provider_type,
            analyze_model = %analyze_entry.model,
            analyze_endpoint = ?analyze_entry.endpoint,
            analyze_env_key = ?analyze_entry.env_key,
            analyze_wire_api = ?analyze_entry.wire_api,
            "Loaded analyze config entry"
        );
        tracing::info!(
            implement_config = %self.implement_config,
            implement_provider_type = ?implement_entry.provider_type,
            implement_model = %implement_entry.model,
            implement_endpoint = ?implement_entry.endpoint,
            implement_env_key = ?implement_entry.env_key,
            implement_wire_api = ?implement_entry.wire_api,
            "Loaded implement config entry"
        );

        // Notify UI that analyze phase is starting
        send_background_event(
            &sess,
            &ctx,
            format!(
                "Alloy Analyze Phase: Using {} ({}) to research and create specs...",
                self.analyze_config, analyze_entry.model
            ),
        )
        .await;

        // ========== PHASE 1: ANALYZE ==========
        // The analyze phase runs as a sub-agent to research the codebase and create specs.
        let specs = match run_analyze_phase(
            session.clone(),
            ctx.clone(),
            input.clone(),
            &analyze_entry,
            &self.analyze_config,
            cancellation_token.child_token(),
        )
        .await
        {
            Some(specs) if !specs.trim().is_empty() => specs,
            Some(_) => {
                tracing::warn!("Analyze phase returned empty specs");
                send_error(
                    &sess,
                    &ctx,
                    "Alloy analyze phase completed but returned no specifications".to_string(),
                )
                .await;
                return None;
            }
            None => {
                tracing::warn!("Analyze phase returned no specs or was cancelled");
                return None;
            }
        };

        if cancellation_token.is_cancelled() {
            return None;
        }

        // Record the specs as a visible message in the conversation
        sess.record_response_item_and_emit_turn_item(
            ctx.as_ref(),
            ResponseItem::Message {
                id: Some("alloy:specs".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: format!(
                        "## Alloy Specifications\n\n<details>\n<summary>Specs from {} analysis (click to expand)</summary>\n\n{}\n\n</details>",
                        self.analyze_config,
                        specs
                    ),
                }],
            },
        )
        .await;

        // Notify UI that implement phase is starting
        send_background_event(
            &sess,
            &ctx,
            format!(
                "Alloy Implement Phase: Using {} ({}) to execute specs...",
                self.implement_config, implement_entry.model
            ),
        )
        .await;

        // ========== PHASE 2: IMPLEMENT ==========
        // The implement phase runs with the implementation model, receiving the specs
        // and executing them with full tool access.
        let result = run_implement_phase(
            session.clone(),
            ctx.clone(),
            specs,
            &implement_entry,
            &self.implement_config,
            cancellation_token.child_token(),
        )
        .await;

        // Notify completion
        send_background_event(&sess, &ctx, "Alloy Agent completed both phases".to_string()).await;

        result
    }
}

/// Helper to send an error event to the UI
async fn send_error(sess: &Session, ctx: &TurnContext, message: String) {
    sess.send_event(
        ctx,
        EventMsg::Error(codex_protocol::protocol::ErrorEvent {
            message,
            codex_error_info: None,
        }),
    )
    .await;
}

/// Helper to send a background event to the UI
async fn send_background_event(sess: &Session, ctx: &TurnContext, message: String) {
    sess.send_event(
        ctx,
        EventMsg::BackgroundEvent(codex_protocol::protocol::BackgroundEventEvent { message }),
    )
    .await;
}

/// Run the analyze phase as a sub-agent.
/// Returns the specifications text if successful.
async fn run_analyze_phase(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    input: Vec<UserInput>,
    analyze_entry: &crate::anycli::config::ConfigEntry,
    config_name: &str,
    cancellation_token: CancellationToken,
) -> Option<String> {
    let base_config = ctx.client.config();
    let mut analyze_config = base_config.as_ref().clone();

    // Configure for analyze phase
    analyze_config.model = analyze_entry.model.clone();
    analyze_config.model_provider = analyze_entry.to_model_provider_info(config_name);
    analyze_config.base_instructions = Some(ALLOY_ANALYZE_PROMPT.to_string());

    let sess = session.clone_session();
    let auth_manager = session.auth_manager();

    // Log detailed provider configuration for debugging
    tracing::info!(
        model = %analyze_config.model,
        provider_name = %analyze_config.model_provider.name,
        provider_base_url = ?analyze_config.model_provider.base_url,
        provider_wire_api = ?analyze_config.model_provider.wire_api,
        provider_env_key = ?analyze_config.model_provider.env_key,
        provider_requires_openai_auth = %analyze_config.model_provider.requires_openai_auth,
        "Starting analyze sub-agent with provider config"
    );

    // Spawn the analyze sub-agent
    let codex = match run_codex_conversation_one_shot(
        analyze_config,
        auth_manager,
        session.models_manager(),
        input,
        session.clone_session(),
        ctx.clone(),
        cancellation_token.clone(),
        Some(InitialHistory::New),
    )
    .await
    {
        Ok(c) => {
            tracing::info!("Analyze sub-agent started successfully");
            c
        }
        Err(e) => {
            tracing::error!("Failed to start analyze phase: {e}");
            send_error(
                &sess,
                &ctx,
                format!("Alloy analyze phase failed to start: {e}"),
            )
            .await;
            return None;
        }
    };

    // Process events from the analyze sub-agent (forward research progress, extract specs)
    process_subagent_events(
        codex,
        sess,
        ctx,
        cancellation_token,
        "Analyze",
        true, // Filter events for analyze phase (only forward research progress)
    )
    .await
}

/// Run the implement phase as a sub-agent.
/// Returns the final agent message if successful.
async fn run_implement_phase(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    specs: String,
    implement_entry: &crate::anycli::config::ConfigEntry,
    config_name: &str,
    cancellation_token: CancellationToken,
) -> Option<String> {
    let base_config = ctx.client.config();
    let mut implement_config = base_config.as_ref().clone();

    // Configure for implement phase
    implement_config.model = implement_entry.model.clone();
    implement_config.model_provider = implement_entry.to_model_provider_info(config_name);

    // Combine the implementation prompt with the specs
    let combined_instructions = format!(
        "{ALLOY_IMPLEMENT_PROMPT}\n\n---\n\n## Specifications to Implement\n\nThe following specifications were created by the analyze agent after researching the codebase. Execute them precisely:\n\n{specs}"
    );
    implement_config.base_instructions = Some(combined_instructions);

    let sess = session.clone_session();
    let auth_manager = session.auth_manager();

    // Log detailed provider configuration for debugging
    tracing::info!(
        model = %implement_config.model,
        provider_name = %implement_config.model_provider.name,
        provider_base_url = ?implement_config.model_provider.base_url,
        provider_wire_api = ?implement_config.model_provider.wire_api,
        provider_env_key = ?implement_config.model_provider.env_key,
        provider_requires_openai_auth = %implement_config.model_provider.requires_openai_auth,
        "Starting implement sub-agent with provider config"
    );

    // Create input for the implement phase
    let implement_input = vec![UserInput::Text {
        text: "Execute the implementation specifications above. Follow each step precisely, verify files exist before modifying them, and report your progress.".to_string(),
    }];

    // Spawn the implement sub-agent
    let codex = match run_codex_conversation_one_shot(
        implement_config,
        auth_manager,
        session.models_manager(),
        implement_input,
        session.clone_session(),
        ctx.clone(),
        cancellation_token.clone(),
        Some(InitialHistory::New),
    )
    .await
    {
        Ok(c) => {
            tracing::info!("Implement sub-agent started successfully");
            c
        }
        Err(e) => {
            tracing::error!("Failed to start implement phase: {e}");
            send_error(
                &sess,
                &ctx,
                format!("Alloy implement phase failed to start: {e}"),
            )
            .await;
            return None;
        }
    };

    // Process events from the implement sub-agent (forward ALL events)
    process_subagent_events(
        codex,
        sess,
        ctx,
        cancellation_token,
        "Implement",
        false, // Don't filter - forward all events for implement phase
    )
    .await
}

/// Process events from a sub-agent and optionally forward them to the parent session.
///
/// Args:
/// - filter_events: If true, only forward research-related events (for analyze phase).
///   If false, forward all events (for implement phase).
async fn process_subagent_events(
    codex: Codex,
    sess: Arc<Session>,
    ctx: Arc<TurnContext>,
    cancellation_token: CancellationToken,
    phase_name: &str,
    filter_events: bool,
) -> Option<String> {
    let mut event_count = 0u32;
    let mut last_event_time = Instant::now();
    let initial_timeout = Duration::from_secs(INITIAL_TIMEOUT_SECS);
    let event_timeout = Duration::from_secs(EVENT_TIMEOUT_SECS);

    tracing::info!("Processing {} phase events", phase_name);

    loop {
        let current_timeout = if event_count == 0 {
            initial_timeout
        } else {
            event_timeout
        };

        let event = tokio::select! {
            biased;
            _ = cancellation_token.cancelled() => {
                tracing::info!(event_count, "{} phase cancelled", phase_name);
                return None;
            }
            result = timeout(current_timeout, codex.next_event()) => {
                match result {
                    Ok(Ok(event)) => event,
                    Ok(Err(_)) => {
                        tracing::warn!(event_count, "{} phase: channel closed", phase_name);
                        return None;
                    }
                    Err(_) => {
                        let elapsed = last_event_time.elapsed();
                        tracing::error!(
                            event_count,
                            elapsed_secs = elapsed.as_secs(),
                            "{} phase timed out", phase_name
                        );
                        send_error(
                            &sess,
                            &ctx,
                            format!(
                                "Alloy {} phase timed out after {}s ({} events received)",
                                phase_name.to_lowercase(),
                                elapsed.as_secs(),
                                event_count
                            ),
                        )
                        .await;
                        return None;
                    }
                }
            }
        };

        event_count += 1;
        last_event_time = Instant::now();

        // Handle completion and abort events
        match &event.msg {
            EventMsg::TaskComplete(tc) => {
                tracing::info!(
                    event_count,
                    has_message = tc.last_agent_message.is_some(),
                    "{} phase: task complete",
                    phase_name
                );
                return tc.last_agent_message.clone();
            }
            EventMsg::TurnAborted(reason) => {
                tracing::warn!(event_count, ?reason, "{} phase: aborted", phase_name);
                return None;
            }
            _ => {}
        }

        // Decide whether to forward this event
        let should_forward = if filter_events {
            // For analyze phase: only forward research progress and errors
            matches!(
                &event.msg,
                EventMsg::TaskStarted(_)
                    | EventMsg::Error(_)
                    | EventMsg::StreamError(_)
                    | EventMsg::Warning(_)
                    | EventMsg::TokenCount(_)
                    | EventMsg::ExecCommandBegin(_)
                    | EventMsg::ExecCommandOutputDelta(_)
                    | EventMsg::ExecCommandEnd(_)
                    | EventMsg::McpToolCallBegin(_)
                    | EventMsg::McpToolCallEnd(_)
            )
        } else {
            // For implement phase: forward all events except SessionConfigured
            !matches!(&event.msg, EventMsg::SessionConfigured(_))
        };

        if should_forward {
            // Log significant events
            match &event.msg {
                EventMsg::TaskStarted(_) => {
                    tracing::info!(event_count, "{} phase: task started", phase_name);
                }
                EventMsg::Error(e) => {
                    tracing::error!(event_count, error = %e.message, "{} phase: error", phase_name);
                }
                EventMsg::StreamError(se) => {
                    tracing::error!(event_count, error = %se.message, "{} phase: stream error", phase_name);
                }
                EventMsg::AgentMessage(am) => {
                    tracing::debug!(
                        event_count,
                        len = am.message.len(),
                        "{} phase: agent message",
                        phase_name
                    );
                }
                _ => {}
            }
            sess.send_event(ctx.as_ref(), event.msg).await;
        }
    }
}
