//! Persisted agent sessions: redacted transcript, tool outcomes, approvals and terminal outcome.
//!
//! One private JSON file per session under the data directory. Resuming never replays tool
//! effects: unresolved tool calls are dropped from the history and must be proposed (and
//! authorized) again.
use anyhow::{Result, anyhow, bail};
use helm_agent::AgentOutcome;
use helm_core::{ContentBlock, Message, Role, redact_secrets};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LogEntry {
    ToolCall {
        call_id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
    Approval {
        call_id: String,
        name: String,
        description: String,
        /// "approved", "denied" or "pending" (session ended before a decision).
        decision: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SessionRecord {
    pub version: u32,
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub goal: String,
    pub workspace: String,
    pub provider: String,
    pub model: String,
    pub outcome: AgentOutcome,
    pub summary: String,
    pub evidence: Vec<String>,
    pub steps: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub messages: Vec<Message>,
    pub log: Vec<LogEntry>,
    /// Totals from before the current run, so repeated `conclude` calls stay idempotent.
    #[serde(skip)]
    base: (u32, u64, u64, f64),
}

impl SessionRecord {
    /// Marks the start of a run (fresh or resumed from disk); later totals add to these.
    pub(crate) fn begin_run(&mut self) {
        self.base = (
            self.steps,
            self.input_tokens,
            self.output_tokens,
            self.cost_usd,
        );
    }

    pub(crate) fn set_run_totals(&mut self, steps: u32, input: u64, output: u64, cost: f64) {
        self.steps = self.base.0 + steps;
        self.input_tokens = self.base.1 + input;
        self.output_tokens = self.base.2 + output;
        self.cost_usd = self.base.3 + cost;
    }

    pub(crate) fn new(goal: &str, workspace: &str, provider: &str, model: &str) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut random = [0u8; 4];
        // ponytail: an unfillable RNG leaves a timestamp-only id; ids are not secrets.
        let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random);
        Self {
            version: VERSION,
            id: format!("sess-{now}-{}", hex::encode(random)),
            created_at: now,
            updated_at: now,
            goal: redact_secrets(goal),
            workspace: workspace.to_owned(),
            provider: provider.to_owned(),
            model: model.to_owned(),
            outcome: AgentOutcome::Running,
            summary: String::new(),
            evidence: Vec::new(),
            steps: 0,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            messages: Vec::new(),
            log: Vec::new(),
            base: (0, 0, 0, 0.0),
        }
    }

    /// History to continue from: the transcript with any assistant turn whose tool calls were
    /// never answered removed, so a resumed run cannot execute or fake those calls.
    pub(crate) fn resumable_history(&self) -> Vec<Message> {
        let mut history = self.messages.clone();
        if let Some(last) = history.last() {
            let unanswered = last.role == Role::Assistant
                && last
                    .content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::ToolUse { .. }));
            if unanswered {
                history.pop();
            }
        }
        history
    }
}

pub(crate) fn redact_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_secrets(text)),
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), redact_value(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub(crate) fn redact_message(message: &Message) -> Message {
    Message {
        role: message.role.clone(),
        content: message
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Text(text) => ContentBlock::Text(redact_secrets(text)),
                ContentBlock::ToolUse { id, name, input } => ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: redact_value(input),
                },
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => ContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: redact_secrets(content),
                    is_error: *is_error,
                },
            })
            .collect(),
    }
}

/// Sessions live beside the database, so `--db-path` (and every test) isolates them too.
pub(crate) fn sessions_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("agent-sessions")
}

fn session_path(dir: &Path, id: &str) -> Result<PathBuf> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        bail!("invalid session id");
    }
    Ok(dir.join(format!("{id}.json")))
}

pub(crate) fn save(db_path: &Path, record: &mut SessionRecord) -> Result<()> {
    record.updated_at = chrono::Utc::now().timestamp();
    record.messages = record.messages.iter().map(redact_message).collect();
    record.summary = redact_secrets(&record.summary);
    let dir = sessions_dir(db_path);
    helm_core::private_fs::create_private_dir(&dir)?;
    let bytes = serde_json::to_vec_pretty(record)?;
    crate::paths::write_atomic(&session_path(&dir, &record.id)?, &bytes, 0o600)?;
    Ok(())
}

pub(crate) fn load(db_path: &Path, id: &str) -> Result<SessionRecord> {
    let path = session_path(&sessions_dir(db_path), id)?;
    let text = std::fs::read_to_string(&path).map_err(|e| anyhow!("session {id}: {e}"))?;
    let record: SessionRecord =
        serde_json::from_str(&text).map_err(|e| anyhow!("session {id} is unreadable: {e}"))?;
    if record.version != VERSION {
        bail!("session {id} has unsupported version {}", record.version);
    }
    Ok(record)
}

pub(crate) fn list(db_path: &Path) -> Vec<SessionRecord> {
    let mut sessions: Vec<SessionRecord> = std::fs::read_dir(sessions_dir(db_path))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            load(db_path, name.strip_suffix(".json")?).ok()
        })
        .collect();
    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool_use(id: &str) -> Message {
        Message::assistant(vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "shell".into(),
            input: json!({"command": "echo hi"}),
        }])
    }

    #[test]
    fn resume_drops_unanswered_tool_calls_only() {
        let mut record = SessionRecord::new("goal", "/w", "p", "m");
        record.messages = vec![Message::user("goal"), tool_use("t1")];
        let history = record.resumable_history();
        assert_eq!(history.len(), 1);

        record
            .messages
            .push(Message::tool_results(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: "ok".into(),
                is_error: false,
            }]));
        assert_eq!(record.resumable_history().len(), 3);
    }

    #[test]
    fn redaction_covers_nested_tool_input_and_results() {
        let secret = "sk-proj-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let message = Message::assistant(vec![
            ContentBlock::Text(format!("key {secret}")),
            ContentBlock::ToolUse {
                id: "t".into(),
                name: "shell".into(),
                input: json!({"nested": [{"command": format!("curl -H 'Authorization: Bearer {secret}'")}]}),
            },
        ]);
        let text = serde_json::to_string(&redact_message(&message)).unwrap();
        assert!(!text.contains(secret), "{text}");
    }

    #[test]
    fn session_ids_cannot_escape_the_directory() {
        for id in ["", "../x", "a/b", "a.b", &"x".repeat(65)] {
            assert!(session_path(Path::new("/d"), id).is_err(), "{id}");
        }
        assert!(session_path(Path::new("/d"), "sess-1-ab").is_ok());
    }
}
