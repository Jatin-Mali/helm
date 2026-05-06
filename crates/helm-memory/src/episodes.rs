//! Episode log API backed by SQLite.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::Utc;
use helm_core::{Capability, ContentBlock, GrantScope, MemoryError, Taint};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

const MIGRATION_0001: &str = include_str!("../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../migrations/0002_model_capability_warning.sql");
const MIGRATION_0003: &str = include_str!("../migrations/0003_v3_corrections.sql");
const MIGRATION_0004: &str = include_str!("../migrations/0004_security.sql");

/// UUID string identifying an episode row.
pub type EpisodeId = String;

/// Final outcome stored for a completed episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeOutcome {
    /// The task completed successfully.
    Success,
    /// The task stopped with a useful partial result.
    Partial,
    /// The task failed and the error was recorded.
    Failure,
}

impl EpisodeOutcome {
    /// Returns the SQLite representation required by the schema.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failure => "failure",
        }
    }
}

/// Role attached to a persisted episode step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepRole {
    /// System prompt step.
    System,
    /// User message step.
    User,
    /// Assistant message step.
    Assistant,
    /// Tool result step.
    Tool,
}

impl StepRole {
    /// Returns the SQLite representation required by the schema.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

/// Read model for a persisted episode row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeRecord {
    /// Episode UUID.
    pub id: EpisodeId,
    /// User goal that started the episode.
    pub goal: String,
    /// Start timestamp in Unix milliseconds.
    pub started_at: i64,
    /// End timestamp in Unix milliseconds, if finished.
    pub ended_at: Option<i64>,
    /// Stored outcome string, if finished.
    pub outcome: Option<String>,
    /// Number of logged steps or iterations recorded for the episode.
    pub iterations: u32,
    /// Total provider input tokens recorded for the episode.
    pub tokens_in: u32,
    /// Total provider output tokens recorded for the episode.
    pub tokens_out: u32,
    /// Final assistant text, if available.
    pub final_message: Option<String>,
    /// Error text recorded for failures.
    pub error: Option<String>,
    /// Warning text explaining suspected model/tool-calling capability problems.
    pub model_capability_warning: Option<String>,
    /// Number of corrective ToolResult messages sent during this episode.
    pub corrections_used: u32,
    /// Whether the parser recovered a text-format tool call during this episode.
    pub format_recovery_used: bool,
    /// JSON-serialised list of response format names observed, one per iteration.
    pub response_format_log: Option<String>,
    /// Number of turns collapsed by the rolling context trimmer.
    pub total_turns_summarized: u32,
}

/// Aggregate episode outcome counts for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpisodeOutcomeCounts {
    /// Total number of episodes.
    pub total: u32,
    /// Episodes with `success` outcome.
    pub success: u32,
    /// Episodes with `partial` outcome.
    pub partial: u32,
    /// Episodes with `failure` outcome.
    pub failure: u32,
}

/// Read model for a persisted episode step.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRecord {
    /// Episode UUID that owns this step.
    pub episode_id: EpisodeId,
    /// Step index assigned by the ReAct loop.
    pub step_index: u32,
    /// Stored role string for the step.
    pub role: String,
    /// Deserialized content blocks for this step.
    pub content: Vec<ContentBlock>,
    /// Provider input tokens recorded for this step.
    pub tokens_in: u32,
    /// Provider output tokens recorded for this step.
    pub tokens_out: u32,
    /// Creation timestamp in Unix milliseconds.
    pub created_at: i64,
}

/// Read model for a persisted capability grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGrantRecord {
    /// Grant UUID.
    pub id: String,
    /// Capability covered by this grant.
    pub capability: Capability,
    /// Scope/lifetime assigned to the grant.
    pub scope: GrantScope,
    /// Grant creation timestamp in Unix milliseconds.
    pub granted_at: i64,
    /// Expiry timestamp in Unix milliseconds, if time-limited.
    pub expires_at: Option<i64>,
    /// Revocation timestamp in Unix milliseconds, if revoked or consumed.
    pub revoked_at: Option<i64>,
}

/// Input data for a hash-chained audit event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEventInput {
    /// Episode that caused this event, if any.
    pub episode_id: Option<String>,
    /// Tool name that was requested.
    pub tool_name: String,
    /// Hash of the tool input JSON.
    pub input_hash: String,
    /// Hash of the tool output or denial text.
    pub output_hash: String,
    /// Capability checked for this event.
    pub capability: Capability,
    /// Taint assigned to the context that requested the tool call.
    pub taint: Taint,
    /// Current working directory used for execution.
    pub cwd: String,
    /// Decision stored for audit, typically `allow` or `deny`.
    pub decision: String,
}

/// Read model for a hash-chained audit event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEventRecord {
    /// Monotonic SQLite row id.
    pub id: i64,
    /// Episode that caused this event, if any.
    pub episode_id: Option<String>,
    /// Event timestamp in Unix milliseconds.
    pub timestamp: i64,
    /// Tool name that was requested.
    pub tool_name: String,
    /// Hash of the tool input JSON.
    pub input_hash: String,
    /// Hash of the tool output or denial text.
    pub output_hash: String,
    /// Capability checked for this event.
    pub capability: Capability,
    /// Taint label stored at execution time.
    pub taint: String,
    /// Current working directory used for execution.
    pub cwd: String,
    /// Decision stored for audit, typically `allow` or `deny`.
    pub decision: String,
    /// Previous event hash.
    pub previous_hash: String,
    /// Event hash for this row.
    pub event_hash: String,
}

/// Result returned by audit hash-chain verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditVerification {
    /// Whether every row matched the expected hash chain.
    pub ok: bool,
    /// Number of audit events checked.
    pub checked: u32,
    /// First bad event id, if verification failed.
    pub failed_at: Option<i64>,
    /// Human-readable failure reason.
    pub reason: Option<String>,
}

/// SQLite-backed episode store with blocking database work isolated from async tasks.
#[derive(Clone)]
pub struct MemoryStore {
    conn: Arc<Mutex<Connection>>,
}

impl MemoryStore {
    /// Opens or creates a SQLite database and runs HELM migrations idempotently.
    pub async fn open(path: &Path) -> Result<Self, MemoryError> {
        let path = path.to_path_buf();
        let conn = tokio::task::spawn_blocking(move || {
            let conn = Connection::open(path).map_err(sqlite_error)?;
            run_migrations(&conn)?;
            Ok::<Connection, MemoryError>(conn)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Starts a new episode for `goal` and returns its UUID.
    pub async fn start_episode(&self, goal: &str) -> Result<EpisodeId, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let goal = goal.to_owned();
        tokio::task::spawn_blocking(move || {
            let id = Uuid::new_v4().to_string();
            let started_at = now_ms();
            let guard = lock_conn(&conn)?;
            guard
                .execute(
                    "INSERT INTO episodes (id, goal, started_at) VALUES (?1, ?2, ?3)",
                    params![id, goal, started_at],
                )
                .map_err(sqlite_error)?;
            Ok::<EpisodeId, MemoryError>(id)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Logs one episode step with serialized content blocks and token counts.
    pub async fn log_step(
        &self,
        episode_id: &str,
        step_index: u32,
        role: StepRole,
        content: &[ContentBlock],
        tokens_in: u32,
        tokens_out: u32,
    ) -> Result<(), MemoryError> {
        let conn = Arc::clone(&self.conn);
        let episode_id = episode_id.to_owned();
        let content_json = serde_json::to_string(content)?;
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            guard
                .execute(
                    "INSERT INTO episode_steps \
                     (episode_id, step_index, role, content_json, tokens_in, tokens_out, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        episode_id,
                        i64::from(step_index),
                        role.as_str(),
                        content_json,
                        i64::from(tokens_in),
                        i64::from(tokens_out),
                        now_ms()
                    ],
                )
                .map_err(sqlite_error)?;
            guard
                .execute(
                    "UPDATE episodes \
                     SET tokens_in = tokens_in + ?2, \
                         tokens_out = tokens_out + ?3 \
                     WHERE id = ?1",
                    params![episode_id, i64::from(tokens_in), i64::from(tokens_out)],
                )
                .map_err(sqlite_error)?;
            Ok::<(), MemoryError>(())
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Marks an episode as finished and records final text or error details.
    pub async fn finish_episode(
        &self,
        episode_id: &str,
        outcome: EpisodeOutcome,
        final_message: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), MemoryError> {
        let conn = Arc::clone(&self.conn);
        let episode_id = episode_id.to_owned();
        let final_message = final_message.map(str::to_owned);
        let error = error.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            guard
                .execute(
                    "UPDATE episodes \
                     SET ended_at = ?2, \
                         outcome = ?3, \
                         iterations = (SELECT COUNT(*) FROM episode_steps WHERE episode_id = ?1 AND role = 'assistant'), \
                         final_message = ?4, \
                         error = ?5 \
                     WHERE id = ?1",
                    params![episode_id, now_ms(), outcome.as_str(), final_message, error],
                )
                .map_err(sqlite_error)?;
            Ok::<(), MemoryError>(())
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Records a warning about model capability or provider behavior for an episode.
    pub async fn set_model_capability_warning(
        &self,
        episode_id: &str,
        warning: &str,
    ) -> Result<(), MemoryError> {
        let conn = Arc::clone(&self.conn);
        let episode_id = episode_id.to_owned();
        let warning = warning.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            guard
                .execute(
                    "UPDATE episodes SET model_capability_warning = ?2 WHERE id = ?1",
                    params![episode_id, warning],
                )
                .map_err(sqlite_error)?;
            Ok::<(), MemoryError>(())
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Records corrective-retry and format-recovery statistics for an episode.
    pub async fn record_corrections(
        &self,
        episode_id: &str,
        corrections_used: u32,
        format_recovery_used: bool,
        response_format_log: Option<&str>,
        total_turns_summarized: u32,
    ) -> Result<(), MemoryError> {
        let conn = Arc::clone(&self.conn);
        let episode_id = episode_id.to_owned();
        let format_log = response_format_log.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            guard
                .execute(
                    "UPDATE episodes \
                     SET corrections_used = ?2, \
                         format_recovery_used = ?3, \
                         response_format_log = ?4, \
                         total_turns_summarized = ?5 \
                     WHERE id = ?1",
                    params![
                        episode_id,
                        i64::from(corrections_used),
                        i64::from(format_recovery_used),
                        format_log,
                        i64::from(total_turns_summarized)
                    ],
                )
                .map_err(sqlite_error)?;
            Ok::<(), MemoryError>(())
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns the most recently started episodes up to `limit`.
    pub async fn recent_episodes(&self, limit: u32) -> Result<Vec<EpisodeRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let mut stmt = guard
                .prepare(
                    "SELECT id, goal, started_at, ended_at, outcome, iterations, tokens_in, \
                     tokens_out, final_message, error, model_capability_warning, \
                     corrections_used, format_recovery_used, response_format_log, \
                     total_turns_summarized \
                     FROM episodes ORDER BY started_at DESC LIMIT ?1",
                )
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map(params![i64::from(limit)], row_to_episode)
                .map_err(sqlite_error)?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(sqlite_error)?);
            }
            Ok::<Vec<EpisodeRecord>, MemoryError>(records)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns the number of episodes in the database.
    pub async fn episode_count(&self) -> Result<u32, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let count: i64 = guard
                .query_row("SELECT COUNT(*) FROM episodes", [], |row| row.get(0))
                .map_err(sqlite_error)?;
            u32::try_from(count).map_err(|error| MemoryError::Migration(error.to_string()))
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns aggregate counts grouped by final episode outcome.
    pub async fn episode_outcome_counts(&self) -> Result<EpisodeOutcomeCounts, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let total: i64 = guard
                .query_row("SELECT COUNT(*) FROM episodes", [], |row| row.get(0))
                .map_err(sqlite_error)?;
            let success = count_outcome(&guard, EpisodeOutcome::Success.as_str())?;
            let partial = count_outcome(&guard, EpisodeOutcome::Partial.as_str())?;
            let failure = count_outcome(&guard, EpisodeOutcome::Failure.as_str())?;
            Ok::<EpisodeOutcomeCounts, MemoryError>(EpisodeOutcomeCounts {
                total: i64_to_u32(total).map_err(|error| MemoryError::Sqlite(error.to_string()))?,
                success,
                partial,
                failure,
            })
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns the number of steps stored for an episode.
    pub async fn step_count(&self, episode_id: &str) -> Result<u32, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let episode_id = episode_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let count: i64 = guard
                .query_row(
                    "SELECT COUNT(*) FROM episode_steps WHERE episode_id = ?1",
                    params![episode_id],
                    |row| row.get(0),
                )
                .map_err(sqlite_error)?;
            u32::try_from(count).map_err(|error| MemoryError::Migration(error.to_string()))
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns one episode by ID, or `None` if it does not exist.
    pub async fn episode_by_id(
        &self,
        episode_id: &str,
    ) -> Result<Option<EpisodeRecord>, MemoryError> {
        self.get_episode(episode_id).await
    }

    /// Returns one episode by ID, or `None` if it does not exist.
    pub async fn get_episode(
        &self,
        episode_id: &str,
    ) -> Result<Option<EpisodeRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let episode_id = episode_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            guard
                .query_row(
                    "SELECT id, goal, started_at, ended_at, outcome, iterations, tokens_in, \
                     tokens_out, final_message, error, model_capability_warning, \
                     corrections_used, format_recovery_used, response_format_log, \
                     total_turns_summarized \
                     FROM episodes WHERE id = ?1",
                    params![episode_id],
                    row_to_episode,
                )
                .optional()
                .map_err(sqlite_error)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns all stored steps for an episode ordered by step index.
    pub async fn get_steps(&self, episode_id: &str) -> Result<Vec<StepRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let episode_id = episode_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let mut stmt = guard
                .prepare(
                    "SELECT episode_id, step_index, role, content_json, tokens_in, tokens_out, \
                     created_at FROM episode_steps WHERE episode_id = ?1 ORDER BY step_index ASC",
                )
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map(params![episode_id], row_to_step)
                .map_err(sqlite_error)?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(sqlite_error)?);
            }
            Ok::<Vec<StepRecord>, MemoryError>(records)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns the SQLite journal mode currently active for this connection.
    pub async fn journal_mode(&self) -> Result<String, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            guard
                .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
                .map_err(sqlite_error)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns the current SQLite `user_version` schema number.
    pub async fn schema_version(&self) -> Result<u32, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let version: i64 = guard
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .map_err(sqlite_error)?;
            i64_to_u32(version).map_err(|error| MemoryError::Sqlite(error.to_string()))
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Grants a capability with the requested scope and returns the stored row.
    pub async fn grant_capability(
        &self,
        capability: Capability,
        scope: GrantScope,
    ) -> Result<CapabilityGrantRecord, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let now = now_ms();
            let id = Uuid::new_v4().to_string();
            let expires_at = match scope {
                GrantScope::FifteenMinutes => Some(now.saturating_add(15 * 60 * 1000)),
                GrantScope::Once | GrantScope::Session | GrantScope::Always => None,
            };
            let guard = lock_conn(&conn)?;
            guard
                .execute(
                    "INSERT INTO capability_grants \
                     (id, capability, scope, granted_at, expires_at, revoked_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                    params![id, capability.as_str(), scope.as_str(), now, expires_at],
                )
                .map_err(sqlite_error)?;
            Ok::<CapabilityGrantRecord, MemoryError>(CapabilityGrantRecord {
                id,
                capability,
                scope,
                granted_at: now,
                expires_at,
                revoked_at: None,
            })
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Lists all capability grants ordered by newest first.
    pub async fn list_capability_grants(&self) -> Result<Vec<CapabilityGrantRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let mut stmt = guard
                .prepare(
                    "SELECT id, capability, scope, granted_at, expires_at, revoked_at \
                     FROM capability_grants ORDER BY granted_at DESC",
                )
                .map_err(sqlite_error)?;
            let rows = stmt.query_map([], row_to_grant).map_err(sqlite_error)?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(sqlite_error)?);
            }
            Ok::<Vec<CapabilityGrantRecord>, MemoryError>(records)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Revokes all active grants for a capability and returns the number changed.
    pub async fn revoke_capability(&self, capability: Capability) -> Result<u32, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let changed = guard
                .execute(
                    "UPDATE capability_grants SET revoked_at = ?2 \
                     WHERE capability = ?1 AND revoked_at IS NULL",
                    params![capability.as_str(), now_ms()],
                )
                .map_err(sqlite_error)?;
            u32::try_from(changed).map_err(|error| MemoryError::Migration(error.to_string()))
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns an active grant for `capability`, optionally requiring fresh confirmation.
    pub async fn active_capability_grant(
        &self,
        capability: Capability,
        require_fresh: bool,
    ) -> Result<Option<CapabilityGrantRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            active_grant_locked(&guard, capability, require_fresh)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Consumes a one-shot grant by revoking it after use.
    pub async fn consume_grant_if_once(&self, grant_id: &str) -> Result<(), MemoryError> {
        let conn = Arc::clone(&self.conn);
        let grant_id = grant_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            guard
                .execute(
                    "UPDATE capability_grants SET revoked_at = ?2 \
                     WHERE id = ?1 AND scope = 'once' AND revoked_at IS NULL",
                    params![grant_id, now_ms()],
                )
                .map_err(sqlite_error)?;
            Ok::<(), MemoryError>(())
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Appends one hash-chained audit event and returns its event hash.
    pub async fn append_audit_event(&self, input: AuditEventInput) -> Result<String, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let previous_hash = latest_audit_hash(&guard)?;
            let timestamp = now_ms();
            let taint = input.taint.label();
            let event_hash = audit_hash(AuditHashParts {
                previous_hash: &previous_hash,
                episode_id: input.episode_id.as_deref(),
                timestamp,
                tool_name: &input.tool_name,
                input_hash: &input.input_hash,
                output_hash: &input.output_hash,
                capability: input.capability.as_str(),
                taint: &taint,
                cwd: &input.cwd,
                decision: &input.decision,
            });
            guard
                .execute(
                    "INSERT INTO audit_events \
                     (episode_id, timestamp, tool_name, input_hash, output_hash, capability, \
                      taint, cwd, decision, previous_hash, event_hash) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        input.episode_id,
                        timestamp,
                        input.tool_name,
                        input.input_hash,
                        input.output_hash,
                        input.capability.as_str(),
                        taint,
                        input.cwd,
                        input.decision,
                        previous_hash,
                        event_hash
                    ],
                )
                .map_err(sqlite_error)?;
            Ok::<String, MemoryError>(event_hash)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns audit events, optionally filtered by episode id.
    pub async fn audit_events(
        &self,
        episode_id: Option<&str>,
    ) -> Result<Vec<AuditEventRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let episode_id = episode_id.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let sql = match episode_id {
                Some(_) => {
                    "SELECT id, episode_id, timestamp, tool_name, input_hash, output_hash, \
                     capability, taint, cwd, decision, previous_hash, event_hash \
                     FROM audit_events WHERE episode_id = ?1 ORDER BY id ASC"
                }
                None => {
                    "SELECT id, episode_id, timestamp, tool_name, input_hash, output_hash, \
                     capability, taint, cwd, decision, previous_hash, event_hash \
                     FROM audit_events ORDER BY id ASC"
                }
            };
            let mut stmt = guard.prepare(sql).map_err(sqlite_error)?;
            let mut records = Vec::new();
            if let Some(id) = episode_id {
                let rows = stmt
                    .query_map(params![id], row_to_audit)
                    .map_err(sqlite_error)?;
                for row in rows {
                    records.push(row.map_err(sqlite_error)?);
                }
            } else {
                let rows = stmt.query_map([], row_to_audit).map_err(sqlite_error)?;
                for row in rows {
                    records.push(row.map_err(sqlite_error)?);
                }
            }
            Ok::<Vec<AuditEventRecord>, MemoryError>(records)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Verifies every audit event hash against the previous row.
    pub async fn verify_audit_chain(&self) -> Result<AuditVerification, MemoryError> {
        let events = self.audit_events(None).await?;
        let mut previous = "GENESIS".to_owned();
        let mut checked = 0_u32;
        for event in events {
            checked = checked.saturating_add(1);
            if event.previous_hash != previous {
                return Ok(AuditVerification {
                    ok: false,
                    checked,
                    failed_at: Some(event.id),
                    reason: Some("previous hash mismatch".to_owned()),
                });
            }
            let expected = audit_hash(AuditHashParts {
                previous_hash: &event.previous_hash,
                episode_id: event.episode_id.as_deref(),
                timestamp: event.timestamp,
                tool_name: &event.tool_name,
                input_hash: &event.input_hash,
                output_hash: &event.output_hash,
                capability: event.capability.as_str(),
                taint: &event.taint,
                cwd: &event.cwd,
                decision: &event.decision,
            });
            if expected != event.event_hash {
                return Ok(AuditVerification {
                    ok: false,
                    checked,
                    failed_at: Some(event.id),
                    reason: Some("event hash mismatch".to_owned()),
                });
            }
            previous = event.event_hash;
        }
        Ok(AuditVerification {
            ok: true,
            checked,
            failed_at: None,
            reason: None,
        })
    }
}

fn count_outcome(conn: &Connection, outcome: &str) -> Result<u32, MemoryError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM episodes WHERE outcome = ?1",
            params![outcome],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    i64_to_u32(count).map_err(|error| MemoryError::Sqlite(error.to_string()))
}

fn run_migrations(conn: &Connection) -> Result<(), MemoryError> {
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sqlite_error)?;
    match user_version {
        0 => {
            conn.execute_batch(MIGRATION_0001).map_err(sqlite_error)?;
            apply_0002(conn)?;
            apply_0003(conn)
        }
        1 => {
            apply_0002(conn)?;
            apply_0003(conn)
        }
        2 => apply_0003(conn),
        3 => apply_0004(conn),
        4 => apply_0005(conn),
        5 => apply_0006(conn),
        6 => apply_0007(conn),
        7 => conn
            .execute_batch("PRAGMA foreign_keys = ON")
            .map_err(sqlite_error),
        other => Err(MemoryError::Migration(format!(
            "unsupported schema version: {other}"
        ))),
    }
}

fn apply_0002(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    if !column_exists(conn, "episodes", "model_capability_warning")? {
        conn.execute_batch(MIGRATION_0002).map_err(sqlite_error)?;
    }
    conn.execute_batch("PRAGMA user_version = 2")
        .map_err(sqlite_error)?;
    Ok(())
}

fn apply_0003(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    if !column_exists(conn, "episodes", "corrections_used")? {
        conn.execute_batch(MIGRATION_0003).map_err(sqlite_error)?;
    }
    conn.execute_batch("PRAGMA user_version = 3")
        .map_err(sqlite_error)?;
    apply_0004(conn)?;
    Ok(())
}

fn apply_0004(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    if !table_exists(conn, "capability_grants")? {
        conn.execute_batch(MIGRATION_0004).map_err(sqlite_error)?;
    }
    conn.execute_batch("PRAGMA user_version = 4")
        .map_err(sqlite_error)?;
    apply_0005(conn)?;
    Ok(())
}

fn apply_0005(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    // Normalize early experimental shell capability grants to the legacy
    // broad shell.run name; apply_0007 splits that to shell.shell.
    conn.execute_batch(
        "UPDATE capability_grants SET capability = 'shell.run' \
         WHERE capability IN ('shell.exec', 'shell.shell')",
    )
    .map_err(sqlite_error)?;
    conn.execute_batch("PRAGMA user_version = 5")
        .map_err(sqlite_error)?;
    apply_0006(conn)?;
    Ok(())
}

fn apply_0006(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    if !column_exists(conn, "episodes", "total_turns_summarized")? {
        conn.execute_batch(
            "ALTER TABLE episodes ADD COLUMN \
             total_turns_summarized INTEGER NOT NULL DEFAULT 0",
        )
        .map_err(sqlite_error)?;
    }
    conn.execute_batch("PRAGMA user_version = 6")
        .map_err(sqlite_error)?;
    apply_0007(conn)?;
    Ok(())
}

fn apply_0007(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    // v0.1.5 split legacy shell.run into explicit shell.shell/shell.exec.
    // Existing broad grants keep the more powerful shell.shell behavior so
    // upgrades do not silently weaken saved approvals.
    conn.execute_batch(
        "UPDATE capability_grants SET capability = 'shell.shell' \
         WHERE capability = 'shell.run'",
    )
    .map_err(sqlite_error)?;
    conn.execute_batch(
        "UPDATE audit_events SET capability = 'shell.shell' \
         WHERE capability = 'shell.run'",
    )
    .map_err(sqlite_error)?;
    conn.execute_batch("PRAGMA user_version = 7")
        .map_err(sqlite_error)?;
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, MemoryError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            params![table],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    Ok(count > 0)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, MemoryError> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql).map_err(sqlite_error)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_error)?;
    for row in rows {
        if row.map_err(sqlite_error)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn row_to_episode(row: &rusqlite::Row<'_>) -> rusqlite::Result<EpisodeRecord> {
    Ok(EpisodeRecord {
        id: row.get(0)?,
        goal: row.get(1)?,
        started_at: row.get(2)?,
        ended_at: row.get(3)?,
        outcome: row.get(4)?,
        iterations: i64_to_u32(row.get(5)?)?,
        tokens_in: i64_to_u32(row.get(6)?)?,
        tokens_out: i64_to_u32(row.get(7)?)?,
        final_message: row.get(8)?,
        error: row.get(9)?,
        model_capability_warning: row.get(10)?,
        corrections_used: i64_to_u32(row.get(11)?)?,
        format_recovery_used: {
            let v: i64 = row.get(12)?;
            v != 0
        },
        response_format_log: row.get(13)?,
        total_turns_summarized: i64_to_u32(row.get(14)?)?,
    })
}

fn row_to_step(row: &rusqlite::Row<'_>) -> rusqlite::Result<StepRecord> {
    let content_json: String = row.get(3)?;
    let content = serde_json::from_str(&content_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(StepRecord {
        episode_id: row.get(0)?,
        step_index: i64_to_u32(row.get(1)?)?,
        role: row.get(2)?,
        content,
        tokens_in: i64_to_u32(row.get(4)?)?,
        tokens_out: i64_to_u32(row.get(5)?)?,
        created_at: row.get(6)?,
    })
}

fn row_to_grant(row: &rusqlite::Row<'_>) -> rusqlite::Result<CapabilityGrantRecord> {
    let capability_text: String = row.get(1)?;
    let scope_text: String = row.get(2)?;
    let capability = capability_text.parse().map_err(|error: String| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, error.into())
    })?;
    let scope = scope_text.parse().map_err(|error: String| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, error.into())
    })?;
    Ok(CapabilityGrantRecord {
        id: row.get(0)?,
        capability,
        scope,
        granted_at: row.get(3)?,
        expires_at: row.get(4)?,
        revoked_at: row.get(5)?,
    })
}

fn row_to_audit(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEventRecord> {
    let capability_text: String = row.get(6)?;
    let capability = capability_text.parse().map_err(|error: String| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, error.into())
    })?;
    Ok(AuditEventRecord {
        id: row.get(0)?,
        episode_id: row.get(1)?,
        timestamp: row.get(2)?,
        tool_name: row.get(3)?,
        input_hash: row.get(4)?,
        output_hash: row.get(5)?,
        capability,
        taint: row.get(7)?,
        cwd: row.get(8)?,
        decision: row.get(9)?,
        previous_hash: row.get(10)?,
        event_hash: row.get(11)?,
    })
}

fn active_grant_locked(
    conn: &Connection,
    capability: Capability,
    require_fresh: bool,
) -> Result<Option<CapabilityGrantRecord>, MemoryError> {
    let now = now_ms();
    let mut stmt = conn
        .prepare(
            "SELECT id, capability, scope, granted_at, expires_at, revoked_at \
             FROM capability_grants \
             WHERE capability = ?1 \
               AND revoked_at IS NULL \
               AND (expires_at IS NULL OR expires_at > ?2) \
             ORDER BY granted_at DESC",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map(params![capability.as_str(), now], row_to_grant)
        .map_err(sqlite_error)?;
    for row in rows {
        let grant = row.map_err(sqlite_error)?;
        if !require_fresh || grant.scope.is_fresh() {
            return Ok(Some(grant));
        }
    }
    Ok(None)
}

fn latest_audit_hash(conn: &Connection) -> Result<String, MemoryError> {
    conn.query_row(
        "SELECT event_hash FROM audit_events ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
    .map(|hash| hash.unwrap_or_else(|| "GENESIS".to_owned()))
    .map_err(sqlite_error)
}

struct AuditHashParts<'a> {
    previous_hash: &'a str,
    episode_id: Option<&'a str>,
    timestamp: i64,
    tool_name: &'a str,
    input_hash: &'a str,
    output_hash: &'a str,
    capability: &'a str,
    taint: &'a str,
    cwd: &'a str,
    decision: &'a str,
}

fn audit_hash(parts: AuditHashParts<'_>) -> String {
    let payload = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        parts.previous_hash,
        parts.episode_id.unwrap_or(""),
        parts.timestamp,
        parts.tool_name,
        parts.input_hash,
        parts.output_hash,
        parts.capability,
        parts.taint,
        parts.cwd,
        parts.decision
    );
    stable_hash_hex(&payload)
}

/// Returns a stable non-cryptographic 64-bit hash for audit payload fields.
pub fn stable_hash_hex(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn i64_to_u32(value: i64) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn lock_conn(
    conn: &Arc<Mutex<Connection>>,
) -> Result<std::sync::MutexGuard<'_, Connection>, MemoryError> {
    conn.lock()
        .map_err(|error| MemoryError::Migration(format!("sqlite mutex poisoned: {error}")))
}

fn sqlite_error(error: rusqlite::Error) -> MemoryError {
    MemoryError::Sqlite(error.to_string())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use helm_core::ContentBlock;
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;

    use super::{Capability, EpisodeOutcome, GrantScope, MIGRATION_0001, MemoryStore, StepRole};

    async fn store() -> (tempfile::TempDir, MemoryStore) {
        let dir = tempdir().unwrap();
        let db = dir.path().join("helm.db");
        let store = MemoryStore::open(&db).await.unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn migration_runs_idempotently_happy_path() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("helm.db");
        let first = MemoryStore::open(&db).await.unwrap();
        drop(first);
        let second = MemoryStore::open(&db).await.unwrap();

        assert_eq!(second.episode_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn full_lifecycle_records_rows() {
        let (_dir, store) = store().await;
        let id = store.start_episode("goal").await.unwrap();
        store
            .log_step(
                &id,
                0,
                StepRole::User,
                &[ContentBlock::Text("goal".to_owned())],
                1,
                0,
            )
            .await
            .unwrap();
        store
            .log_step(
                &id,
                1,
                StepRole::Assistant,
                &[ContentBlock::Text("thinking".to_owned())],
                2,
                3,
            )
            .await
            .unwrap();
        store
            .log_step(
                &id,
                2,
                StepRole::Tool,
                &[ContentBlock::ToolResult {
                    tool_use_id: "toolu".to_owned(),
                    content: "ok".to_owned(),
                    is_error: false,
                }],
                0,
                0,
            )
            .await
            .unwrap();
        store
            .finish_episode(&id, EpisodeOutcome::Success, Some("done"), None)
            .await
            .unwrap();

        let episode = store.episode_by_id(&id).await.unwrap().unwrap();
        assert_eq!(episode.outcome, Some("success".to_owned()));
        assert_eq!(episode.iterations, 1);
        assert_eq!(episode.tokens_in, 3);
        assert_eq!(store.step_count(&id).await.unwrap(), 3);
        let steps = store.get_steps(&id).await.unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].role, "user");
    }

    #[tokio::test]
    async fn warning_round_trips_happy_path() {
        let (_dir, store) = store().await;
        let id = store.start_episode("goal").await.unwrap();

        store
            .set_model_capability_warning(&id, "model emitted tool-shaped text")
            .await
            .unwrap();

        let episode = store.get_episode(&id).await.unwrap().unwrap();
        assert_eq!(
            episode.model_capability_warning,
            Some("model emitted tool-shaped text".to_owned())
        );
    }

    #[tokio::test]
    async fn schema_version_reports_current_version_edge_case() {
        let (_dir, store) = store().await;

        assert_eq!(store.schema_version().await.unwrap(), 7);
    }

    #[tokio::test]
    async fn outcome_counts_group_finished_episodes() {
        let (_dir, store) = store().await;
        let success = store.start_episode("success").await.unwrap();
        let partial = store.start_episode("partial").await.unwrap();
        let failure = store.start_episode("failure").await.unwrap();
        store
            .finish_episode(&success, EpisodeOutcome::Success, Some("ok"), None)
            .await
            .unwrap();
        store
            .finish_episode(&partial, EpisodeOutcome::Partial, Some("p"), None)
            .await
            .unwrap();
        store
            .finish_episode(&failure, EpisodeOutcome::Failure, Some("f"), Some("bad"))
            .await
            .unwrap();

        let counts = store.episode_outcome_counts().await.unwrap();

        assert_eq!(counts.total, 3);
        assert_eq!(counts.success, 1);
        assert_eq!(counts.partial, 1);
        assert_eq!(counts.failure, 1);
    }

    #[tokio::test]
    async fn get_episode_missing_returns_none_error_path() {
        let (_dir, store) = store().await;

        let episode = store.get_episode("missing").await.unwrap();

        assert!(episode.is_none());
    }

    #[tokio::test]
    async fn get_steps_empty_episode_edge_case() {
        let (_dir, store) = store().await;
        let id = store.start_episode("goal").await.unwrap();

        let steps = store.get_steps(&id).await.unwrap();

        assert!(steps.is_empty());
    }

    #[tokio::test]
    async fn old_schema_migrates_to_v2_edge_case() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("old.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_0001).unwrap();
            conn.execute_batch("PRAGMA user_version = 1").unwrap();
        }

        let store = MemoryStore::open(&db).await.unwrap();
        drop(store);
        let conn = Connection::open(&db).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let has_column: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('episodes') WHERE name = 'model_capability_warning'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(version, 7);
        assert_eq!(has_column, 1);
    }

    #[tokio::test]
    async fn grant_lifecycle_happy_path() {
        let (_dir, store) = store().await;
        let grant = store
            .grant_capability(Capability::ShellShell, GrantScope::Once)
            .await
            .unwrap();

        assert_eq!(grant.capability, Capability::ShellShell);
        assert!(
            store
                .active_capability_grant(Capability::ShellShell, true)
                .await
                .unwrap()
                .is_some()
        );
        store.consume_grant_if_once(&grant.id).await.unwrap();
        assert!(
            store
                .active_capability_grant(Capability::ShellShell, true)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn expired_grant_is_rejected_error_path() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("expired.db");
        let store = MemoryStore::open(&db).await.unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "INSERT INTO capability_grants \
             (id, capability, scope, granted_at, expires_at, revoked_at) \
             VALUES ('g1', 'shell.shell', '15m', 1, 2, NULL)",
            [],
        )
        .unwrap();

        let grant = store
            .active_capability_grant(Capability::ShellShell, false)
            .await
            .unwrap();

        assert!(grant.is_none());
    }

    #[tokio::test]
    async fn audit_chain_verifies_happy_path() {
        let (_dir, store) = store().await;
        store
            .append_audit_event(super::AuditEventInput {
                episode_id: Some("ep".to_owned()),
                tool_name: "shell".to_owned(),
                input_hash: super::stable_hash_hex("in"),
                output_hash: super::stable_hash_hex("out"),
                capability: Capability::ShellShell,
                taint: helm_core::Taint::User,
                cwd: "/tmp".to_owned(),
                decision: "allow".to_owned(),
            })
            .await
            .unwrap();

        let verification = store.verify_audit_chain().await.unwrap();

        assert!(verification.ok);
        assert_eq!(verification.checked, 1);
    }

    #[tokio::test]
    async fn audit_chain_detects_tampering_error_path() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("audit.db");
        let store = MemoryStore::open(&db).await.unwrap();
        store
            .append_audit_event(super::AuditEventInput {
                episode_id: Some("ep".to_owned()),
                tool_name: "shell".to_owned(),
                input_hash: super::stable_hash_hex("in"),
                output_hash: super::stable_hash_hex("out"),
                capability: Capability::ShellShell,
                taint: helm_core::Taint::User,
                cwd: "/tmp".to_owned(),
                decision: "allow".to_owned(),
            })
            .await
            .unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute("UPDATE audit_events SET decision = 'deny' WHERE id = 1", [])
            .unwrap();

        let verification = store.verify_audit_chain().await.unwrap();

        assert!(!verification.ok);
        assert_eq!(verification.failed_at, Some(1));
    }

    #[tokio::test]
    async fn wal_mode_is_enabled_edge_case() {
        let (_dir, store) = store().await;

        assert_eq!(store.journal_mode().await.unwrap().to_lowercase(), "wal");
    }

    #[tokio::test]
    async fn concurrent_log_step_does_not_corrupt_error_path() {
        let (_dir, store) = store().await;
        let id = store.start_episode("goal").await.unwrap();
        let left_content = [ContentBlock::Text("a".to_owned())];
        let right_content = [ContentBlock::Text("b".to_owned())];
        let left = store.log_step(&id, 0, StepRole::Assistant, &left_content, 1, 1);
        let right = store.log_step(&id, 1, StepRole::Assistant, &right_content, 1, 1);

        let (left_result, right_result) = tokio::join!(left, right);

        left_result.unwrap();
        right_result.unwrap();
        assert_eq!(store.step_count(&id).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn recent_episodes_respects_limit() {
        let (_dir, store) = store().await;
        let first = store.start_episode("first").await.unwrap();
        store
            .log_step(
                &first,
                0,
                StepRole::User,
                &[ContentBlock::Text(json!("one").to_string())],
                0,
                0,
            )
            .await
            .unwrap();
        let second = store.start_episode("second").await.unwrap();

        let episodes = store.recent_episodes(1).await.unwrap();

        assert_eq!(episodes.len(), 1);
        assert!(episodes[0].id == first || episodes[0].id == second);
    }

    #[test]
    fn enum_strings_match_schema() {
        assert_eq!(EpisodeOutcome::Partial.as_str(), "partial");
        assert_eq!(StepRole::System.as_str(), "system");
    }
}
