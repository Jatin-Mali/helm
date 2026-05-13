//! AgentEventSink that prints each event as a newline-delimited JSON line on
//! stdout. Used by `helm run --emit-events` so that a parent process (the
//! local CLI in Q2 agent-on-remote mode, or any external orchestrator) can
//! consume the run as a stream.

use std::sync::Mutex;

use helm_agent::{AgentEvent, AgentEventSink};
use serde_json::{Value, json};

pub struct NdjsonSink {
    writer: Mutex<()>,
}

impl Default for NdjsonSink {
    fn default() -> Self {
        Self {
            writer: Mutex::new(()),
        }
    }
}

impl NdjsonSink {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AgentEventSink for NdjsonSink {
    fn emit(&self, event: AgentEvent) {
        let (name, data) = match event {
            AgentEvent::RunStarted { episode_id, goal } => (
                "run_started",
                json!({ "episode_id": episode_id, "goal": goal }),
            ),
            AgentEvent::ProviderCallStarted {
                iteration,
                provider,
                model,
            } => (
                "provider_call_started",
                json!({ "iteration": iteration, "provider": provider, "model": model }),
            ),
            AgentEvent::ProviderCallFinished {
                iteration,
                stop_reason,
                tokens_in,
                tokens_out,
            } => (
                "provider_call_finished",
                json!({
                    "iteration": iteration,
                    "stop_reason": format!("{stop_reason:?}"),
                    "tokens_in": tokens_in,
                    "tokens_out": tokens_out,
                }),
            ),
            AgentEvent::AssistantText { text } => ("assistant_text", json!({ "text": text })),
            AgentEvent::TextDelta { chunk } => ("text_delta", json!({ "chunk": chunk })),
            AgentEvent::ToolCallParsed { id, name, input } => (
                "tool_call_parsed",
                json!({ "id": id, "name": name, "input": input }),
            ),
            AgentEvent::ToolCallValidated { id, name } => {
                ("tool_call_validated", json!({ "id": id, "name": name }))
            }
            AgentEvent::ToolCallStarted { id, name } => {
                ("tool_started", json!({ "id": id, "name": name }))
            }
            AgentEvent::ToolCallFinished {
                id,
                name,
                success,
                content,
            } => (
                "tool_finished",
                json!({
                    "id": id,
                    "name": name,
                    "success": success,
                    "content": truncate(&content, 4096),
                }),
            ),
            AgentEvent::ToolCallDenied { id, name, reason } => (
                "tool_denied",
                json!({ "id": id, "name": name, "reason": reason }),
            ),
            AgentEvent::PermissionRequested {
                capability,
                tool_name,
                taint,
            } => (
                "permission_requested",
                json!({
                    "capability": capability.to_string(),
                    "tool": tool_name,
                    "taint": taint,
                }),
            ),
            AgentEvent::RunFinished { result } => (
                "run_finished",
                json!({
                    "episode_id": result.episode_id,
                    "final": result.final_message,
                    "iterations": result.iterations,
                    "tokens_in": result.tokens_in,
                    "tokens_out": result.tokens_out,
                }),
            ),
            AgentEvent::RunFailed { episode_id, error } => (
                "run_failed",
                json!({ "episode_id": episode_id, "error": error }),
            ),
            AgentEvent::ProviderFailover { from, to, reason } => (
                "provider_failover",
                json!({ "from": from, "to": to, "reason": reason }),
            ),
            AgentEvent::BudgetWarning {
                spent_usd,
                limit_usd,
            } => (
                "budget_warning",
                json!({ "spent_usd": spent_usd, "limit_usd": limit_usd }),
            ),
            AgentEvent::BudgetExceeded {
                spent_usd,
                limit_usd,
            } => (
                "budget_exceeded",
                json!({ "spent_usd": spent_usd, "limit_usd": limit_usd }),
            ),
            AgentEvent::PromptCacheHit { tokens_saved } => {
                ("prompt_cache_hit", json!({ "tokens_saved": tokens_saved }))
            }
            AgentEvent::PlanCacheHit { goal_hash, steps } => (
                "plan_cache_hit",
                json!({ "goal_hash": goal_hash, "steps": steps }),
            ),
            AgentEvent::FormatRecoveryUsed { format } => {
                ("format_recovery_used", json!({ "format": format }))
            }
            AgentEvent::CorrectionUsed { count, tool_name } => (
                "correction_used",
                json!({ "count": count, "tool": tool_name }),
            ),
            AgentEvent::PostconditionWarning { warning } => {
                ("postcondition_warning", json!({ "warning": warning }))
            }
            AgentEvent::SkillSuggested {
                skill_id,
                skill_name,
                tool_sequence,
                confidence,
            } => (
                "skill_suggested",
                json!({
                    "skill_id": skill_id,
                    "name": skill_name,
                    "tools": tool_sequence,
                    "confidence": confidence,
                }),
            ),
            AgentEvent::PermissionDenied {
                tool_name,
                role,
                reason,
            } => (
                "permission_denied",
                json!({ "tool": tool_name, "role": role, "reason": reason }),
            ),
            AgentEvent::ValidationFailed { input, reason } => (
                "validation_failed",
                json!({ "input": truncate(&input, 512), "reason": reason }),
            ),
            AgentEvent::BreakpointHit {
                step_index,
                tool_name,
            } => (
                "breakpoint_hit",
                json!({ "step_index": step_index, "tool": tool_name }),
            ),
            AgentEvent::PlanStarted { iteration } => {
                ("plan_started", json!({ "iteration": iteration }))
            }
            AgentEvent::PlanFinished { iteration } => {
                ("plan_finished", json!({ "iteration": iteration }))
            }
            AgentEvent::ToolDryRun {
                id,
                name,
                synthetic_output,
            } => (
                "tool_dry_run",
                json!({ "id": id, "name": name, "synthetic_output": synthetic_output }),
            ),
            AgentEvent::EvidenceReport {
                tool_name,
                system_state,
                taint,
                risk_level,
            } => (
                "evidence_report",
                json!({
                    "tool": tool_name,
                    "system_state": system_state,
                    "taint": taint,
                    "risk_level": risk_level,
                }),
            ),
        };
        let line = json!({ "event": name, "data": data });
        let _guard = self.writer.lock().ok();
        println!("{}", serde_json::to_string(&line).unwrap_or_default());
    }
}

/// A composite sink that forwards every event to two inner sinks. Used to
/// keep the CLI progress display on stderr while also emitting NDJSON for an
/// orchestrating parent process.
pub struct TeeSink<A: AgentEventSink, B: AgentEventSink> {
    pub primary: A,
    pub secondary: B,
}

impl<A: AgentEventSink, B: AgentEventSink> TeeSink<A, B> {
    pub fn new(primary: A, secondary: B) -> Self {
        Self { primary, secondary }
    }
}

impl<A: AgentEventSink, B: AgentEventSink> AgentEventSink for TeeSink<A, B> {
    fn emit(&self, event: AgentEvent) {
        self.secondary.emit(event.clone());
        self.primary.emit(event);
    }
}

/// Erased AgentEventSink wrapper so that the run handler can pick a base
/// sink at runtime (CLI progress alone vs. CLI progress + NDJSON) without
/// duplicating the entire chain construction code.
pub struct DynSink(pub Box<dyn AgentEventSink + Send + Sync>);

impl DynSink {
    pub fn new<S: AgentEventSink + Send + Sync + 'static>(inner: S) -> Self {
        Self(Box::new(inner))
    }
}

impl AgentEventSink for DynSink {
    fn emit(&self, event: AgentEvent) {
        self.0.emit(event);
    }
}

fn truncate(s: &str, max: usize) -> Value {
    if s.len() <= max {
        Value::String(s.to_owned())
    } else {
        let prefix = s.chars().take(max).collect::<String>();
        Value::String(format!("{prefix}…"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_under_limit_returns_unchanged() {
        assert_eq!(truncate("hi", 32), Value::String("hi".to_owned()));
    }

    #[test]
    fn truncate_over_limit_appends_ellipsis() {
        let v = truncate("aaaaaaaaaa", 4);
        assert_eq!(v, Value::String("aaaa…".to_owned()));
    }
}
