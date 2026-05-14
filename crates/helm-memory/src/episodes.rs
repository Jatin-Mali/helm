//! Episode log API backed by SQLite.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
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
const MIGRATION_0009: &str = include_str!("../migrations/0005_remote_audit.sql");
const MIGRATION_0010: &str = include_str!("../migrations/0006_snapshots.sql");
const MIGRATION_0011: &str = include_str!("../migrations/0007_findings.sql");
const MIGRATION_0012: &str = include_str!("../migrations/0008_changesets.sql");

/// Minimal schema for per-host audit shard DBs (audit_events only).
const SHARD_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS audit_events (\
    id INTEGER PRIMARY KEY,\
    episode_id TEXT,\
    target TEXT,\
    timestamp INTEGER NOT NULL,\
    tool_name TEXT NOT NULL,\
    input_hash TEXT NOT NULL DEFAULT '',\
    output_hash TEXT NOT NULL DEFAULT '',\
    capability TEXT NOT NULL DEFAULT '',\
    taint TEXT NOT NULL DEFAULT 'clean',\
    cwd TEXT NOT NULL DEFAULT '',\
    decision TEXT NOT NULL DEFAULT '',\
    previous_hash TEXT NOT NULL DEFAULT '',\
    event_hash TEXT NOT NULL\
);\
CREATE INDEX IF NOT EXISTS idx_audit_shard_episode ON audit_events(episode_id, id);\
CREATE INDEX IF NOT EXISTS idx_audit_shard_target ON audit_events(target, id);";

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
    /// The task was cancelled by the user (Ctrl+C or token).
    Cancelled,
    /// The task exceeded the cost budget limit.
    BudgetExceeded,
}

impl EpisodeOutcome {
    /// Returns the SQLite representation required by the schema.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
            Self::BudgetExceeded => "budget_exceeded",
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
    /// Remote target identity, if the tool call was scoped to a named host.
    pub target: Option<String>,
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
    /// Remote target identity, if any.
    pub target: Option<String>,
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
    /// Directory where per-host audit shard DBs are written (sibling of main helm.db).
    audit_dir: Option<PathBuf>,
    /// Lazily-opened per-host shard connections keyed by sanitized host name.
    audit_shards: Arc<Mutex<HashMap<String, Arc<Mutex<Connection>>>>>,
}

impl MemoryStore {
    /// Opens or creates a SQLite database and runs HELM migrations idempotently.
    pub async fn open(path: &Path) -> Result<Self, MemoryError> {
        let path = path.to_path_buf();
        let audit_dir = path.parent().map(|p| p.join("audit"));
        let conn = tokio::task::spawn_blocking(move || {
            let conn = Connection::open(path).map_err(sqlite_error)?;
            run_migrations(&conn)?;
            Ok::<Connection, MemoryError>(conn)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            audit_dir,
            audit_shards: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Returns (opening if necessary) the audit shard connection for `host`.
    fn get_or_open_shard(&self, host: &str) -> Option<Arc<Mutex<Connection>>> {
        let dir = self.audit_dir.as_ref()?;
        let safe = sanitize_host(host);
        if safe.is_empty() {
            return None;
        }
        let mut shards = self.audit_shards.lock().ok()?;
        if let Some(conn) = shards.get(&safe) {
            return Some(Arc::clone(conn));
        }
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("helm: audit shard dir create failed: {e}");
            return None;
        }
        let db_path = dir.join(format!("{safe}.db"));
        match Connection::open(&db_path) {
            Ok(conn) => {
                if let Err(e) = conn.execute_batch(SHARD_SCHEMA) {
                    eprintln!("helm: audit shard schema failed for {safe}: {e}");
                    return None;
                }
                let arc = Arc::new(Mutex::new(conn));
                shards.insert(safe, Arc::clone(&arc));
                Some(arc)
            }
            Err(e) => {
                eprintln!("helm: audit shard open failed for {safe}: {e}");
                None
            }
        }
    }

    /// Starts a new episode for `goal` and returns its UUID.
    pub async fn start_episode(&self, goal: &str) -> Result<EpisodeId, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let goal = helm_core::redact_secrets(goal);
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
        let content_json = helm_core::redact_secrets(&content_json);
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
        let final_message = final_message.map(helm_core::redact_secrets);
        let error = error.map(helm_core::redact_secrets);
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
        let warning = helm_core::redact_secrets(warning);
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
    pub async fn append_audit_event(
        &self,
        mut input: AuditEventInput,
    ) -> Result<String, MemoryError> {
        let conn = Arc::clone(&self.conn);
        input.input_hash = helm_core::redact_secrets(&input.input_hash);
        input.output_hash = helm_core::redact_secrets(&input.output_hash);
        input.cwd = helm_core::redact_secrets(&input.cwd);
        input.decision = helm_core::redact_secrets(&input.decision);
        input.target = input
            .target
            .map(|target| helm_core::redact_secrets(&target).trim().to_owned())
            .filter(|target| !target.is_empty());

        // Grab shard connection before entering spawn_blocking.
        let shard = input
            .target
            .as_deref()
            .and_then(|h| self.get_or_open_shard(h));

        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let previous_hash = latest_audit_hash(&guard, input.target.as_deref())?;
            let timestamp = now_ms();
            let taint = input.taint.label();
            let event_hash = audit_hash(AuditHashParts {
                previous_hash: &previous_hash,
                episode_id: input.episode_id.as_deref(),
                target: input.target.as_deref(),
                timestamp,
                tool_name: &input.tool_name,
                input_hash: &input.input_hash,
                output_hash: &input.output_hash,
                capability: input.capability.as_str(),
                taint: &taint,
                cwd: &input.cwd,
                decision: &input.decision,
            });
            let insert_sql = "INSERT INTO audit_events \
                     (episode_id, target, timestamp, tool_name, input_hash, output_hash, capability, \
                      taint, cwd, decision, previous_hash, event_hash) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)";
            let insert_params = params![
                input.episode_id,
                input.target,
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
            ];
            guard
                .execute(insert_sql, insert_params)
                .map_err(sqlite_error)?;

            // Mirror to per-host shard DB when a target is set.
            if let Some(shard_conn) = shard {
                if let Ok(sg) = shard_conn.lock() {
                    let _ = sg.execute(
                        insert_sql,
                        params![
                            input.episode_id,
                            input.target,
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
                    );
                }
            }

            Ok::<String, MemoryError>(event_hash)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Returns audit events, optionally filtered by episode id.
    pub async fn audit_events(
        &self,
        episode_id: Option<&str>,
        target: Option<&str>,
    ) -> Result<Vec<AuditEventRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let episode_id = episode_id.map(str::to_owned);
        let target = target.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let sql = match (episode_id.is_some(), target.is_some()) {
                (true, true) => {
                    "SELECT id, episode_id, target, timestamp, tool_name, input_hash, output_hash, \
                     capability, taint, cwd, decision, previous_hash, event_hash \
                     FROM audit_events WHERE episode_id = ?1 AND target IS ?2 ORDER BY id ASC"
                }
                (true, false) => {
                    "SELECT id, episode_id, target, timestamp, tool_name, input_hash, output_hash, \
                     capability, taint, cwd, decision, previous_hash, event_hash \
                     FROM audit_events WHERE episode_id = ?1 ORDER BY id ASC"
                }
                (false, true) => {
                    "SELECT id, episode_id, target, timestamp, tool_name, input_hash, output_hash, \
                     capability, taint, cwd, decision, previous_hash, event_hash \
                     FROM audit_events WHERE target IS ?1 ORDER BY id ASC"
                }
                (false, false) => {
                    "SELECT id, episode_id, target, timestamp, tool_name, input_hash, output_hash, \
                     capability, taint, cwd, decision, previous_hash, event_hash \
                     FROM audit_events ORDER BY id ASC"
                }
            };
            let mut stmt = guard.prepare(sql).map_err(sqlite_error)?;
            let mut records = Vec::new();
            match (episode_id.as_deref(), target.as_deref()) {
                (Some(id), Some(target_name)) => {
                    let rows = stmt
                        .query_map(params![id, target_name], row_to_audit)
                        .map_err(sqlite_error)?;
                    for row in rows {
                        records.push(row.map_err(sqlite_error)?);
                    }
                }
                (Some(id), None) => {
                    let rows = stmt
                        .query_map(params![id], row_to_audit)
                        .map_err(sqlite_error)?;
                    for row in rows {
                        records.push(row.map_err(sqlite_error)?);
                    }
                }
                (None, Some(target_name)) => {
                    let rows = stmt
                        .query_map(params![target_name], row_to_audit)
                        .map_err(sqlite_error)?;
                    for row in rows {
                        records.push(row.map_err(sqlite_error)?);
                    }
                }
                (None, None) => {
                    let rows = stmt.query_map([], row_to_audit).map_err(sqlite_error)?;
                    for row in rows {
                        records.push(row.map_err(sqlite_error)?);
                    }
                }
            }
            Ok::<Vec<AuditEventRecord>, MemoryError>(records)
        })
        .await
        .map_err(|error| MemoryError::Join(error.to_string()))?
    }

    /// Verifies every audit event hash against the previous row.
    pub async fn verify_audit_chain(&self) -> Result<AuditVerification, MemoryError> {
        let events = self.audit_events(None, None).await?;
        verify_partitioned_audit_events(events)
    }

    /// Verifies one audit partition, optionally filtered by remote target.
    pub async fn verify_audit_chain_for_target(
        &self,
        target: Option<&str>,
    ) -> Result<AuditVerification, MemoryError> {
        let events = self.audit_events(None, target).await?;
        verify_partitioned_audit_events(events)
    }

    /// Record a single routing outcome for `model` (and optionally `provider`).
    #[allow(clippy::too_many_arguments)]
    pub async fn record_routing_outcome(
        &self,
        model: &str,
        provider: Option<&str>,
        success: bool,
        latency_ms: u64,
        tokens_in: u32,
        tokens_out: u32,
        cost_usd: f64,
        episode_id: Option<&str>,
    ) -> Result<(), MemoryError> {
        let conn = Arc::clone(&self.conn);
        let model = model.to_owned();
        let provider = provider.map(str::to_owned);
        let episode_id = episode_id.map(str::to_owned);
        let now = chrono::Utc::now().timestamp_millis();
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|e| MemoryError::Sqlite(e.to_string()))?;
            guard
                .execute(
                    "INSERT INTO router_outcomes \
                     (model, provider, success, latency_ms, tokens_in, tokens_out, cost_usd, episode_id, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        model,
                        provider,
                        success as i64,
                        latency_ms as i64,
                        tokens_in as i64,
                        tokens_out as i64,
                        cost_usd,
                        episode_id,
                        now,
                    ],
                )
                .map_err(sqlite_error)?;
            Ok::<(), MemoryError>(())
        })
        .await
        .map_err(|e| MemoryError::Join(e.to_string()))?
    }

    /// Aggregate routing stats grouped by model.
    pub async fn routing_stats(&self) -> Result<Vec<RoutingStat>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|e| MemoryError::Sqlite(e.to_string()))?;
            let mut stmt = guard
                .prepare(
                    "SELECT model, \
                            COUNT(*) AS total, \
                            SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) AS ok, \
                            AVG(latency_ms) AS avg_latency, \
                            SUM(cost_usd) AS total_cost \
                     FROM router_outcomes GROUP BY model ORDER BY total DESC",
                )
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    let total: i64 = row.get(1)?;
                    let ok: i64 = row.get(2)?;
                    let avg_latency: Option<f64> = row.get(3)?;
                    let total_cost: Option<f64> = row.get(4)?;
                    Ok(RoutingStat {
                        model: row.get(0)?,
                        total: total as u64,
                        successes: ok as u64,
                        avg_latency_ms: avg_latency.unwrap_or(0.0),
                        total_cost_usd: total_cost.unwrap_or(0.0),
                    })
                })
                .map_err(sqlite_error)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(sqlite_error)?);
            }
            Ok::<Vec<RoutingStat>, MemoryError>(out)
        })
        .await
        .map_err(|e| MemoryError::Join(e.to_string()))?
    }
}

fn verify_partitioned_audit_events(
    events: Vec<AuditEventRecord>,
) -> Result<AuditVerification, MemoryError> {
    let mut previous_by_target: std::collections::HashMap<Option<String>, String> =
        std::collections::HashMap::new();
    let mut checked = 0_u32;
    for event in events {
        checked = checked.saturating_add(1);
        let key = event.target.clone();
        let previous = previous_by_target
            .entry(key.clone())
            .or_insert_with(|| "GENESIS".to_owned())
            .clone();
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
            target: event.target.as_deref(),
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
        previous_by_target.insert(key, event.event_hash);
    }
    Ok(AuditVerification {
        ok: true,
        checked,
        failed_at: None,
        reason: None,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoutingStat {
    pub model: String,
    pub total: u64,
    pub successes: u64,
    pub avg_latency_ms: f64,
    pub total_cost_usd: f64,
}

impl RoutingStat {
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.successes as f64) / (self.total as f64)
        }
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
        7 => apply_0008(conn),
        8 => apply_0009(conn),
        9 => apply_0010(conn),
        10 => apply_0011(conn),
        11 => {
            conn.execute_batch("PRAGMA foreign_keys = ON")
                .map_err(sqlite_error)?;
            apply_0012(conn)
        }
        12 => conn
            .execute_batch("PRAGMA foreign_keys = ON")
            .map_err(sqlite_error),
        other => Err(MemoryError::Migration(format!(
            "unsupported schema version: {other}"
        ))),
    }
}

fn apply_0009(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    if !column_exists(conn, "audit_events", "target")? {
        conn.execute_batch(MIGRATION_0009).map_err(sqlite_error)?;
    }
    conn.execute_batch("PRAGMA user_version = 9")
        .map_err(sqlite_error)?;
    apply_0010(conn)?;
    Ok(())
}

fn apply_0010(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    if !table_exists(conn, "snapshots")? {
        conn.execute_batch(MIGRATION_0010).map_err(sqlite_error)?;
    }
    conn.execute_batch("PRAGMA user_version = 10")
        .map_err(sqlite_error)?;
    apply_0011(conn)?;
    Ok(())
}

fn apply_0011(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    if !column_exists(conn, "snapshots", "findings_json")? {
        conn.execute_batch(MIGRATION_0011).map_err(sqlite_error)?;
    }
    conn.execute_batch("PRAGMA user_version = 11")
        .map_err(sqlite_error)?;
    apply_0012(conn)
}

fn apply_0012(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    if !table_exists(conn, "change_sets")? {
        conn.execute_batch(MIGRATION_0012).map_err(sqlite_error)?;
    }
    conn.execute_batch("PRAGMA user_version = 12")
        .map_err(sqlite_error)?;
    Ok(())
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
    apply_0008(conn)?;
    Ok(())
}

fn apply_0008(conn: &Connection) -> Result<(), MemoryError> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(sqlite_error)?;
    if !table_exists(conn, "router_outcomes")? {
        conn.execute_batch(
            "CREATE TABLE router_outcomes (\
                 id INTEGER PRIMARY KEY AUTOINCREMENT,\
                 model TEXT NOT NULL,\
                 provider TEXT,\
                 success INTEGER NOT NULL,\
                 latency_ms INTEGER NOT NULL DEFAULT 0,\
                 tokens_in INTEGER NOT NULL DEFAULT 0,\
                 tokens_out INTEGER NOT NULL DEFAULT 0,\
                 cost_usd REAL NOT NULL DEFAULT 0.0,\
                 episode_id TEXT,\
                 created_at INTEGER NOT NULL\
             );\
             CREATE INDEX IF NOT EXISTS idx_router_outcomes_model ON router_outcomes(model);",
        )
        .map_err(sqlite_error)?;
    }
    conn.execute_batch("PRAGMA user_version = 8")
        .map_err(sqlite_error)?;
    apply_0009(conn)?;
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
    let capability_text: String = row.get(7)?;
    let capability = capability_text.parse().map_err(|error: String| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, error.into())
    })?;
    Ok(AuditEventRecord {
        id: row.get(0)?,
        episode_id: row.get(1)?,
        target: row.get(2)?,
        timestamp: row.get(3)?,
        tool_name: row.get(4)?,
        input_hash: row.get(5)?,
        output_hash: row.get(6)?,
        capability,
        taint: row.get(8)?,
        cwd: row.get(9)?,
        decision: row.get(10)?,
        previous_hash: row.get(11)?,
        event_hash: row.get(12)?,
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

fn latest_audit_hash(conn: &Connection, target: Option<&str>) -> Result<String, MemoryError> {
    let sql = "SELECT event_hash FROM audit_events WHERE target IS ?1 ORDER BY id DESC LIMIT 1";
    conn.query_row(sql, params![target], |row| row.get(0))
        .optional()
        .map(|hash| hash.unwrap_or_else(|| "GENESIS".to_owned()))
        .map_err(sqlite_error)
}

struct AuditHashParts<'a> {
    previous_hash: &'a str,
    episode_id: Option<&'a str>,
    target: Option<&'a str>,
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
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        parts.previous_hash,
        parts.episode_id.unwrap_or(""),
        parts.target.unwrap_or(""),
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

/// Sanitize a remote host string so it can be used as a filename component.
fn sanitize_host(host: &str) -> String {
    host.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
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
    async fn secrets_are_redacted_before_persistence() {
        let (_dir, store) = store().await;
        let id = store
            .start_episode(
                "inspect /home/test/.helm/secrets.toml with sk-or-abcdefghijklmnopqrstuvwxyz123456",
            )
            .await
            .unwrap();
        store
            .set_model_capability_warning(
                &id,
                "warning for /home/test/.helm/helm.db and sk-ant-api03-abcdefghijklmnopqrstuvwxyz123456",
            )
            .await
            .unwrap();
        store
            .finish_episode(
                &id,
                EpisodeOutcome::Failure,
                Some("see ~/.helm/helm.log"),
                Some("gsk_abcdefghijklmnopqrstuvwxyz123456"),
            )
            .await
            .unwrap();

        let episode = store.get_episode(&id).await.unwrap().unwrap();
        assert!(!episode.goal.contains(".helm/secrets.toml"));
        assert!(!episode.goal.contains("abcdefghijklmnopqrstuvwxyz123456"));
        assert_eq!(episode.goal.matches("[REDACTED_PATH]").count(), 1);
        assert_eq!(
            episode.model_capability_warning,
            Some("warning for [REDACTED_PATH] and ***REDACTED***".to_owned())
        );
        assert_eq!(
            episode.final_message,
            Some("see [REDACTED_PATH]".to_owned())
        );
        assert_eq!(episode.error, Some("***REDACTED***".to_owned()));
    }

    #[tokio::test]
    async fn schema_version_reports_current_version_edge_case() {
        let (_dir, store) = store().await;

        assert_eq!(store.schema_version().await.unwrap(), 12);
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

        assert_eq!(version, 12);
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
                target: None,
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
                target: None,
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
    async fn audit_chain_partitions_by_target() {
        let (_dir, store) = store().await;
        for target in [Some("prod-1".to_owned()), Some("prod-2".to_owned())] {
            store
                .append_audit_event(super::AuditEventInput {
                    episode_id: Some("ep".to_owned()),
                    target: target.clone(),
                    tool_name: "ssh".to_owned(),
                    input_hash: super::stable_hash_hex("in"),
                    output_hash: super::stable_hash_hex("out"),
                    capability: Capability::NetworkOut,
                    taint: helm_core::Taint::User,
                    cwd: "/tmp".to_owned(),
                    decision: "allow".to_owned(),
                })
                .await
                .unwrap();
        }

        let verification = store
            .verify_audit_chain_for_target(Some("prod-1"))
            .await
            .unwrap();

        assert!(verification.ok);
        assert_eq!(verification.checked, 1);
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
