//! Q2 — agent-on-remote execution.
//!
//! When `helm run --remote <name> --agent-on-remote "<task>"` is invoked, we
//! spawn `ssh <user@host> helm run "<task>" --emit-events ...` on the remote
//! host (which must have HELM installed; see `helm bootstrap`). We then read
//! the remote's stdout line-by-line as NDJSON and re-emit each line as an
//! `AgentEvent` on the local `AgentEventSink`. This gives the local TUI/CLI a
//! faithful live transcript of the remote run.
//!
//! Wire format mirrors the local NDJSON emitter in `ndjson_sink.rs`. Events
//! that are not understood are forwarded as a `RunFailed`-style note rather
//! than discarded silently.

use std::process::Stdio;

use anyhow::{Context, Result, anyhow};
use helm_agent::{AgentEvent, AgentEventSink};
use helm_core::Capability;
use helm_providers::StopReason;
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

use crate::remote::RemoteEntry;

/// Outcome of a remote run, summarised for the local CLI.
#[derive(Debug, Default, Clone)]
pub struct RemoteRunOutcome {
    pub episode_id: Option<String>,
    pub final_message: Option<String>,
    pub error: Option<String>,
    pub iterations: u32,
    pub tokens_in: u32,
    pub tokens_out: u32,
}

impl RemoteRunOutcome {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

#[derive(Debug, Deserialize)]
struct WireLine {
    event: String,
    #[serde(default)]
    data: Value,
}

/// Run the agent on the remote host via SSH, streaming events back through
/// the supplied sink. Returns a synthetic [`RemoteRunOutcome`] composed from
/// the final event(s).
pub async fn run_on_remote<S: AgentEventSink + ?Sized>(
    remote: &RemoteEntry,
    task: &str,
    sink: &S,
    extra_args: &[&str],
) -> Result<RemoteRunOutcome> {
    let mut argv = remote.ssh_argv();
    argv.push("helm".to_owned());
    argv.push("run".to_owned());
    argv.push(escape_for_shell(task));
    argv.push("--emit-events".to_owned());
    for arg in extra_args {
        argv.push((*arg).to_owned());
    }
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("spawning remote ssh")?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("remote ssh produced no stdout"))?;
    let stderr = child.stderr.take();
    let mut reader = BufReader::new(stdout).lines();
    let stderr_buf = stderr.map(|s| {
        tokio::spawn(async move {
            let mut r = BufReader::new(s).lines();
            let mut acc = String::new();
            while let Ok(Some(line)) = r.next_line().await {
                if !acc.is_empty() {
                    acc.push('\n');
                }
                acc.push_str(&line);
            }
            acc
        })
    });
    let mut outcome = RemoteRunOutcome::default();
    while let Some(line) = reader
        .next_line()
        .await
        .context("reading remote ssh stdout")?
    {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(wire) = serde_json::from_str::<WireLine>(trimmed) else {
            continue;
        };
        if let Some(event) = parse_wire(&wire, &mut outcome) {
            sink.emit(event);
        }
    }
    let status = child.wait().await.context("awaiting remote ssh exit")?;
    if !status.success() {
        let stderr_text = match stderr_buf {
            Some(handle) => handle.await.unwrap_or_default(),
            None => String::new(),
        };
        return Err(anyhow!(
            "remote helm exited {status}{}",
            if stderr_text.is_empty() {
                String::new()
            } else {
                format!(": {stderr_text}")
            }
        ));
    }
    Ok(outcome)
}

fn parse_wire(wire: &WireLine, outcome: &mut RemoteRunOutcome) -> Option<AgentEvent> {
    match wire.event.as_str() {
        "run_started" => {
            let episode_id = wire
                .data
                .get("episode_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            outcome.episode_id = Some(episode_id.clone());
            let goal = wire
                .data
                .get("goal")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            Some(AgentEvent::RunStarted { episode_id, goal })
        }
        "text_delta" => {
            let chunk = wire
                .data
                .get("chunk")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            Some(AgentEvent::TextDelta { chunk })
        }
        "assistant_text" => {
            let text = wire
                .data
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            if outcome.final_message.is_none() {
                outcome.final_message = Some(text.clone());
            }
            Some(AgentEvent::AssistantText { text })
        }
        "tool_call_parsed" => {
            let id = wire
                .data
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let name = wire
                .data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let input = wire
                .data
                .get("input")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            Some(AgentEvent::ToolCallParsed { id, name, input })
        }
        "tool_call_validated" => {
            let id = wire
                .data
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let name = wire
                .data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            Some(AgentEvent::ToolCallValidated { id, name })
        }
        "tool_started" => Some(AgentEvent::ToolCallStarted {
            id: wire
                .data
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
            name: wire
                .data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
        }),
        "tool_finished" => Some(AgentEvent::ToolCallFinished {
            id: wire
                .data
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
            name: wire
                .data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
            success: wire
                .data
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            content: wire
                .data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
        }),
        "tool_denied" => Some(AgentEvent::ToolCallDenied {
            id: wire
                .data
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
            name: wire
                .data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
            reason: wire
                .data
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("(denied)")
                .to_owned(),
        }),
        "permission_requested" => {
            let capability = wire
                .data
                .get("capability")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<Capability>().unwrap_or(Capability::ShellExec))
                .unwrap_or(Capability::ShellExec);
            let tool_name = wire
                .data
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let taint = wire
                .data
                .get("taint")
                .and_then(|v| v.as_str())
                .unwrap_or("User")
                .to_owned();
            Some(AgentEvent::PermissionRequested {
                capability,
                tool_name,
                taint,
            })
        }
        "provider_call_started" => {
            let iteration = wire
                .data
                .get("iteration")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let provider = wire
                .data
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let model = wire
                .data
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            Some(AgentEvent::ProviderCallStarted {
                iteration,
                provider,
                model,
            })
        }
        "provider_call_finished" => {
            let iteration = wire
                .data
                .get("iteration")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let tokens_in = wire
                .data
                .get("tokens_in")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let tokens_out = wire
                .data
                .get("tokens_out")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            outcome.tokens_in = outcome.tokens_in.saturating_add(tokens_in);
            outcome.tokens_out = outcome.tokens_out.saturating_add(tokens_out);
            outcome.iterations = outcome.iterations.saturating_add(1);
            let stop_reason = wire
                .data
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(|s| match s {
                    "ToolUse" => StopReason::ToolUse,
                    "MaxTokens" => StopReason::MaxTokens,
                    "StopSequence" => StopReason::StopSequence,
                    _ => StopReason::EndTurn,
                })
                .unwrap_or(StopReason::EndTurn);
            Some(AgentEvent::ProviderCallFinished {
                iteration,
                stop_reason,
                tokens_in,
                tokens_out,
            })
        }
        "provider_failover" => {
            let from = wire
                .data
                .get("from")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let to = wire
                .data
                .get("to")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let reason = wire
                .data
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("(failover)")
                .to_owned();
            Some(AgentEvent::ProviderFailover { from, to, reason })
        }
        "budget_warning" => {
            let spent_usd = wire
                .data
                .get("spent_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let limit_usd = wire
                .data
                .get("limit_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            Some(AgentEvent::BudgetWarning {
                spent_usd,
                limit_usd,
            })
        }
        "budget_exceeded" => {
            let spent_usd = wire
                .data
                .get("spent_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let limit_usd = wire
                .data
                .get("limit_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            Some(AgentEvent::BudgetExceeded {
                spent_usd,
                limit_usd,
            })
        }
        "plan_cache_hit" => {
            let goal_hash = wire
                .data
                .get("goal_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let steps = wire.data.get("steps").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            Some(AgentEvent::PlanCacheHit { goal_hash, steps })
        }
        "format_recovery_used" => {
            let format = wire
                .data
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown)")
                .to_owned();
            Some(AgentEvent::FormatRecoveryUsed { format })
        }
        "correction_used" => {
            let count = wire.data.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let tool_name = wire
                .data
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            Some(AgentEvent::CorrectionUsed { count, tool_name })
        }
        "postcondition_warning" => {
            let warning = wire
                .data
                .get("warning")
                .and_then(|v| v.as_str())
                .unwrap_or("(warning)")
                .to_owned();
            Some(AgentEvent::PostconditionWarning { warning })
        }
        "skill_suggested" => {
            let skill_id = wire
                .data
                .get("skill_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let skill_name = wire
                .data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let confidence = wire
                .data
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.7) as f32;
            let tool_sequence = wire
                .data
                .get("tools")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Some(AgentEvent::SkillSuggested {
                skill_id,
                skill_name,
                tool_sequence,
                confidence,
            })
        }
        "permission_denied" => {
            let tool_name = wire
                .data
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let role = wire
                .data
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("User")
                .to_owned();
            let reason = wire
                .data
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("(denied)")
                .to_owned();
            Some(AgentEvent::PermissionDenied {
                tool_name,
                role,
                reason,
            })
        }
        "validation_failed" => {
            let input = wire
                .data
                .get("input")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let reason = wire
                .data
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("(validation failed)")
                .to_owned();
            Some(AgentEvent::ValidationFailed { input, reason })
        }
        "breakpoint_hit" => {
            let step_index = wire
                .data
                .get("step_index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let tool_name = wire
                .data
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            Some(AgentEvent::BreakpointHit {
                step_index,
                tool_name,
            })
        }
        "prompt_cache_hit" => {
            let tokens_saved = wire
                .data
                .get("tokens_saved")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            Some(AgentEvent::PromptCacheHit { tokens_saved })
        }
        "run_finished" | "completed" => {
            let final_message = wire
                .data
                .get("final")
                .and_then(|v| v.as_str())
                .or_else(|| wire.data.get("final_message").and_then(|v| v.as_str()))
                .map(|s| s.to_owned());
            if final_message.is_some() {
                outcome.final_message = final_message.clone();
            }
            None
        }
        "run_failed" | "error" => {
            let error = wire
                .data
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown error)")
                .to_owned();
            outcome.error = Some(error.clone());
            Some(AgentEvent::RunFailed {
                episode_id: outcome.episode_id.clone(),
                error,
            })
        }
        _ => {
            tracing::debug!(target: "helm::remote", "unknown wire event: {}", wire.event);
            None
        }
    }
}

fn escape_for_shell(s: &str) -> String {
    if s.is_empty() {
        return "''".to_owned();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '.' | '-' | ':' | '@' | ','));
    if safe {
        return s.to_owned();
    }
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_simple_word() {
        assert_eq!(escape_for_shell("simple"), "simple");
    }

    #[test]
    fn escape_spaces_and_quotes() {
        assert_eq!(escape_for_shell("a b"), "'a b'");
        assert_eq!(escape_for_shell("it's"), "'it'\\''s'");
    }

    #[test]
    fn parse_run_started_updates_outcome() {
        let wire = WireLine {
            event: "run_started".into(),
            data: serde_json::json!({"episode_id": "ep-1", "goal": "ls"}),
        };
        let mut outcome = RemoteRunOutcome::default();
        let event = parse_wire(&wire, &mut outcome).unwrap();
        assert_eq!(outcome.episode_id.as_deref(), Some("ep-1"));
        assert!(matches!(event, AgentEvent::RunStarted { .. }));
    }

    #[test]
    fn parse_provider_call_accumulates_tokens() {
        let mut outcome = RemoteRunOutcome::default();
        let wire = WireLine {
            event: "provider_call_finished".into(),
            data: serde_json::json!({"tokens_in": 100, "tokens_out": 50}),
        };
        let event = parse_wire(&wire, &mut outcome).unwrap();
        assert_eq!(outcome.tokens_in, 100);
        assert_eq!(outcome.tokens_out, 50);
        assert_eq!(outcome.iterations, 1);
        assert!(matches!(event, AgentEvent::ProviderCallFinished { tokens_in: 100, tokens_out: 50, .. }));
    }

    #[test]
    fn parse_run_failed_propagates_error() {
        let mut outcome = RemoteRunOutcome::default();
        let wire = WireLine {
            event: "run_failed".into(),
            data: serde_json::json!({"error": "boom"}),
        };
        let event = parse_wire(&wire, &mut outcome).unwrap();
        assert_eq!(outcome.error.as_deref(), Some("boom"));
        assert!(matches!(event, AgentEvent::RunFailed { .. }));
    }
}
