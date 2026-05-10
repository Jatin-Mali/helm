//! Procedural memory — stores and retrieves reusable step sequences for HELM.
//!
//! A "procedure" is a named sequence of tool calls that successfully completed
//! a goal matching a given pattern.  The agent can look these up before
//! planning to avoid re-deriving known solutions.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ProcedureError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("lock poisoned")]
    Lock,
}

// ── Domain types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureStep {
    pub tool: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct Procedure {
    pub id: String,
    pub goal_pattern: String,
    pub steps: Vec<ProcedureStep>,
    pub success_count: u32,
    pub last_used: Option<String>,
}

// ── ProcedureStore ────────────────────────────────────────────────────────────

pub struct ProcedureStore {
    conn: Arc<Mutex<Connection>>,
}

impl ProcedureStore {
    pub fn open(path: &Path) -> Result<Self, ProcedureError> {
        let conn = Connection::open(path)?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> Result<Self, ProcedureError> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a new procedure.  Returns its generated id.
    pub fn insert(&self, goal_pattern: &str, steps: &[ProcedureStep]) -> Result<String, ProcedureError> {
        let id = uuid_v4();
        let steps_json = serde_json::to_string(steps)?;
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO procedures (id, goal_pattern, steps, success_count)
             VALUES (?1, ?2, ?3, 0)",
            params![id, goal_pattern, steps_json],
        )?;
        Ok(id)
    }

    /// Find all procedures whose goal_pattern is a substring match for `goal`.
    pub fn find_by_goal(&self, goal: &str, limit: u32) -> Result<Vec<Procedure>, ProcedureError> {
        let conn = lock(&self.conn)?;
        let pattern = format!("%{goal}%");
        let mut stmt = conn.prepare(
            "SELECT id, goal_pattern, steps, success_count, last_used
             FROM procedures
             WHERE goal_pattern LIKE ?1
             ORDER BY success_count DESC, last_used DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit], row_to_procedure)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(ProcedureError::Sqlite)
    }

    /// Increment success_count and update last_used for a procedure.
    pub fn record_success(&self, id: &str) -> Result<(), ProcedureError> {
        let conn = lock(&self.conn)?;
        conn.execute(
            "UPDATE procedures
             SET success_count = success_count + 1,
                 last_used = strftime('%Y-%m-%dT%H:%M:%SZ','now')
             WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// List all procedures ordered by success_count descending.
    pub fn list(&self, limit: u32) -> Result<Vec<Procedure>, ProcedureError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn.prepare(
            "SELECT id, goal_pattern, steps, success_count, last_used
             FROM procedures ORDER BY success_count DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], row_to_procedure)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(ProcedureError::Sqlite)
    }

    pub fn get(&self, id: &str) -> Result<Option<Procedure>, ProcedureError> {
        let conn = lock(&self.conn)?;
        conn.query_row(
            "SELECT id, goal_pattern, steps, success_count, last_used FROM procedures WHERE id = ?1",
            params![id],
            row_to_procedure,
        )
        .optional()
        .map_err(ProcedureError::Sqlite)
    }

    pub fn delete(&self, id: &str) -> Result<(), ProcedureError> {
        let conn = lock(&self.conn)?;
        conn.execute("DELETE FROM procedures WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn count(&self) -> Result<u64, ProcedureError> {
        let conn = lock(&self.conn)?;
        conn.query_row("SELECT COUNT(*) FROM procedures", [], |row| row.get::<_, i64>(0))
            .map(|n| n as u64)
            .map_err(ProcedureError::Sqlite)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn row_to_procedure(row: &rusqlite::Row<'_>) -> rusqlite::Result<Procedure> {
    let steps_str: String = row.get(2)?;
    let steps: Vec<ProcedureStep> = serde_json::from_str(&steps_str).unwrap_or_default();
    Ok(Procedure {
        id: row.get(0)?,
        goal_pattern: row.get(1)?,
        steps,
        success_count: row.get::<_, i64>(3)? as u32,
        last_used: row.get(4)?,
    })
}

fn lock(conn: &Arc<Mutex<Connection>>) -> Result<MutexGuard<'_, Connection>, ProcedureError> {
    conn.lock().map_err(|_| ProcedureError::Lock)
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn run_migrations(conn: &Connection) -> Result<(), ProcedureError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS procedures (
            id            TEXT PRIMARY KEY,
            goal_pattern  TEXT NOT NULL,
            steps         TEXT NOT NULL DEFAULT '[]',
            success_count INTEGER NOT NULL DEFAULT 0,
            last_used     TEXT,
            created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
        );
        CREATE INDEX IF NOT EXISTS idx_procedures_goal ON procedures(goal_pattern);
        CREATE INDEX IF NOT EXISTS idx_procedures_hits ON procedures(success_count DESC);",
    )
    .map_err(ProcedureError::Sqlite)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ProcedureStep, ProcedureStore};

    fn store() -> ProcedureStore {
        ProcedureStore::open_in_memory().unwrap()
    }

    fn steps(tools: &[&str]) -> Vec<ProcedureStep> {
        tools
            .iter()
            .map(|t| ProcedureStep {
                tool: t.to_string(),
                input: json!({}),
            })
            .collect()
    }

    #[test]
    fn insert_and_get_happy_path() {
        let s = store();
        let id = s.insert("deploy nginx", &steps(&["shell", "service"])).unwrap();
        let p = s.get(&id).unwrap().unwrap();
        assert_eq!(p.goal_pattern, "deploy nginx");
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.success_count, 0);
    }

    #[test]
    fn find_by_goal_substring_happy_path() {
        let s = store();
        s.insert("deploy nginx on staging", &steps(&["shell"])).unwrap();
        s.insert("deploy redis on staging", &steps(&["shell"])).unwrap();
        s.insert("check disk usage", &steps(&["disk"])).unwrap();

        let results = s.find_by_goal("deploy", 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn record_success_increments_count_happy_path() {
        let s = store();
        let id = s.insert("run tests", &steps(&["shell"])).unwrap();
        s.record_success(&id).unwrap();
        s.record_success(&id).unwrap();
        let p = s.get(&id).unwrap().unwrap();
        assert_eq!(p.success_count, 2);
        assert!(p.last_used.is_some());
    }

    #[test]
    fn list_ordered_by_success_happy_path() {
        let s = store();
        let id_a = s.insert("task a", &steps(&["shell"])).unwrap();
        let id_b = s.insert("task b", &steps(&["shell"])).unwrap();
        s.record_success(&id_b).unwrap();
        s.record_success(&id_b).unwrap();
        s.record_success(&id_a).unwrap();

        let list = s.list(10).unwrap();
        assert_eq!(list[0].id, id_b);
        assert_eq!(list[0].success_count, 2);
    }

    #[test]
    fn delete_removes_procedure_edge_case() {
        let s = store();
        let id = s.insert("ephemeral", &steps(&["shell"])).unwrap();
        assert_eq!(s.count().unwrap(), 1);
        s.delete(&id).unwrap();
        assert_eq!(s.count().unwrap(), 0);
        assert!(s.get(&id).unwrap().is_none());
    }

    #[test]
    fn find_by_goal_no_match_edge_case() {
        let s = store();
        s.insert("build docker image", &steps(&["shell"])).unwrap();
        let results = s.find_by_goal("kubernetes", 10).unwrap();
        assert!(results.is_empty());
    }
}
