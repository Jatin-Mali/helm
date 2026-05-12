//! Lifecycle hooks (P1-P3): pre_run, post_run, on_tool_call.
//!
//! Hooks fire as shell commands with HELM_* env vars. Defined either inline
//! via `--pre-run`/`--post-run`/`--on-tool-call` flags or globally in
//! `~/.helm/hooks.toml`. Hooks are best-effort: failure logs a warning but
//! does NOT abort the run.

#![allow(dead_code)]

use crate::paths::config_dir;

use anyhow::{Context, Result};
use helm_agent::{AgentEvent, AgentEventSink};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;

#[derive(Debug, Default, Clone, Deserialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub pre_run: Option<String>,
    #[serde(default)]
    pub post_run: Option<String>,
    #[serde(default)]
    pub on_tool_call: Option<String>,
    /// Fired when an episode starts (RunStarted event).
    #[serde(default)]
    pub on_episode_start: Option<String>,
    /// Fired when an episode ends (RunFinished or RunFailed).
    #[serde(default)]
    pub on_episode_end: Option<String>,
    /// Fired before each LLM planning call.
    #[serde(default)]
    pub pre_plan: Option<String>,
    /// Fired after each LLM planning call completes.
    #[serde(default)]
    pub post_plan: Option<String>,
    /// Optional matchers: tool name -> command (overrides on_tool_call for that tool).
    #[serde(default)]
    pub tool: HashMap<String, ToolHook>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolHook {
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir().join("hooks.toml"))
}

pub fn load_global() -> HooksConfig {
    let Ok(path) = config_path() else {
        return HooksConfig::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return HooksConfig::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

/// Merge inline CLI flags over the global hooks config.
pub fn merge_inline(
    base: HooksConfig,
    pre_run: Option<String>,
    post_run: Option<String>,
    on_tool_call: Option<String>,
) -> HooksConfig {
    HooksConfig {
        pre_run: pre_run.or(base.pre_run),
        post_run: post_run.or(base.post_run),
        on_tool_call: on_tool_call.or(base.on_tool_call),
        on_episode_start: base.on_episode_start,
        on_episode_end: base.on_episode_end,
        pre_plan: base.pre_plan,
        post_plan: base.post_plan,
        tool: base.tool,
    }
}

/// Run a shell hook with a base set of env vars. Best-effort, never aborts.
pub async fn fire(stage: &str, cmd: &str, extra: HashMap<String, String>) {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(cmd)
        .env("HELM_HOOK", stage)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Ok(cwd) = env::current_dir() {
        command.env("HELM_CWD", cwd);
    }
    for (k, v) in extra {
        command.env(k, v);
    }
    match command.output().await {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(
                    target: "helm::hook",
                    "hook `{stage}` exited {:?}: {}",
                    output.status.code(),
                    stderr.trim()
                );
            } else if !output.stdout.is_empty() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                tracing::debug!(target: "helm::hook", "hook `{stage}` stdout: {}", stdout.trim());
            }
        }
        Err(error) => {
            tracing::warn!(target: "helm::hook", "hook `{stage}` spawn failed: {error}");
        }
    }
}

/// Synchronous helper for sinks (uses blocking spawn).
pub fn fire_sync(stage: String, cmd: String, extra: HashMap<String, String>) {
    tokio::spawn(async move {
        fire(&stage, &cmd, extra).await;
    });
}

/// AgentEventSink wrapper that fires lifecycle hooks on agent events and
/// forwards every event to the inner sink unchanged.
pub struct HookEventSink<S: AgentEventSink> {
    inner: S,
    on_tool_call: Option<String>,
    on_episode_start: Option<String>,
    on_episode_end: Option<String>,
    pre_plan: Option<String>,
    post_plan: Option<String>,
    per_tool: Arc<HashMap<String, ToolHook>>,
    episode_id: Arc<std::sync::Mutex<Option<String>>>,
    target: Option<String>,
}

impl<S: AgentEventSink> HookEventSink<S> {
    pub fn new(inner: S, hooks: &HooksConfig, target: Option<String>) -> Self {
        Self {
            inner,
            on_tool_call: hooks.on_tool_call.clone(),
            on_episode_start: hooks.on_episode_start.clone(),
            on_episode_end: hooks.on_episode_end.clone(),
            pre_plan: hooks.pre_plan.clone(),
            post_plan: hooks.post_plan.clone(),
            per_tool: Arc::new(hooks.tool.clone()),
            episode_id: Arc::new(std::sync::Mutex::new(None)),
            target,
        }
    }

    fn base_env(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Some(eid) = self.episode_id.lock().ok().and_then(|g| g.clone()) {
            map.insert("HELM_EPISODE_ID".to_owned(), eid);
        }
        if let Some(t) = &self.target {
            map.insert("HELM_TARGET".to_owned(), t.clone());
        }
        map
    }
}

impl<S: AgentEventSink> AgentEventSink for HookEventSink<S> {
    fn emit(&self, event: AgentEvent) {
        match &event {
            AgentEvent::RunStarted { episode_id, goal } => {
                if let Ok(mut guard) = self.episode_id.lock() {
                    *guard = Some(episode_id.clone());
                }
                if let Some(cmd) = self.on_episode_start.as_ref() {
                    let mut extra = self.base_env();
                    extra.insert("HELM_GOAL".to_owned(), goal.clone());
                    fire_sync("on_episode_start".to_owned(), cmd.clone(), extra);
                }
            }
            AgentEvent::RunFinished { result } => {
                if let Some(cmd) = self.on_episode_end.as_ref() {
                    let mut extra = self.base_env();
                    extra.insert("HELM_OUTCOME".to_owned(), "success".to_owned());
                    extra.insert("HELM_EPISODE_ID".to_owned(), result.episode_id.clone());
                    fire_sync("on_episode_end".to_owned(), cmd.clone(), extra);
                }
            }
            AgentEvent::RunFailed { episode_id, error } => {
                if let Some(cmd) = self.on_episode_end.as_ref() {
                    let mut extra = self.base_env();
                    extra.insert("HELM_OUTCOME".to_owned(), "failed".to_owned());
                    extra.insert("HELM_ERROR".to_owned(), error.clone());
                    if let Some(eid) = episode_id {
                        extra.insert("HELM_EPISODE_ID".to_owned(), eid.clone());
                    }
                    fire_sync("on_episode_end".to_owned(), cmd.clone(), extra);
                }
            }
            AgentEvent::PlanStarted { iteration } => {
                if let Some(cmd) = self.pre_plan.as_ref() {
                    let mut extra = self.base_env();
                    extra.insert("HELM_ITERATION".to_owned(), iteration.to_string());
                    fire_sync("pre_plan".to_owned(), cmd.clone(), extra);
                }
            }
            AgentEvent::PlanFinished { iteration } => {
                if let Some(cmd) = self.post_plan.as_ref() {
                    let mut extra = self.base_env();
                    extra.insert("HELM_ITERATION".to_owned(), iteration.to_string());
                    fire_sync("post_plan".to_owned(), cmd.clone(), extra);
                }
            }
            AgentEvent::ToolCallParsed { name, input, .. } => {
                let input_str = serde_json::to_string(input).unwrap_or_default();
                let mut extra = self.base_env();
                extra.insert("HELM_TOOL".to_owned(), name.clone());
                extra.insert("HELM_TOOL_NAME".to_owned(), name.clone());
                extra.insert("HELM_INPUT".to_owned(), input_str.clone());
                extra.insert("HELM_TOOL_INPUT".to_owned(), input_str);
                if let Some(per) = self.per_tool.get(name)
                    && let Some(cmd) = per.before.as_ref()
                {
                    fire_sync("pre_tool".to_owned(), cmd.clone(), extra.clone());
                }
                if let Some(cmd) = self.on_tool_call.as_ref() {
                    fire_sync("on_tool_call".to_owned(), cmd.clone(), extra);
                }
            }
            AgentEvent::ToolCallFinished {
                name,
                success,
                content,
                ..
            } => {
                if let Some(per) = self.per_tool.get(name)
                    && let Some(cmd) = per.after.as_ref()
                {
                    let mut extra = self.base_env();
                    extra.insert("HELM_TOOL".to_owned(), name.clone());
                    extra.insert("HELM_SUCCESS".to_owned(), success.to_string());
                    extra.insert(
                        "HELM_OUTPUT".to_owned(),
                        content.chars().take(2000).collect(),
                    );
                    fire_sync("post_tool".to_owned(), cmd.clone(), extra);
                }
            }
            _ => {}
        }
        self.inner.emit(event);
    }
}

/// Convenience helpers used in the run handlers.
pub async fn fire_pre_run(hooks: &HooksConfig, task: &str, target: Option<&str>) -> Result<()> {
    if let Some(cmd) = hooks.pre_run.as_deref() {
        let mut extra: HashMap<String, String> = HashMap::new();
        extra.insert("HELM_TASK".to_owned(), task.to_owned());
        if let Some(t) = target {
            extra.insert("HELM_TARGET".to_owned(), t.to_owned());
        }
        fire("pre_run", cmd, extra).await;
    }
    Ok(())
}

pub async fn fire_post_run(
    hooks: &HooksConfig,
    episode_id: &str,
    success: bool,
    target: Option<&str>,
) -> Result<()> {
    if let Some(cmd) = hooks.post_run.as_deref() {
        let mut extra: HashMap<String, String> = HashMap::new();
        extra.insert("HELM_EPISODE_ID".to_owned(), episode_id.to_owned());
        extra.insert("HELM_SUCCESS".to_owned(), success.to_string());
        if let Some(t) = target {
            extra.insert("HELM_TARGET".to_owned(), t.to_owned());
        }
        fire("post_run", cmd, extra).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct CaptureSink(Mutex<Vec<String>>);
    impl AgentEventSink for CaptureSink {
        fn emit(&self, event: AgentEvent) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{event:?}").chars().take(40).collect());
        }
    }

    #[test]
    fn merge_inline_overrides_base() {
        let base = HooksConfig {
            pre_run: Some("base_pre".into()),
            post_run: Some("base_post".into()),
            on_tool_call: None,
            tool: HashMap::new(),
            ..HooksConfig::default()
        };
        let merged = merge_inline(base, Some("cli_pre".into()), None, Some("cli_tool".into()));
        assert_eq!(merged.pre_run.as_deref(), Some("cli_pre"));
        assert_eq!(merged.post_run.as_deref(), Some("base_post"));
        assert_eq!(merged.on_tool_call.as_deref(), Some("cli_tool"));
    }

    #[test]
    fn hook_event_sink_forwards_events() {
        let sink = HookEventSink::new(
            CaptureSink(Mutex::new(Vec::new())),
            &HooksConfig::default(),
            None,
        );
        sink.emit(AgentEvent::RunStarted {
            episode_id: "ep-1".into(),
            goal: "test".into(),
        });
        let captured = sink.inner.0.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert!(captured[0].contains("RunStarted"));
    }
}

/// Convenience: read hooks.toml + inline overrides.
pub fn load(
    pre_run: Option<String>,
    post_run: Option<String>,
    on_tool_call: Option<String>,
) -> HooksConfig {
    merge_inline(load_global(), pre_run, post_run, on_tool_call)
}

#[allow(clippy::items_after_test_module)]
/// Helper used by tests in main.rs.
#[allow(dead_code)]
pub fn from_str(text: &str) -> Result<HooksConfig> {
    toml::from_str(text).context("parsing hooks.toml")
}
