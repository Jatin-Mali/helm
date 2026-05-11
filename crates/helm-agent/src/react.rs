//! HELM ReAct loop implementation.

use std::{env, path::Path, sync::Arc};

use futures::future::join_all;
use helm_core::{
    Capability, ContentBlock, HelmError, ProviderError, Taint, ValidationError, Validator,
};
use helm_memory::{AuditEventInput, EpisodeOutcome, MemoryStore, StepRole, stable_hash_hex};
use helm_providers::{ChatRequest, ChatResponse, Provider, ProviderQuirks, pricing_for, StopReason, quirks_for};
use helm_tools::{ToolContext, ToolRegistry};
use serde_json::Value;
use tracing::{debug, info, instrument, trace, warn};

use crate::{
    budget::{Budget, BudgetTracker},
    context_window,
    parser::{ResponseFormat, parse_tool_calls},
    plan_cache::PlanCache,
    supervisor::EvidenceVerifier,
};

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are HELM, a Linux operations agent. The user has given you a task.

Tools:
- shell: run commands. Two modes:
    mode="exec" (default): runs `command` with `args` literally — no shell, no expansion, no pipes. Use this when args come from data or untrusted input.
    mode="shell": runs the full `command` string through `bash -c` — supports pipes, $(...), redirection, globbing. Use this for one-liners and command composition.
  Prefer mode="shell" when you need to compose commands. Prefer mode="exec" when running a single binary with literal arguments.
  You can redirect stdout/stderr to a file with `redirect_stdout_to` / `redirect_stderr_to` in either mode. Prefer this over running a command and then writing its output via fs_write — it's one step, atomic, and avoids the model copying output by hand.

- fs_read: read a file. Returns text or base64. Reading is safe and cheap; verify file contents after writing.

- fs_write: write literal bytes to a file. **fs_write does NOT execute shell or expand $(...) or backticks. The `content` field is written verbatim.** If you want a file to contain the output of a command, use shell with `redirect_stdout_to`, not fs_write with `$(...)` in the content.

- process/service/package/disk/network/logs: typed Linux system-control tools. Prefer these over raw shell for process, systemd, package-manager, disk, port/network, and journalctl tasks because their inputs are stricter and easier to verify.

Execution policy:
1. Plan first: before touching the machine, decide the shortest safe path and list only the next concrete checks you need.
2. Prefer one reliable typed tool call over many speculative shell commands.
3. For disk-capacity/root-cause tasks, start with `disk df` for the exact path, then `disk du` with a small limit on the mounted filesystem, then `disk largest_files` only if directory totals do not explain the issue. Avoid raw `du /home/*` and unbounded `find /home` commands.
4. After each tool call, inspect the result before continuing. If a tool returns partial output or a timeout, narrow the path and retry with a smaller scope instead of repeating the same broad scan.
5. When the task is done, give a short summary of what you found, the root cause if known, and the safest fix.

Be precise. Do not fabricate file contents or command output. If a tool returns an error, address it or tell the user honestly. If you wrote a file, read it back to verify before declaring success."#;

/// Maximum corrective ToolResult messages sent per episode before giving up.
const MAX_CORRECTIONS: u32 = 2;

/// ReAct agent that coordinates a provider, tools, memory, and a run budget.
pub struct ReactAgent {
    provider: Box<dyn Provider>,
    tools: ToolRegistry,
    memory: Arc<MemoryStore>,
    budget: Budget,
    model: String,
    system_prompt: String,
    tool_context: ToolContext,
    quirks: ProviderQuirks,
    forced_initial_taint: Option<Taint>,
    cancel_token: Option<crate::cancel::CancellationToken>,
    plan_cache: Option<Arc<PlanCache>>,
}

/// Live event emitted while an agent run is executing.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// A new run was created in memory.
    RunStarted {
        /// Episode UUID for this run.
        episode_id: String,
        /// User goal for the run.
        goal: String,
    },
    /// A provider request is about to be sent.
    ProviderCallStarted {
        /// Zero-based provider iteration.
        iteration: u32,
        /// Provider name.
        provider: String,
        /// Model id.
        model: String,
    },
    /// A provider response was received.
    ProviderCallFinished {
        /// One-based provider iteration.
        iteration: u32,
        /// Provider stop reason.
        stop_reason: StopReason,
        /// Input tokens reported by the provider.
        tokens_in: u32,
        /// Output tokens reported by the provider.
        tokens_out: u32,
    },
    /// Assistant text became available.
    AssistantText {
        /// Full text block content.
        text: String,
    },
    /// A chunk of assistant text for progressive rendering (fake-streaming).
    TextDelta {
        /// Partial text chunk (up to 64 chars).
        chunk: String,
    },
    /// A tool call was parsed from native or recovered output.
    ToolCallParsed {
        /// Tool-use id.
        id: String,
        /// Tool name.
        name: String,
        /// Tool input.
        input: Value,
    },
    /// A tool call passed schema validation and authorization.
    ToolCallValidated {
        /// Tool-use id.
        id: String,
        /// Tool name.
        name: String,
    },
    /// A tool call is about to execute.
    ToolCallStarted {
        /// Tool-use id.
        id: String,
        /// Tool name.
        name: String,
    },
    /// A tool call finished.
    ToolCallFinished {
        /// Tool-use id.
        id: String,
        /// Tool name.
        name: String,
        /// Whether the tool reported success.
        success: bool,
        /// Human-readable result summary.
        content: String,
    },
    /// A tool call was denied by the capability gate.
    ToolCallDenied {
        /// Tool-use id.
        id: String,
        /// Tool name.
        name: String,
        /// Denial reason.
        reason: String,
    },
    /// A permission would be required before execution can proceed.
    PermissionRequested {
        /// Capability needed by the tool.
        capability: Capability,
        /// Tool name.
        tool_name: String,
        /// Taint at time of request.
        taint: String,
    },
    /// A plan was found in the cache — 0 planner tokens used.
    PlanCacheHit {
        /// Normalized goal hash that matched.
        goal_hash: String,
        /// Number of steps in the cached plan.
        steps: u32,
    },
    /// A text-format tool call was recovered.
    FormatRecoveryUsed {
        /// Recovery format label.
        format: String,
    },
    /// A corrective tool result was sent back to the model.
    CorrectionUsed {
        /// Number of corrections used so far.
        count: u32,
        /// Tool name that needed correction.
        tool_name: String,
    },
    /// Post-condition verification found a warning.
    PostconditionWarning {
        /// Warning text.
        warning: String,
    },
    /// A skill was suggested for the current goal.
    SkillSuggested {
        /// Skill identifier.
        skill_id: String,
        /// Skill name.
        skill_name: String,
        /// Tool sequence for the skill.
        tool_sequence: Vec<String>,
        /// Confidence score (0.0-1.0).
        confidence: f32,
    },
    /// The run finished successfully or partially.
    RunFinished {
        /// Completed run result.
        result: RunResult,
    },
    /// The run failed before a normal result was available.
    RunFailed {
        /// Episode UUID when available.
        episode_id: Option<String>,
        /// Failure message.
        error: String,
    },
    /// Provider failover occurred during fallback chain execution.
    ProviderFailover {
        /// Provider being switched from.
        from: String,
        /// Provider being switched to.
        to: String,
        /// Reason for the failover.
        reason: String,
    },
    /// Budget warning threshold reached.
    BudgetWarning {
        /// Amount spent in USD.
        spent_usd: f64,
        /// Total budget limit in USD.
        limit_usd: f64,
    },
    /// Budget limit has been exceeded.
    BudgetExceeded {
        /// Amount spent in USD.
        spent_usd: f64,
        /// Total budget limit in USD.
        limit_usd: f64,
    },
    /// Prompt cache hit on system prompt.
    PromptCacheHit {
        /// Number of tokens saved by cache reuse.
        tokens_saved: u32,
    },
    /// Permission denied due to role-based access control.
    PermissionDenied {
        /// Tool name that was denied.
        tool_name: String,
        /// User role that attempted the tool.
        role: String,
        /// Reason for denial.
        reason: String,
    },
    /// Input validation failed (injection detection).
    ValidationFailed {
        /// User input that failed validation.
        input: String,
        /// Validation error reason.
        reason: String,
    },
    /// Breakpoint hit during episode replay.
    BreakpointHit {
        /// Step index where breakpoint triggered.
        step_index: u32,
        /// Tool name at breakpoint.
        tool_name: String,
    },
}

/// Receives live events from a running agent.
pub trait AgentEventSink: Send + Sync {
    /// Emits one event. Implementations should be non-blocking.
    fn emit(&self, event: AgentEvent);
}

/// Event sink that discards all events.
#[derive(Debug, Default)]
pub struct NoopAgentEventSink;

impl AgentEventSink for NoopAgentEventSink {
    fn emit(&self, _event: AgentEvent) {}
}

impl ReactAgent {
    /// Creates an agent rooted at the current process directory.
    pub fn new(
        provider: Box<dyn Provider>,
        tools: ToolRegistry,
        memory: Arc<MemoryStore>,
        budget: Budget,
        model: impl Into<String>,
    ) -> Result<Self, HelmError> {
        let working_dir = env::current_dir()?;
        Ok(Self::with_tool_context(
            provider,
            tools,
            memory,
            budget,
            model,
            ToolContext::new(working_dir),
        ))
    }

    /// Creates an agent with an explicit tool context, primarily for tests.
    pub fn with_tool_context(
        provider: Box<dyn Provider>,
        tools: ToolRegistry,
        memory: Arc<MemoryStore>,
        budget: Budget,
        model: impl Into<String>,
        tool_context: ToolContext,
    ) -> Self {
        let model_str: String = model.into();
        let quirks = quirks_for(provider.name(), &model_str);
        let mut system_prompt = DEFAULT_SYSTEM_PROMPT.to_owned();
        if let Ok(context) = load_project_context(&tool_context.working_dir) {
            if !context.is_empty() {
                system_prompt.push_str("\n\n# Project Context\n\n");
                system_prompt.push_str(&context);
            }
        }

        Self {
            provider,
            tools,
            memory,
            budget,
            model: model_str,
            system_prompt,
            tool_context,
            quirks,
            forced_initial_taint: None,
            cancel_token: None,
            plan_cache: None,
        }
    }

    /// Attaches a cancellation token.  Calling `token.cancel()` from any
    /// thread will cause the run loop to stop at the next iteration boundary.
    pub fn with_cancel_token(mut self, token: crate::cancel::CancellationToken) -> Self {
        self.cancel_token = Some(token);
        self
    }

    /// Attaches a plan cache for goal-hash-based plan reuse.
    pub fn with_plan_cache(mut self, cache: Arc<PlanCache>) -> Self {
        self.plan_cache = Some(cache);
        self
    }

    /// Replaces the default system prompt with a caller-provided prompt.
    pub fn with_system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = system_prompt.into();
        self
    }

    /// Forces the starting taint for this agent run.
    ///
    /// This is used by tests and future external-ingestion entry points. Normal
    /// interactive user tasks start with `Taint::User`.
    pub fn with_initial_taint(mut self, taint: Taint) -> Self {
        self.forced_initial_taint = Some(taint);
        self
    }

    /// Runs one ReAct episode to completion, failure, or partial budget exhaustion.
    pub async fn run(&self, goal: &str) -> Result<RunResult, HelmError> {
        self.run_with_events(goal, &NoopAgentEventSink).await
    }

    /// Runs one ReAct episode and emits live progress events to `sink`.
    #[instrument(skip(self, sink), fields(goal = %goal, provider = %self.provider.name(), model = %self.model))]
    pub async fn run_with_events(
        &self,
        goal: &str,
        sink: &dyn AgentEventSink,
    ) -> Result<RunResult, HelmError> {
        let episode_id = self.memory.start_episode(goal).await?;
        sink.emit(AgentEvent::RunStarted {
            episode_id: episode_id.clone(),
            goal: goal.to_owned(),
        });

        // Validate prompt for injection attacks.
        if let Err(e) = Validator::validate_prompt(goal) {
            let reason = match e {
                ValidationError::PromptInjection(_) => {
                    "Prompt injection pattern detected".to_string()
                }
                ValidationError::ShellInjection(_) => {
                    "Shell injection pattern detected".to_string()
                }
                ValidationError::BlockedUrl(_) => "Blocked URL detected".to_string(),
            };
            sink.emit(AgentEvent::ValidationFailed {
                input: goal.to_owned(),
                reason: reason.clone(),
            });
            return Err(HelmError::ValidationFailed(reason));
        }

        // Check plan cache for exact goal match before planning.
        if let Some(ref cache) = self.plan_cache {
            if let Ok(Some(cached)) = cache.get(goal) {
                info!(
                    "plan cache hit for goal hash {} (hit_count={})",
                    cached.goal_hash, cached.hit_count
                );
                sink.emit(AgentEvent::PlanCacheHit {
                    goal_hash: cached.goal_hash,
                    steps: cached.steps.len() as u32,
                });
                // Return cached plan steps as final message.
                let steps_desc: Vec<String> = cached
                    .steps
                    .iter()
                    .map(|s| format!("{}: {}", s.tool, s.description))
                    .collect();
                let final_message = format!(
                    "[cached plan, {} steps]\n{}\n\nRun with cached plan?",
                    cached.steps.len(),
                    steps_desc.join("\n")
                );
                self.memory
                    .finish_episode(
                        &episode_id,
                        EpisodeOutcome::Partial,
                        Some(&final_message),
                        None,
                    )
                    .await?;
                let result = RunResult {
                    episode_id,
                    final_message,
                    last_assistant_text: None,
                    model_capability_warning: None,
                    corrections_used: 0,
                    format_recovery_used: false,
                    total_turns_summarized: 0,
                    tokens_in: 0,
                    tokens_out: 0,
                    iterations: 0,
                };
                sink.emit(AgentEvent::RunFinished {
                    result: result.clone(),
                });
                return Ok(result);
            }
        }

        let mut step_index = 0_u32;
        let mut messages = vec![helm_core::Message::user(goal)];
        self.memory
            .log_step(
                &episode_id,
                step_index,
                StepRole::User,
                &messages[0].content,
                0,
                0,
            )
            .await?;
        step_index = step_index.saturating_add(1);

        let mut tracker = BudgetTracker::new(self.budget);
        let mut final_message = "(no final message)".to_owned();
        let mut last_assistant_text = None;
        let mut model_capability_warning = None;
        let mut corrections_used: u32 = 0;
        let mut format_recovery_used = false;
        let mut total_turns_summarized: u32 = 0;
        let mut response_format_log: Vec<String> = Vec::new();
        let mut current_taint = self.forced_initial_taint.clone().unwrap_or(Taint::User);

        // Build effective system prompt (with optional quirks addendum).
        let effective_system = build_system_prompt(&self.system_prompt, &self.quirks);
        // Temperature: use quirks override if set, else default 0.0.
        let temperature = self.quirks.force_temperature.unwrap_or(0.0);

        loop {
            if let Err(error) = tracker.check() {
                self.persist_corrections(
                    &episode_id,
                    corrections_used,
                    format_recovery_used,
                    &response_format_log,
                    total_turns_summarized,
                )
                .await?;
                self.memory
                    .finish_episode(
                        &episode_id,
                        EpisodeOutcome::Partial,
                        Some(&final_message),
                        Some(&error.to_string()),
                    )
                    .await?;
                let result = RunResult::from_parts(
                    RunResultParts {
                        episode_id,
                        final_message,
                        last_assistant_text,
                        model_capability_warning,
                        corrections_used,
                        format_recovery_used,
                        total_turns_summarized,
                    },
                    &tracker,
                );
                sink.emit(AgentEvent::RunFinished {
                    result: result.clone(),
                });
                return Ok(result);
            }

            // Cooperative cancellation check — runs at every iteration boundary.
            if self.cancel_token.as_ref().is_some_and(|t| t.is_cancelled()) {
                self.persist_corrections(
                    &episode_id,
                    corrections_used,
                    format_recovery_used,
                    &response_format_log,
                    total_turns_summarized,
                )
                .await?;
                self.memory
                    .finish_episode(
                        &episode_id,
                        EpisodeOutcome::Cancelled,
                        Some(&final_message),
                        None,
                    )
                    .await?;
                let result = RunResult::from_parts(
                    RunResultParts {
                        episode_id,
                        final_message,
                        last_assistant_text,
                        model_capability_warning,
                        corrections_used,
                        format_recovery_used,
                        total_turns_summarized,
                    },
                    &tracker,
                );
                sink.emit(AgentEvent::RunFinished {
                    result: result.clone(),
                });
                return Ok(result);
            }

            let (trimmed_messages, new_summarized) = context_window::trim(&messages);
            total_turns_summarized = total_turns_summarized.saturating_add(new_summarized);
            let request = ChatRequest {
                model: self.model.clone(),
                system: Some(effective_system.clone()),
                messages: trimmed_messages,
                tools: self.tools.schemas(),
                max_tokens: self.budget.max_output_tokens.min(8_192),
                temperature,
            };

            info!(
                provider = self.provider.name(),
                iteration = tracker.iterations(),
                "calling provider"
            );
            sink.emit(AgentEvent::ProviderCallStarted {
                iteration: tracker.iterations(),
                provider: self.provider.name().to_owned(),
                model: self.model.clone(),
            });
            let response = match self.provider.chat(request).await {
                Ok(response) => response,
                Err(error) => {
                    self.persist_corrections(
                        &episode_id,
                        corrections_used,
                        format_recovery_used,
                        &response_format_log,
                        total_turns_summarized,
                    )
                    .await?;
                    self.finish_failure(&episode_id, &final_message, &error.to_string())
                        .await?;
                    sink.emit(AgentEvent::RunFailed {
                        episode_id: Some(episode_id.clone()),
                        error: error.to_string(),
                    });
                    return Err(error.into());
                }
            };
            let iteration = tracker.iterations().saturating_add(1);
            log_provider_response(iteration, &response);
            sink.emit(AgentEvent::ProviderCallFinished {
                iteration,
                stop_reason: response.stop_reason,
                tokens_in: response.usage.input_tokens,
                tokens_out: response.usage.output_tokens,
            });
            // Persist a routing outcome so `helm profile routes` reflects observed behavior.
            let cost_usd = if self.budget.max_cost_usd.is_some() {
                let pricing = pricing_for(self.provider.name(), &self.model);
                let input_cost = (response.usage.input_tokens as f64) * pricing.input_rate / 1_000_000.0;
                let output_cost = (response.usage.output_tokens as f64) * pricing.output_rate / 1_000_000.0;
                input_cost + output_cost
            } else {
                0.0
            };
            let _ = self
                .memory
                .record_routing_outcome(
                    &self.model,
                    Some(self.provider.name()),
                    !matches!(response.stop_reason, StopReason::MaxTokens),
                    0,
                    response.usage.input_tokens,
                    response.usage.output_tokens,
                    cost_usd,
                    Some(&episode_id),
                )
                .await;

            tracker.record_iteration();
            tracker.record_tokens(response.usage.input_tokens, response.usage.output_tokens);

            // Record cost based on provider pricing.
            if self.budget.max_cost_usd.is_some() {
                let pricing = pricing_for(self.provider.name(), &self.model);
                tracker.record_cost(
                    response.usage.input_tokens,
                    response.usage.output_tokens,
                    pricing.input_rate,
                    pricing.output_rate,
                );
            }

            let mut assistant_content = response.content.clone();

            // Layer 3 / format recovery: if provider returned only text with no native
            // tool_use blocks, try to parse tool calls out of the text.
            if response.stop_reason == StopReason::ToolUse && !has_tool_use(&assistant_content) {
                // Provider said tool_use but gave no structured blocks — fall through.
            } else if response.stop_reason == StopReason::EndTurn
                && !has_tool_use(&assistant_content)
            {
                if let Some(recovered) =
                    try_recover_text_tool_calls(&assistant_content, &mut response_format_log)
                {
                    warn!("recovered text-format tool call from EndTurn response");
                    format_recovery_used = true;
                    if let Some(format) = response_format_log.last() {
                        sink.emit(AgentEvent::FormatRecoveryUsed {
                            format: format.clone(),
                        });
                    }
                    assistant_content = recovered;
                    // Treat as ToolUse from here on.
                } else {
                    // No recovery — record format as Text.
                    response_format_log.push(ResponseFormat::Text.as_str().to_owned());
                }
            } else if has_tool_use(&assistant_content) {
                response_format_log.push(ResponseFormat::Native.as_str().to_owned());
            }

            if let Some(text) = extract_optional_text(&assistant_content) {
                sink.emit(AgentEvent::AssistantText { text: text.clone() });
                // Emit TextDelta chunks for progressive rendering.
                for chunk in text.as_bytes().chunks(64) {
                    if let Ok(s) = std::str::from_utf8(chunk) {
                        sink.emit(AgentEvent::TextDelta {
                            chunk: s.to_owned(),
                        });
                    }
                }
                last_assistant_text = Some(text);
            }
            for block in &assistant_content {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    sink.emit(AgentEvent::ToolCallParsed {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    });
                }
            }
            messages.push(helm_core::Message::assistant(assistant_content.clone()));
            self.memory
                .log_step(
                    &episode_id,
                    step_index,
                    StepRole::Assistant,
                    &assistant_content,
                    response.usage.input_tokens,
                    response.usage.output_tokens,
                )
                .await?;
            step_index = step_index.saturating_add(1);

            let detected_capability_warning = if response.stop_reason == StopReason::EndTurn
                && !has_tool_use(&assistant_content)
            {
                detect_tool_shaped_text(&assistant_content)
            } else {
                None
            };
            if let Some(warning) = &detected_capability_warning {
                warn!(warning = warning.as_str(), "model emitted tool-shaped text");
                model_capability_warning = Some(warning.clone());
                self.memory
                    .set_model_capability_warning(&episode_id, warning)
                    .await?;
            }

            let effective_stop = if has_tool_use(&assistant_content) {
                StopReason::ToolUse
            } else {
                response.stop_reason
            };

            match effective_stop {
                StopReason::EndTurn | StopReason::StopSequence => {
                    final_message = extract_text(&assistant_content);

                    // Layer 4: post-condition verification.
                    let pc_warnings =
                        verify_postconditions(goal, &messages, &self.tool_context.working_dir);
                    for warning in &pc_warnings {
                        warn!(
                            postcondition = warning.as_str(),
                            "postcondition check failed"
                        );
                        sink.emit(AgentEvent::PostconditionWarning {
                            warning: warning.clone(),
                        });
                    }

                    self.persist_corrections(
                        &episode_id,
                        corrections_used,
                        format_recovery_used,
                        &response_format_log,
                        total_turns_summarized,
                    )
                    .await?;

                    if !pc_warnings.is_empty() {
                        let warning = pc_warnings.join("\n");
                        self.memory
                            .finish_episode(
                                &episode_id,
                                EpisodeOutcome::Failure,
                                Some(&final_message),
                                Some(&warning),
                            )
                            .await?;
                        let result = RunResult::from_parts(
                            RunResultParts {
                                episode_id,
                                final_message,
                                last_assistant_text,
                                model_capability_warning,
                                corrections_used,
                                format_recovery_used,
                                total_turns_summarized,
                            },
                            &tracker,
                        );
                        sink.emit(AgentEvent::RunFinished {
                            result: result.clone(),
                        });
                        return Ok(result);
                    }

                    if let Some(warning) = detected_capability_warning {
                        self.memory
                            .finish_episode(
                                &episode_id,
                                EpisodeOutcome::Failure,
                                Some(&final_message),
                                Some(&warning),
                            )
                            .await?;
                        let result = RunResult::from_parts(
                            RunResultParts {
                                episode_id,
                                final_message,
                                last_assistant_text,
                                model_capability_warning,
                                corrections_used,
                                format_recovery_used,
                                total_turns_summarized,
                            },
                            &tracker,
                        );
                        sink.emit(AgentEvent::RunFinished {
                            result: result.clone(),
                        });
                        return Ok(result);
                    }
                    self.memory
                        .finish_episode(
                            &episode_id,
                            EpisodeOutcome::Success,
                            Some(&final_message),
                            None,
                        )
                        .await?;
                    let result = RunResult::from_parts(
                        RunResultParts {
                            episode_id,
                            final_message,
                            last_assistant_text,
                            model_capability_warning,
                            corrections_used,
                            format_recovery_used,
                            total_turns_summarized,
                        },
                        &tracker,
                    );
                    sink.emit(AgentEvent::RunFinished {
                        result: result.clone(),
                    });
                    return Ok(result);
                }
                StopReason::MaxTokens => {
                    final_message = extract_text(&assistant_content);
                    self.persist_corrections(
                        &episode_id,
                        corrections_used,
                        format_recovery_used,
                        &response_format_log,
                        total_turns_summarized,
                    )
                    .await?;
                    self.memory
                        .finish_episode(
                            &episode_id,
                            EpisodeOutcome::Partial,
                            Some(&final_message),
                            Some("provider stopped at max_tokens"),
                        )
                        .await?;
                    let result = RunResult::from_parts(
                        RunResultParts {
                            episode_id,
                            final_message,
                            last_assistant_text,
                            model_capability_warning,
                            corrections_used,
                            format_recovery_used,
                            total_turns_summarized,
                        },
                        &tracker,
                    );
                    sink.emit(AgentEvent::RunFinished {
                        result: result.clone(),
                    });
                    return Ok(result);
                }
                StopReason::ToolUse => {
                    let tool_results = self
                        .execute_tool_uses(
                            &episode_id,
                            &assistant_content,
                            &mut corrections_used,
                            &mut current_taint,
                            sink,
                        )
                        .await;
                    if tool_results.is_empty() {
                        let error = ProviderError::MalformedResponse(
                            "stop_reason was tool_use but no tool_use content blocks were present"
                                .to_owned(),
                        );
                        self.persist_corrections(
                            &episode_id,
                            corrections_used,
                            format_recovery_used,
                            &response_format_log,
                            total_turns_summarized,
                        )
                        .await?;
                        self.finish_failure(&episode_id, &final_message, &error.to_string())
                            .await?;
                        sink.emit(AgentEvent::RunFailed {
                            episode_id: Some(episode_id.clone()),
                            error: error.to_string(),
                        });
                        return Err(error.into());
                    }
                    let tool_message = helm_core::Message::tool_results(tool_results.clone());
                    messages.push(tool_message);
                    self.memory
                        .log_step(&episode_id, step_index, StepRole::Tool, &tool_results, 0, 0)
                        .await?;
                    step_index = step_index.saturating_add(1);
                }
            }
        }
    }

    async fn execute_tool_uses(
        &self,
        episode_id: &str,
        assistant_content: &[ContentBlock],
        corrections_used: &mut u32,
        current_taint: &mut Taint,
        sink: &dyn AgentEventSink,
    ) -> Vec<ContentBlock> {
        let tool_uses: Vec<(&str, &str, &Value)> = assistant_content
            .iter()
            .filter_map(|b| {
                if let ContentBlock::ToolUse { id, name, input } = b {
                    Some((id.as_str(), name.as_str(), input))
                } else {
                    None
                }
            })
            .collect();

        if tool_uses.is_empty() {
            return Vec::new();
        }

        let taint_snapshot = current_taint.clone();
        let corrections_budget = *corrections_used;

        let futures: Vec<_> = tool_uses
            .iter()
            .map(|(id, name, input)| {
                self.execute_single_tool(
                    episode_id,
                    id,
                    name,
                    input,
                    corrections_budget,
                    taint_snapshot.clone(),
                    sink,
                )
            })
            .collect();

        let batch = join_all(futures).await;

        let mut results = Vec::new();
        for (tool_result, taint_delta, corrections_delta) in batch {
            *current_taint = current_taint.escalate(&taint_delta);
            *corrections_used = corrections_used.saturating_add(corrections_delta);
            results.push(tool_result);
        }
        results
    }

    /// Execute one tool call and return `(result, taint_delta, corrections_delta)`.
    #[allow(clippy::too_many_arguments)]
    async fn execute_single_tool(
        &self,
        episode_id: &str,
        id: &str,
        name: &str,
        input: &Value,
        corrections_budget: u32,
        taint_snapshot: Taint,
        sink: &dyn AgentEventSink,
    ) -> (ContentBlock, Taint, u32) {
        debug!(tool = name, tool_use_id = id, "executing tool");
        let capability = self.tools.required_capability(name, input);
        let authorization = self.authorize_tool_call(capability, &taint_snapshot).await;
        let mut corrections_delta: u32 = 0;

        let result = match authorization {
            Ok(Authorization::Allowed { grant_id }) => {
                sink.emit(AgentEvent::ToolCallValidated {
                    id: id.to_owned(),
                    name: name.to_owned(),
                });
                sink.emit(AgentEvent::ToolCallStarted {
                    id: id.to_owned(),
                    name: name.to_owned(),
                });
                let result = self
                    .tools
                    .execute(name, input.clone(), &self.tool_context)
                    .await;
                let (decision, output_hash) = match &result {
                    Ok(output) => ("allow", stable_hash_hex(&output.content)),
                    Err(error) => ("allow", stable_hash_hex(&error.to_string())),
                };
                self.audit_tool_event(ToolAudit {
                    episode_id,
                    tool_name: name,
                    input,
                    capability,
                    taint: &taint_snapshot,
                    decision,
                    output_hash: &output_hash,
                })
                .await;
                if let Some(grant_id) = grant_id {
                    if let Err(error) = self.memory.consume_grant_if_once(&grant_id).await {
                        warn!(error = %error, "failed to consume one-shot grant");
                    }
                }
                result
            }
            Ok(Authorization::Denied { reason }) => {
                sink.emit(AgentEvent::PermissionRequested {
                    capability,
                    tool_name: name.to_owned(),
                    taint: format!("{:?}", taint_snapshot),
                });
                sink.emit(AgentEvent::ToolCallDenied {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    reason: reason.clone(),
                });
                let denial_hash = stable_hash_hex(&reason);
                self.audit_tool_event(ToolAudit {
                    episode_id,
                    tool_name: name,
                    input,
                    capability,
                    taint: &taint_snapshot,
                    decision: "deny",
                    output_hash: &denial_hash,
                })
                .await;
                Err(helm_tools::ToolError::InvalidInput(reason))
            }
            Err(error) => Err(helm_tools::ToolError::Other(error.to_string())),
        };

        let tool_result = match result {
            Ok(output) => ContentBlock::ToolResult {
                tool_use_id: id.to_owned(),
                content: output.content,
                is_error: !output.success,
            },
            Err(error) => {
                let is_validation_error = matches!(error, helm_tools::ToolError::InvalidInput(_));
                let content = if is_validation_error && corrections_budget < MAX_CORRECTIONS {
                    corrections_delta = 1;
                    let schema_hint = build_correction_hint(name, &self.tools, &error.to_string());
                    warn!(
                        tool = name,
                        corrections_used = corrections_budget + 1,
                        "sending corrective tool result"
                    );
                    sink.emit(AgentEvent::CorrectionUsed {
                        count: corrections_budget + 1,
                        tool_name: name.to_owned(),
                    });
                    schema_hint
                } else {
                    error.to_string()
                };
                sink.emit(AgentEvent::ToolCallFinished {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    success: false,
                    content: content.clone(),
                });
                ContentBlock::ToolResult {
                    tool_use_id: id.to_owned(),
                    content,
                    is_error: true,
                }
            }
        };

        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &tool_result
        {
            if !*is_error {
                sink.emit(AgentEvent::ToolCallFinished {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    success: true,
                    content: content.clone(),
                });
            }
        }

        let taint_delta = taint_after_tool(name);
        (tool_result, taint_delta, corrections_delta)
    }

    async fn authorize_tool_call(
        &self,
        capability: Capability,
        taint: &Taint,
    ) -> Result<Authorization, HelmError> {
        if self.budget.read_only && capability.is_write() {
            return Ok(Authorization::Denied {
                reason: format!(
                    "permission denied: capability '{capability}' requires write access but agent is running in read-only mode"
                ),
            });
        }
        if self.budget.auto_approve {
            return Ok(Authorization::Allowed { grant_id: None });
        }
        let needs_fresh =
            taint.is_external() && capability.requires_fresh_grant_for_external_taint();
        let needs_grant = capability.requires_grant_by_default() || needs_fresh;
        if !needs_grant {
            return Ok(Authorization::Allowed { grant_id: None });
        }
        let grant = self
            .memory
            .active_capability_grant(capability, needs_fresh)
            .await?;
        match grant {
            Some(grant) => Ok(Authorization::Allowed {
                grant_id: Some(grant.id),
            }),
            None => {
                let freshness = if needs_fresh {
                    " fresh once/session"
                } else {
                    ""
                };
                Ok(Authorization::Denied {
                    reason: format!(
                        "permission denied: capability '{capability}' requires{freshness} user grant. Run `helm permissions grant {capability} --scope once` or approve it from the TUI."
                    ),
                })
            }
        }
    }

    async fn audit_tool_event(&self, audit: ToolAudit<'_>) {
        let input_text =
            serde_json::to_string(audit.input).unwrap_or_else(|_| "<invalid-json>".to_owned());
        let event = AuditEventInput {
            episode_id: Some(audit.episode_id.to_owned()),
            tool_name: audit.tool_name.to_owned(),
            input_hash: stable_hash_hex(&input_text),
            output_hash: audit.output_hash.to_owned(),
            capability: audit.capability,
            taint: audit.taint.clone(),
            cwd: self.tool_context.working_dir.display().to_string(),
            decision: audit.decision.to_owned(),
        };
        if let Err(error) = self.memory.append_audit_event(event).await {
            warn!(error = %error, "failed to append audit event");
        }
    }

    async fn finish_failure(
        &self,
        episode_id: &str,
        final_message: &str,
        error: &str,
    ) -> Result<(), HelmError> {
        self.memory
            .finish_episode(
                episode_id,
                EpisodeOutcome::Failure,
                Some(final_message),
                Some(error),
            )
            .await?;
        Ok(())
    }

    async fn persist_corrections(
        &self,
        episode_id: &str,
        corrections_used: u32,
        format_recovery_used: bool,
        response_format_log: &[String],
        total_turns_summarized: u32,
    ) -> Result<(), HelmError> {
        let log_json = if response_format_log.is_empty() {
            None
        } else {
            serde_json::to_string(response_format_log).ok()
        };
        self.memory
            .record_corrections(
                episode_id,
                corrections_used,
                format_recovery_used,
                log_json.as_deref(),
                total_turns_summarized,
            )
            .await?;
        Ok(())
    }
}

fn load_project_context(working_dir: &Path) -> Result<String, std::io::Error> {
    const FILES: &[&str] = &["AGENTS.md", "HELM.md", "CLAUDE.md", ".helm/context.md"];
    const MAX_CHARS: usize = 4_000;

    let mut output = String::new();
    for relative in FILES {
        let path = working_dir.join(relative);
        if !path.is_file() {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        if content.trim().is_empty() {
            continue;
        }
        let remaining = MAX_CHARS.saturating_sub(output.chars().count());
        if remaining == 0 {
            break;
        }
        let heading = format!("## {}\n\n", relative);
        if heading.chars().count() >= remaining {
            break;
        }
        output.push_str(&heading);
        let take = remaining.saturating_sub(heading.chars().count());
        output.extend(content.chars().take(take));
        output.push_str("\n\n");
        if output.chars().count() >= MAX_CHARS {
            break;
        }
    }
    Ok(output)
}

enum Authorization {
    Allowed { grant_id: Option<String> },
    Denied { reason: String },
}

struct ToolAudit<'a> {
    episode_id: &'a str,
    tool_name: &'a str,
    input: &'a Value,
    capability: Capability,
    taint: &'a Taint,
    decision: &'a str,
    output_hash: &'a str,
}

fn taint_after_tool(name: &str) -> Taint {
    if name.starts_with("browser") {
        Taint::External {
            source: name.to_owned(),
        }
    } else {
        Taint::Tool {
            name: name.to_owned(),
        }
    }
}

/// Builds the effective system prompt, optionally appending quirks addendum.
fn build_system_prompt(base: &str, quirks: &ProviderQuirks) -> String {
    match &quirks.system_prompt_addendum {
        Some(addendum) => format!("{base}{addendum}"),
        None => base.to_owned(),
    }
}

/// Builds a corrective hint for the model including the schema of the failed tool.
fn build_correction_hint(tool_name: &str, tools: &ToolRegistry, error: &str) -> String {
    let schema = tools
        .schemas()
        .into_iter()
        .find(|s| s.name == tool_name)
        .map(|s| serde_json::to_string_pretty(&s.input_schema).unwrap_or_else(|_| "{}".to_owned()))
        .unwrap_or_else(|| "{}".to_owned());
    format!(
        "Error: {error}\n\nCorrection required. Expected input schema for `{tool_name}`:\n{schema}\n\nPlease retry with a valid input."
    )
}

/// Try to recover text-format tool calls from a response with no native tool_use blocks.
///
/// Returns `Some(new_content_blocks)` if at least one tool call was recovered.
fn try_recover_text_tool_calls(
    content: &[ContentBlock],
    format_log: &mut Vec<String>,
) -> Option<Vec<ContentBlock>> {
    let text = extract_optional_text(content)?;
    let parsed = parse_tool_calls(&text);
    if parsed.tool_calls.is_empty() {
        return None;
    }
    format_log.push(parsed.format_used.as_str().to_owned());
    let mut new_blocks: Vec<ContentBlock> = Vec::new();
    // Keep any non-text prefix/suffix as a text block.
    if !parsed.residual_text.is_empty() {
        new_blocks.push(ContentBlock::Text(parsed.residual_text));
    }
    for call in parsed.tool_calls {
        new_blocks.push(ContentBlock::ToolUse {
            id: call.id,
            name: call.name,
            input: call.input,
        });
    }
    Some(new_blocks)
}

/// Layer 4: verify post-conditions for written files after EndTurn.
///
/// Scans all assistant messages in the conversation for `fs_write` tool calls,
/// then checks each written path for existence, non-emptiness, and absence of
/// unexpanded shell syntax.
fn verify_postconditions(
    goal: &str,
    messages: &[helm_core::Message],
    working_dir: &Path,
) -> Vec<String> {
    EvidenceVerifier::new(working_dir)
        .verify_goal(goal, messages)
        .problems()
}

/// Result of a completed or partially completed HELM run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    /// Episode UUID stored in memory.
    pub episode_id: String,
    /// Final assistant text or a placeholder if no text was produced.
    pub final_message: String,
    /// Number of provider turns executed.
    pub iterations: u32,
    /// Total provider input tokens recorded.
    pub tokens_in: u32,
    /// Total provider output tokens recorded.
    pub tokens_out: u32,
    /// Last assistant text observed, including partial or warning-producing turns.
    pub last_assistant_text: Option<String>,
    /// Warning recorded when the model appears not to support native tool calling.
    pub model_capability_warning: Option<String>,
    /// Number of corrective ToolResult messages sent during this episode.
    pub corrections_used: u32,
    /// Whether the parser recovered a text-format tool call during this episode.
    pub format_recovery_used: bool,
    /// Number of turns collapsed by the rolling context trimmer.
    pub total_turns_summarized: u32,
}

impl RunResult {
    fn from_parts(parts: RunResultParts, tracker: &BudgetTracker) -> Self {
        Self {
            episode_id: parts.episode_id,
            final_message: parts.final_message,
            iterations: tracker.iterations(),
            tokens_in: tracker.input_tokens(),
            tokens_out: tracker.output_tokens(),
            last_assistant_text: parts.last_assistant_text,
            model_capability_warning: parts.model_capability_warning,
            corrections_used: parts.corrections_used,
            format_recovery_used: parts.format_recovery_used,
            total_turns_summarized: parts.total_turns_summarized,
        }
    }
}

struct RunResultParts {
    episode_id: String,
    final_message: String,
    last_assistant_text: Option<String>,
    model_capability_warning: Option<String>,
    corrections_used: u32,
    format_recovery_used: bool,
    total_turns_summarized: u32,
}

fn extract_text(content: &[ContentBlock]) -> String {
    match extract_optional_text(content) {
        Some(text) => text,
        None => "(no final message)".to_owned(),
    }
}

fn extract_optional_text(content: &[ContentBlock]) -> Option<String> {
    let text = content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() { None } else { Some(text) }
}

fn has_tool_use(content: &[ContentBlock]) -> bool {
    content
        .iter()
        .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
}

/// Returns Some(reason) if the assistant message looks like a model failed
/// to use native tool calls and instead emitted JSON in text.
fn detect_tool_shaped_text(blocks: &[ContentBlock]) -> Option<String> {
    for block in blocks {
        let ContentBlock::Text(text) = block else {
            continue;
        };
        if text_contains_tool_shaped_json(text) {
            return Some("model emitted tool-shaped text instead of native tool call".to_owned());
        }
    }
    None
}

fn text_contains_tool_shaped_json(text: &str) -> bool {
    let mut start = None;
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if start.is_none() {
            if ch == '{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let Some(start_index) = start else {
                        return false;
                    };
                    let end = index + ch.len_utf8();
                    let candidate = &text[start_index..end];
                    if json_candidate_is_tool_shaped(candidate)
                        || loose_candidate_is_tool_shaped(candidate)
                    {
                        return true;
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    false
}

fn json_candidate_is_tool_shaped(candidate: &str) -> bool {
    match serde_json::from_str::<Value>(candidate) {
        Ok(Value::Object(object)) => {
            object.contains_key("name")
                && (object.contains_key("parameters") || object.contains_key("arguments"))
        }
        _ => false,
    }
}

fn loose_candidate_is_tool_shaped(candidate: &str) -> bool {
    candidate.contains("\"name\"")
        && (candidate.contains("\"parameters\"") || candidate.contains("\"arguments\""))
}

fn log_provider_response(iteration: u32, response: &ChatResponse) {
    let total_text_len = total_text_len(&response.content);
    let tool_use_count = tool_use_count(&response.content);
    debug!(
        iteration,
        stop_reason = ?response.stop_reason,
        text_len = total_text_len,
        tool_calls = tool_use_count,
        tokens_in = response.usage.input_tokens,
        tokens_out = response.usage.output_tokens,
        "provider response"
    );
    for (block_index, block) in response.content.iter().enumerate() {
        let (block_type, content) = summarize_block(block);
        let content = helm_core::redact_secrets(&content);
        trace!(
            iteration,
            block_index,
            block_type,
            content = content.as_str(),
            "provider response block"
        );
    }
}

fn total_text_len(content: &[ContentBlock]) -> usize {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.len()),
            _ => None,
        })
        .sum()
}

fn tool_use_count(content: &[ContentBlock]) -> usize {
    content
        .iter()
        .filter(|block| matches!(block, ContentBlock::ToolUse { .. }))
        .count()
}

fn summarize_block(block: &ContentBlock) -> (&'static str, String) {
    match block {
        ContentBlock::Text(text) => ("text", truncate_for_trace(text)),
        ContentBlock::ToolUse { name, input, .. } => {
            let input_text = serde_json::to_string(input)
                .unwrap_or_else(|error| format!("invalid json input: {error}"));
            (
                "tool_use",
                truncate_for_trace(&format!("{name}: {input_text}")),
            )
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => (
            "tool_result",
            truncate_for_trace(&format!("{tool_use_id} error={is_error}: {content}")),
        ),
    }
}

fn truncate_for_trace(text: &str) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(400).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

// ── ResponseFormat display ────────────────────────────────────────────────────

trait ResponseFormatExt {
    fn as_str(&self) -> &'static str;
}

impl ResponseFormatExt for ResponseFormat {
    fn as_str(&self) -> &'static str {
        match self {
            ResponseFormat::Native => "native",
            ResponseFormat::XmlTag => "xml_tag",
            ResponseFormat::FunctionTag => "function_tag",
            ResponseFormat::Pythonic => "pythonic",
            ResponseFormat::BareJson => "bare_json",
            ResponseFormat::Text => "text",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use helm_core::{Capability, ContentBlock, GrantScope, Taint};
    use helm_memory::{EpisodeOutcome, MemoryStore};
    use helm_providers::{ChatResponse, MockProvider, StopReason, Usage};
    use helm_tools::{ToolContext, ToolRegistry};
    use serde_json::json;
    use tempfile::tempdir;
    use tracing::field::{Field, Visit};
    use tracing_subscriber::{Layer, Registry, filter::LevelFilter, layer::Context, prelude::*};

    use crate::budget::Budget;

    use super::{
        AgentEvent, AgentEventSink, ReactAgent, detect_tool_shaped_text, extract_text,
        summarize_block,
    };

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        level: tracing::Level,
        fields: Vec<(String, String)>,
    }

    #[derive(Clone)]
    struct CaptureLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    #[derive(Clone, Default)]
    struct EventCollector {
        events: Arc<Mutex<Vec<AgentEvent>>>,
    }

    impl AgentEventSink for EventCollector {
        fn emit(&self, event: AgentEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[derive(Default)]
    struct FieldCapture {
        fields: Vec<(String, String)>,
    }

    impl Visit for FieldCapture {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .push((field.name().to_owned(), format!("{value:?}")));
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .push((field.name().to_owned(), value.to_string()));
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .push((field.name().to_owned(), value.to_string()));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .push((field.name().to_owned(), value.to_owned()));
        }
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut capture = FieldCapture::default();
            event.record(&mut capture);
            self.events.lock().unwrap().push(CapturedEvent {
                level: *event.metadata().level(),
                fields: capture.fields,
            });
        }
    }

    fn response(content: Vec<ContentBlock>, stop_reason: StopReason) -> ChatResponse {
        ChatResponse {
            id: "msg".to_owned(),
            content,
            stop_reason,
            usage: Usage {
                input_tokens: 2,
                output_tokens: 3,
            },
        }
    }

    async fn agent(
        responses: Vec<ChatResponse>,
        budget: Budget,
    ) -> (tempfile::TempDir, Arc<MemoryStore>, ReactAgent) {
        let dir = tempdir().unwrap();
        let memory = Arc::new(
            MemoryStore::open(&dir.path().join("helm.db"))
                .await
                .unwrap(),
        );
        let agent = ReactAgent::with_tool_context(
            Box::new(MockProvider::new(responses)),
            ToolRegistry::default(),
            Arc::clone(&memory),
            budget,
            "mock",
            ToolContext::new(dir.path().to_path_buf()),
        );
        (dir, memory, agent)
    }

    #[tokio::test]
    async fn single_turn_records_success_happy_path() {
        let (_dir, memory, agent) = agent(
            vec![response(
                vec![ContentBlock::Text("done".to_owned())],
                StopReason::EndTurn,
            )],
            Budget::default(),
        )
        .await;

        let result = agent.run("say done").await.unwrap();
        let episode = memory
            .episode_by_id(&result.episode_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(result.final_message, "done");
        assert_eq!(
            episode.outcome,
            Some(EpisodeOutcome::Success.as_str().to_owned())
        );
    }

    #[tokio::test]
    async fn default_system_prompt_mentions_fs_write_literal_behavior() {
        let dir = tempdir().unwrap();
        let memory = Arc::new(
            MemoryStore::open(&dir.path().join("helm.db"))
                .await
                .unwrap(),
        );
        let provider = MockProvider::new(vec![response(
            vec![ContentBlock::Text("done".to_owned())],
            StopReason::EndTurn,
        )]);
        let captured_provider = provider.clone();
        let agent = ReactAgent::with_tool_context(
            Box::new(provider),
            ToolRegistry::default(),
            Arc::clone(&memory),
            Budget::default(),
            "mock",
            ToolContext::new(dir.path().to_path_buf()),
        );

        agent.run("check prompt").await.unwrap();

        let requests = captured_provider.requests().unwrap();
        let system = requests
            .first()
            .and_then(|request| request.system.as_deref())
            .unwrap();
        assert!(system.contains("fs_write does NOT execute shell"));
    }

    #[tokio::test]
    async fn multi_turn_with_shell_tool() {
        let (_dir, _memory, agent) = agent(
            vec![
                response(
                    vec![ContentBlock::ToolUse {
                        id: "toolu_1".to_owned(),
                        name: "shell".to_owned(),
                        input: json!({"command": "echo", "args": ["hi"]}),
                    }],
                    StopReason::ToolUse,
                ),
                response(
                    vec![ContentBlock::Text("saw hi".to_owned())],
                    StopReason::EndTurn,
                ),
            ],
            Budget::default(),
        )
        .await;

        let result = agent.run("echo hi").await.unwrap();

        assert_eq!(result.final_message, "saw hi");
        assert_eq!(result.iterations, 2);
    }

    #[tokio::test]
    async fn run_with_events_emits_tool_timeline_happy_path() {
        let dir = tempdir().unwrap();
        let memory = Arc::new(
            MemoryStore::open(&dir.path().join("events.db"))
                .await
                .unwrap(),
        );
        memory
            .grant_capability(Capability::ShellExec, GrantScope::Always)
            .await
            .unwrap();
        let provider = MockProvider::new(vec![
            response(
                vec![ContentBlock::ToolUse {
                    id: "tool_1".to_owned(),
                    name: "shell".to_owned(),
                    input: json!({"command": "echo", "args": ["hi"]}),
                }],
                StopReason::ToolUse,
            ),
            response(
                vec![ContentBlock::Text("done".to_owned())],
                StopReason::EndTurn,
            ),
        ]);
        let agent = ReactAgent::with_tool_context(
            Box::new(provider),
            ToolRegistry::default(),
            memory,
            Budget::default(),
            "mock",
            ToolContext::new(dir.path().to_path_buf()),
        );
        let collector = EventCollector::default();

        let result = agent.run_with_events("echo hi", &collector).await.unwrap();
        assert_eq!(result.final_message, "done");

        let events = collector.events.lock().unwrap().clone();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AgentEvent::RunStarted { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AgentEvent::ProviderCallStarted { .. }))
        );
        assert!(events.iter().any(
            |event| matches!(event, AgentEvent::ToolCallParsed { name, .. } if name == "shell")
        ));
        assert!(events.iter().any(
            |event| matches!(event, AgentEvent::ToolCallStarted { name, .. } if name == "shell")
        ));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AgentEvent::ToolCallFinished { success: true, .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AgentEvent::RunFinished { .. }))
        );
    }

    #[tokio::test]
    async fn budget_exhaustion_returns_partial() {
        let budget = Budget {
            max_iterations: 2,
            max_input_tokens: 1_000,
            max_output_tokens: 1_000,
            max_wall_time: Duration::from_secs(60),
            ..Budget::default()
        };
        let responses = vec![
            response(
                vec![ContentBlock::ToolUse {
                    id: "toolu_1".to_owned(),
                    name: "shell".to_owned(),
                    input: json!({"command": "printf", "args": ["one"]}),
                }],
                StopReason::ToolUse,
            ),
            response(
                vec![ContentBlock::ToolUse {
                    id: "toolu_2".to_owned(),
                    name: "shell".to_owned(),
                    input: json!({"command": "printf", "args": ["two"]}),
                }],
                StopReason::ToolUse,
            ),
        ];
        let (_dir, memory, agent) = agent(responses, budget).await;

        let result = agent.run("loop").await.unwrap();
        let episode = memory
            .episode_by_id(&result.episode_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(result.iterations, 2);
        assert_eq!(
            episode.outcome,
            Some(EpisodeOutcome::Partial.as_str().to_owned())
        );
    }

    #[tokio::test]
    async fn tool_error_becomes_error_tool_result_and_continues() {
        let (_dir, memory, agent) = agent(
            vec![
                response(
                    vec![ContentBlock::ToolUse {
                        id: "toolu_bad".to_owned(),
                        name: "shell".to_owned(),
                        input: json!({"args": ["missing command"]}),
                    }],
                    StopReason::ToolUse,
                ),
                response(
                    vec![ContentBlock::Text("handled error".to_owned())],
                    StopReason::EndTurn,
                ),
            ],
            Budget::default(),
        )
        .await;

        let result = agent.run("bad tool").await.unwrap();

        assert_eq!(result.final_message, "handled error");
        assert_eq!(memory.step_count(&result.episode_id).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn validation_error_sends_corrective_hint() {
        // shell requires "command" field; omit it to trigger InvalidInput.
        let (_dir, _memory, agent) = agent(
            vec![
                response(
                    vec![ContentBlock::ToolUse {
                        id: "toolu_1".to_owned(),
                        name: "shell".to_owned(),
                        input: json!({"args": ["missing command"]}),
                    }],
                    StopReason::ToolUse,
                ),
                response(
                    vec![ContentBlock::Text("corrected".to_owned())],
                    StopReason::EndTurn,
                ),
            ],
            Budget::default(),
        )
        .await;

        let result = agent.run("test correction").await.unwrap();
        assert_eq!(result.corrections_used, 1);
    }

    #[tokio::test]
    async fn external_tainted_context_cannot_run_shell_without_fresh_grant() {
        let (_dir, memory, agent) = agent(
            vec![
                response(
                    vec![ContentBlock::ToolUse {
                        id: "toolu_ext".to_owned(),
                        name: "shell".to_owned(),
                        input: json!({"command": "date && uname -a", "mode": "shell"}),
                    }],
                    StopReason::ToolUse,
                ),
                response(
                    vec![ContentBlock::Text("denied safely".to_owned())],
                    StopReason::EndTurn,
                ),
            ],
            Budget::default(),
        )
        .await;
        let agent = agent.with_initial_taint(Taint::External {
            source: "browser".to_owned(),
        });

        let result = agent.run("external tried shell").await.unwrap();
        let steps = memory.get_steps(&result.episode_id).await.unwrap();
        let audit = memory.audit_events(Some(&result.episode_id)).await.unwrap();

        assert_eq!(result.final_message, "denied safely");
        assert!(steps.iter().any(|step| {
            step.content.iter().any(|block| {
                matches!(
                    block,
                    ContentBlock::ToolResult {
                        is_error: true,
                        content,
                        ..
                    } if content.contains("permission denied")
                )
            })
        }));
        assert_eq!(audit[0].decision, "deny");
        assert_eq!(audit[0].capability, Capability::ShellShell);
    }

    #[tokio::test]
    async fn fresh_grant_allows_external_shell_once() {
        let (_dir, memory, agent) = agent(
            vec![
                response(
                    vec![ContentBlock::ToolUse {
                        id: "toolu_ext".to_owned(),
                        name: "shell".to_owned(),
                        input: json!({"command": "printf ok", "mode": "shell"}),
                    }],
                    StopReason::ToolUse,
                ),
                response(
                    vec![ContentBlock::Text("allowed".to_owned())],
                    StopReason::EndTurn,
                ),
            ],
            Budget::default(),
        )
        .await;
        memory
            .grant_capability(Capability::ShellShell, GrantScope::Once)
            .await
            .unwrap();
        let agent = agent.with_initial_taint(Taint::External {
            source: "browser".to_owned(),
        });

        let result = agent.run("external approved shell").await.unwrap();
        let audit = memory.audit_events(Some(&result.episode_id)).await.unwrap();

        assert_eq!(result.final_message, "allowed");
        assert_eq!(audit[0].decision, "allow");
    }

    #[tokio::test]
    async fn tool_shaped_text_records_failure_warning() {
        // Provider emits bare-JSON tool call as text (EndTurn, no native ToolUse blocks).
        // Format recovery should parse it, execute the tool, and then ask the provider again.
        let (_dir, _memory, agent) = agent(
            vec![
                response(
                    vec![ContentBlock::Text(
                        r#"{"name":"shell","parameters":{"command":"echo","args":["hi"]}}"#
                            .to_owned(),
                    )],
                    StopReason::EndTurn,
                ),
                response(
                    vec![ContentBlock::Text("recovered".to_owned())],
                    StopReason::EndTurn,
                ),
            ],
            Budget::default(),
        )
        .await;

        let result = agent.run("echo hi").await.unwrap();
        assert!(result.format_recovery_used);
    }

    #[test]
    fn provider_response_tracing_includes_expected_fields() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(
            CaptureLayer {
                events: Arc::clone(&events),
            }
            .with_filter(LevelFilter::TRACE),
        );
        let dispatch = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, || {
            super::log_provider_response(
                1,
                &response(
                    vec![ContentBlock::Text("trace me".to_owned())],
                    StopReason::EndTurn,
                ),
            );
        });

        let events = events.lock().unwrap().clone();
        let debug_event = events
            .iter()
            .find(|event| {
                event.level == tracing::Level::DEBUG
                    && event
                        .fields
                        .iter()
                        .any(|(name, value)| name == "message" && value == "provider response")
            })
            .unwrap();
        assert_field(debug_event, "iteration", "1");
        assert_field(debug_event, "stop_reason", "EndTurn");
        assert_field(debug_event, "text_len", "8");
        assert_field(debug_event, "tool_calls", "0");
        assert_field(debug_event, "tokens_in", "2");
        assert_field(debug_event, "tokens_out", "3");
        let (block_type, content) = summarize_block(&ContentBlock::Text("trace me".to_owned()));
        assert_eq!(block_type, "text");
        assert_eq!(content, "trace me");
    }

    fn assert_field(event: &CapturedEvent, field: &str, value: &str) {
        assert!(
            event
                .fields
                .iter()
                .any(|(name, actual)| name == field && actual == value),
            "missing {field}={value:?} in {:?}",
            event.fields
        );
    }

    #[test]
    fn detects_actual_broken_tool_json_output() {
        let blocks = [ContentBlock::Text(
            r#"{"name": "fs_write", "parameters": [{"path": "/tmp/test.txt", "content": "x"}]}"#
                .to_owned(),
        )];

        assert!(detect_tool_shaped_text(&blocks).is_some());
    }

    #[test]
    fn ignores_clean_text_response() {
        let blocks = [ContentBlock::Text("done without tools".to_owned())];

        assert!(detect_tool_shaped_text(&blocks).is_none());
    }

    #[test]
    fn ignores_json_config_code_block_edge_case() {
        let blocks = [ContentBlock::Text(
            "```json\n{\"name\":\"app\",\"version\":1}\n```".to_owned(),
        )];

        assert!(detect_tool_shaped_text(&blocks).is_none());
    }

    #[test]
    fn ignores_plain_discussion_of_name() {
        let blocks = [ContentBlock::Text(
            "The field name is important, but no tool call is needed.".to_owned(),
        )];

        assert!(detect_tool_shaped_text(&blocks).is_none());
    }

    #[test]
    fn extract_text_edge_case_no_text() {
        assert_eq!(extract_text(&[]), "(no final message)");
    }

    #[test]
    fn postconditions_fail_when_shell_redirect_file_missing() {
        let dir = tempdir().unwrap();
        let messages = vec![helm_core::Message::assistant(vec![ContentBlock::ToolUse {
            id: "call_1".to_owned(),
            name: "shell".to_owned(),
            input: json!({"command": "date", "redirect_stdout_to": dir.path().join("missing.txt")}),
        }])];

        let warnings =
            super::verify_postconditions("create file with output of date", &messages, dir.path());

        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("does not exist"))
        );
    }

    #[test]
    fn postconditions_fail_when_uname_output_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("out.txt");
        std::fs::write(&path, "Tuesday 05 May 2026").unwrap();
        let messages = vec![helm_core::Message::assistant(vec![ContentBlock::ToolUse {
            id: "call_1".to_owned(),
            name: "shell".to_owned(),
            input: json!({"command": "date", "redirect_stdout_to": path}),
        }])];

        let warnings = super::verify_postconditions(
            "create file with output of date and uname -a",
            &messages,
            dir.path(),
        );

        assert!(warnings.iter().any(|warning| warning.contains("Linux")));
    }

    #[test]
    fn postconditions_pass_for_date_and_uname_output() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("out.txt");
        std::fs::write(&path, "Tuesday 05 May 2026\nLinux PHANTOM").unwrap();
        let messages = vec![helm_core::Message::assistant(vec![ContentBlock::ToolUse {
            id: "call_1".to_owned(),
            name: "shell".to_owned(),
            input: json!({"command": "date && uname -a", "redirect_stdout_to": path}),
        }])];

        let warnings = super::verify_postconditions(
            "create file with output of date and uname -a",
            &messages,
            dir.path(),
        );

        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn project_context_loads_known_files_and_caps_length() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "agents").unwrap();
        std::fs::write(dir.path().join("HELM.md"), "helm").unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "claude").unwrap();
        std::fs::create_dir_all(dir.path().join(".helm")).unwrap();
        std::fs::write(dir.path().join(".helm/context.md"), "x".repeat(5_000)).unwrap();

        let context = super::load_project_context(dir.path()).unwrap();

        assert!(context.contains("AGENTS.md"));
        assert!(context.contains("HELM.md"));
        assert!(context.contains("CLAUDE.md"));
        assert!(context.contains(".helm/context.md"));
        assert!(context.chars().count() <= 4_100);
    }

    #[test]
    fn project_context_ignores_missing_files() {
        let dir = tempdir().unwrap();
        let context = super::load_project_context(dir.path()).unwrap();
        assert!(context.is_empty());
    }

    #[test]
    fn run_result_tracks_corrections_and_recovery() {
        // Verify the new fields exist and default to zero/false.
        let tracker = crate::budget::BudgetTracker::new(Budget::default());
        let result = super::RunResult::from_parts(
            super::RunResultParts {
                episode_id: "ep1".into(),
                final_message: "done".into(),
                last_assistant_text: None,
                model_capability_warning: None,
                corrections_used: 3,
                format_recovery_used: true,
                total_turns_summarized: 0,
            },
            &tracker,
        );
        assert_eq!(result.corrections_used, 3);
        assert!(result.format_recovery_used);
        assert_eq!(result.total_turns_summarized, 0);
    }

    // v1.5 tests

    #[tokio::test]
    async fn text_delta_emitted_for_assistant_text_happy_path() {
        let (_dir, _memory, agent) = agent(
            vec![response(
                vec![ContentBlock::Text("hello world from HELM".to_owned())],
                StopReason::EndTurn,
            )],
            Budget::default(),
        )
        .await;
        let collector = EventCollector::default();
        agent
            .run_with_events("say hello", &collector)
            .await
            .unwrap();
        let events = collector.events.lock().unwrap().clone();
        // At least one TextDelta must be emitted for the assistant response.
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::TextDelta { .. })),
            "expected at least one TextDelta event"
        );
        // All TextDelta chunks concatenated must rebuild the original text.
        let rebuilt: String = events
            .iter()
            .filter_map(|e| {
                if let AgentEvent::TextDelta { chunk } = e {
                    Some(chunk.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(rebuilt, "hello world from HELM");
    }

    #[tokio::test]
    async fn parallel_tool_execution_both_results_collected_happy_path() {
        let dir = tempdir().unwrap();
        let memory = Arc::new(
            MemoryStore::open(&dir.path().join("parallel.db"))
                .await
                .unwrap(),
        );
        memory
            .grant_capability(Capability::ShellExec, GrantScope::Always)
            .await
            .unwrap();
        // Provider returns two tool-use blocks in the same response.
        let agent = ReactAgent::with_tool_context(
            Box::new(MockProvider::new(vec![
                response(
                    vec![
                        ContentBlock::ToolUse {
                            id: "t1".to_owned(),
                            name: "shell".to_owned(),
                            input: json!({"command": "echo", "args": ["a"]}),
                        },
                        ContentBlock::ToolUse {
                            id: "t2".to_owned(),
                            name: "shell".to_owned(),
                            input: json!({"command": "echo", "args": ["b"]}),
                        },
                    ],
                    StopReason::ToolUse,
                ),
                response(
                    vec![ContentBlock::Text("both done".to_owned())],
                    StopReason::EndTurn,
                ),
            ])),
            ToolRegistry::default(),
            Arc::clone(&memory),
            Budget::default(),
            "mock",
            ToolContext::new(dir.path().to_path_buf()),
        );
        let collector = EventCollector::default();
        let result = agent
            .run_with_events("echo a and b", &collector)
            .await
            .unwrap();
        assert_eq!(result.final_message, "both done");
        let events = collector.events.lock().unwrap().clone();
        // Both tool calls must appear as finished-success.
        let finished_ok: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolCallFinished { success: true, .. }))
            .collect();
        assert_eq!(
            finished_ok.len(),
            2,
            "expected two successful ToolCallFinished events"
        );
    }
}
