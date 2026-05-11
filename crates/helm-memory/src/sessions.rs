//! Session persistence for TUI: list, delete, export, resume.
//!
//! Sessions are TUI conversational units that can survive restarts.
//! Each session wraps an episode with additional metadata: name, auto-save state.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{EpisodeId, MemoryError};

const MIGRATION_SESSION: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    goal TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    model TEXT,
    provider TEXT,
    working_dir TEXT
);

CREATE TABLE IF NOT EXISTS session_snapshots (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    step_index INTEGER NOT NULL,
    content_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE INDEX IF NOT EXISTS idx_snapshots_session ON session_snapshots(session_id);
"#;

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn sqlite_error<E: std::fmt::Display>(e: E) -> MemoryError {
    MemoryError::Sqlite(e.to_string())
}

fn lock_conn(
    conn: &Arc<Mutex<Connection>>,
) -> Result<std::sync::MutexGuard<'_, Connection>, MemoryError> {
    conn.lock().map_err(|e| MemoryError::Sqlite(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub name: String,
    pub goal: String,
    pub episode_id: EpisodeId,
    pub created_at: i64,
    pub updated_at: i64,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRecord {
    pub id: String,
    pub session_id: String,
    pub step_index: u32,
    pub file_path: PathBuf,
    pub created_at: i64,
}

pub struct SessionStore {
    conn: Arc<Mutex<Connection>>,
    snapshots_dir: PathBuf,
}

impl SessionStore {
    pub async fn open(db_path: &Path, snapshots_dir: PathBuf) -> Result<Self, MemoryError> {
        let conn = Connection::open(db_path).map_err(sqlite_error)?;
        conn.execute_batch(MIGRATION_SESSION)
            .map_err(sqlite_error)?;
        if !snapshots_dir.exists() {
            fs::create_dir_all(&snapshots_dir).map_err(|e| MemoryError::Other(e.to_string()))?;
        }
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            snapshots_dir,
        })
    }

    pub async fn create_session(
        &self,
        name: &str,
        goal: &str,
        episode_id: EpisodeId,
        model: Option<String>,
        provider: Option<String>,
        working_dir: Option<String>,
    ) -> Result<String, MemoryError> {
        let id = Uuid::new_v4().to_string();
        let id_for_insert = id.clone();
        let conn = Arc::clone(&self.conn);
        let name = name.to_owned();
        let goal = helm_core::redact_secrets(goal);
        let episode_id_owned = episode_id;
        let model_owned = model;
        let provider_owned = provider;
        let working_dir_owned = working_dir;
        let now = now_ms();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            guard.execute(
                "INSERT INTO sessions (id, name, goal, episode_id, created_at, updated_at, model, provider, working_dir) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id_for_insert, name, goal, episode_id_owned, now, now, model_owned, provider_owned, working_dir_owned],
            ).map_err(sqlite_error)?;
            Ok::<(), MemoryError>(())
        }).await.map_err(|e| MemoryError::Join(e.to_string()))??;
        Ok(id)
    }

    pub async fn list_sessions(&self, limit: u32) -> Result<Vec<SessionRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let mut stmt = guard.prepare(
                "SELECT id, name, goal, episode_id, created_at, updated_at, model, provider, working_dir FROM sessions ORDER BY updated_at DESC LIMIT ?1"
            ).map_err(sqlite_error)?;
            let rows = stmt.query_map(params![i64::from(limit)], |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    goal: row.get(2)?,
                    episode_id: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    model: row.get(6)?,
                    provider: row.get(7)?,
                    working_dir: row.get(8)?,
                })
            }).map_err(sqlite_error)?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(sqlite_error)?);
            }
            Ok::<Vec<SessionRecord>, MemoryError>(records)
        }).await.map_err(|e| MemoryError::Join(e.to_string()))?
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<SessionRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let result = guard.query_row(
                "SELECT id, name, goal, episode_id, created_at, updated_at, model, provider, working_dir FROM sessions WHERE id = ?1",
                params![id],
                |row| Ok(SessionRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    goal: row.get(2)?,
                    episode_id: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    model: row.get(6)?,
                    provider: row.get(7)?,
                    working_dir: row.get(8)?,
                })
            ).optional().map_err(sqlite_error)?;
            Ok::<Option<SessionRecord>, MemoryError>(result)
        }).await.map_err(|e| MemoryError::Join(e.to_string()))?
    }

    pub async fn delete_session(&self, id: &str) -> Result<u32, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let _snapshots_dir = self.snapshots_dir.clone();
        let id_owned = id.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let mut stmt = guard
                .prepare("SELECT file_path FROM session_snapshots WHERE session_id = ?1")
                .map_err(sqlite_error)?;
            let paths: Vec<String> = stmt
                .query_map(params![id_owned], |row| row.get(0))
                .map_err(sqlite_error)?
                .filter_map(|r| r.ok())
                .collect();
            for path in &paths {
                let _ = fs::remove_file(path);
            }
            guard
                .execute(
                    "DELETE FROM session_snapshots WHERE session_id = ?1",
                    params![id_owned.clone()],
                )
                .map_err(sqlite_error)?;
            let deleted = guard
                .execute("DELETE FROM sessions WHERE id = ?1", params![id_owned])
                .map_err(sqlite_error)?;
            Ok::<u32, MemoryError>(deleted as u32)
        })
        .await
        .map_err(|e| MemoryError::Join(e.to_string()))?
    }

    pub async fn export_session(&self, id: &str, format: &str) -> Result<String, MemoryError> {
        let session = self
            .get_session(id)
            .await?
            .ok_or_else(|| MemoryError::NotFound("session not found".into()))?;
        let content = match format {
            "json" => serde_json::to_string_pretty(&session)
                .map_err(|e| MemoryError::Other(e.to_string()))?,
            "md" => {
                let ts = DateTime::from_timestamp_millis(session.created_at)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                format!(
                    "# Session: {}\n\n**Goal:** {}\n**Created:** {}\n**Model:** {}\n**Provider:** {}\n**Episode ID:** {}\n",
                    session.name,
                    session.goal,
                    ts,
                    session.model.as_deref().unwrap_or("unknown"),
                    session.provider.as_deref().unwrap_or("unknown"),
                    session.episode_id
                )
            }
            _ => return Err(MemoryError::InvalidInput("unsupported format".into())),
        };
        Ok(content)
    }

    pub async fn take_snapshot(
        &self,
        session_id: &str,
        step_index: u32,
        content_json: &str,
        file_path: &Path,
    ) -> Result<String, MemoryError> {
        let id = Uuid::new_v4().to_string();
        let id_clone = id.clone();
        let conn = Arc::clone(&self.conn);
        let session_id_clone = session_id.to_owned();
        let file_path_str = file_path.to_string_lossy().to_string();
        let content_json = helm_core::redact_secrets(content_json);
        let now = now_ms();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            guard.execute(
                "INSERT INTO session_snapshots (id, session_id, step_index, content_json, created_at, file_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id_clone, session_id_clone, step_index, content_json, now, file_path_str],
            ).map_err(sqlite_error)?;
            Ok::<(), MemoryError>(())
        }).await.map_err(|e| MemoryError::Join(e.to_string()))??;
        Ok(id)
    }

    pub async fn list_snapshots(
        &self,
        session_id: &str,
    ) -> Result<Vec<SnapshotRecord>, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let session_id = session_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let mut stmt = guard.prepare(
                "SELECT id, session_id, step_index, created_at, file_path FROM session_snapshots WHERE session_id = ?1 ORDER BY step_index DESC"
            ).map_err(sqlite_error)?;
            let rows = stmt.query_map(params![session_id], |row| {
                Ok(SnapshotRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    step_index: row.get(2)?,
                    created_at: row.get(3)?,
                    file_path: PathBuf::from(row.get::<_, String>(4)?),
                })
            }).map_err(sqlite_error)?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(sqlite_error)?);
            }
            Ok::<Vec<SnapshotRecord>, MemoryError>(records)
        }).await.map_err(|e| MemoryError::Join(e.to_string()))?
    }

    pub async fn restore_snapshot(&self, snapshot_id: &str) -> Result<String, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let snapshot_id = snapshot_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let content: String = guard
                .query_row(
                    "SELECT content_json FROM session_snapshots WHERE id = ?1",
                    params![snapshot_id],
                    |row| row.get(0),
                )
                .map_err(sqlite_error)?;
            Ok::<String, MemoryError>(content)
        })
        .await
        .map_err(|e| MemoryError::Join(e.to_string()))?
    }

    /// Write the snapshot's saved content back to the recorded `file_path`
    /// (or to `override_path` when supplied). Creates parent dirs and is atomic.
    pub async fn apply_snapshot(
        &self,
        snapshot_id: &str,
        override_path: Option<PathBuf>,
    ) -> Result<PathBuf, MemoryError> {
        let conn = Arc::clone(&self.conn);
        let snapshot_id = snapshot_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = lock_conn(&conn)?;
            let (file_path, content): (String, String) = guard
                .query_row(
                    "SELECT file_path, content_json FROM session_snapshots WHERE id = ?1",
                    params![snapshot_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(sqlite_error)?;
            let target = override_path.unwrap_or_else(|| PathBuf::from(&file_path));
            if let Some(parent) = target.parent()
                && !parent.as_os_str().is_empty()
                && !parent.exists()
            {
                fs::create_dir_all(parent).map_err(|e| MemoryError::Other(e.to_string()))?;
            }
            let tmp = target.with_extension(format!(
                "helm-snap-{}.tmp",
                std::process::id()
            ));
            fs::write(&tmp, content.as_bytes())
                .map_err(|e| MemoryError::Other(e.to_string()))?;
            fs::rename(&tmp, &target).map_err(|e| MemoryError::Other(e.to_string()))?;
            Ok::<PathBuf, MemoryError>(target)
        })
        .await
        .map_err(|e| MemoryError::Join(e.to_string()))?
    }

    /// Find the most recent snapshot for a session id.
    pub async fn latest_snapshot(
        &self,
        session_id: &str,
    ) -> Result<Option<SnapshotRecord>, MemoryError> {
        let mut snaps = self.list_snapshots(session_id).await?;
        Ok(snaps.drain(..).next())
    }
}
