//! Plan cache — stores previously computed plans keyed by goal hash.
//!
//! Before the ReactAgent starts planning, it checks the cache for an exact
//! match on the normalized goal string.  On hit, the cached step sequence
//! is returned directly, saving provider round-trips.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum PlanCacheError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("lock poisoned")]
    Lock,
}

// ── Domain types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPlan {
    pub id: String,
    pub goal_hash: String,
    pub goal_text: String,
    pub steps: Vec<PlanStep>,
    pub hit_count: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub tool: String,
    pub description: String,
    pub input_template: serde_json::Value,
}

// ── PlanCache ─────────────────────────────────────────────────────────────────

pub struct PlanCache {
    conn: Arc<Mutex<Connection>>,
}

impl PlanCache {
    pub fn open(path: &Path) -> Result<Self, PlanCacheError> {
        let conn = Connection::open(path)?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> Result<Self, PlanCacheError> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Look up a plan by exact goal text (normalized to lowercase trimmed).
    pub fn get(&self, goal: &str) -> Result<Option<CachedPlan>, PlanCacheError> {
        let hash = goal_hash(goal);
        let conn = lock(&self.conn)?;
        let plan = conn
            .query_row(
                "SELECT id, goal_hash, goal_text, steps, hit_count, created_at
                 FROM plan_cache WHERE goal_hash = ?1",
                params![hash],
                row_to_plan,
            )
            .optional()
            .map_err(PlanCacheError::Sqlite)?;

        if let Some(ref p) = plan {
            conn.execute(
                "UPDATE plan_cache SET hit_count = hit_count + 1 WHERE id = ?1",
                params![p.id],
            )?;
        }
        Ok(plan)
    }

    /// Store a plan.  Replaces any existing entry for the same goal hash.
    pub fn put(&self, goal: &str, steps: &[PlanStep]) -> Result<String, PlanCacheError> {
        let id = uuid_v4();
        let hash = goal_hash(goal);
        let steps_json = serde_json::to_string(steps)?;
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO plan_cache (id, goal_hash, goal_text, steps, hit_count)
             VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(goal_hash) DO UPDATE
             SET id=excluded.id, goal_text=excluded.goal_text,
                 steps=excluded.steps, hit_count=0,
                 created_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')",
            params![id, hash, goal.trim(), steps_json],
        )?;
        Ok(id)
    }

    pub fn invalidate(&self, goal: &str) -> Result<(), PlanCacheError> {
        let hash = goal_hash(goal);
        let conn = lock(&self.conn)?;
        conn.execute("DELETE FROM plan_cache WHERE goal_hash = ?1", params![hash])?;
        Ok(())
    }

    pub fn count(&self) -> Result<u64, PlanCacheError> {
        let conn = lock(&self.conn)?;
        conn.query_row("SELECT COUNT(*) FROM plan_cache", [], |row| row.get::<_, i64>(0))
            .map(|n| n as u64)
            .map_err(PlanCacheError::Sqlite)
    }

    /// Returns all cached plans ordered by hit_count desc.
    pub fn list(&self, limit: u32) -> Result<Vec<CachedPlan>, PlanCacheError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn.prepare(
            "SELECT id, goal_hash, goal_text, steps, hit_count, created_at
             FROM plan_cache ORDER BY hit_count DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], row_to_plan)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(PlanCacheError::Sqlite)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Normalize goal text and return a stable hex hash.
pub fn goal_hash(goal: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let normalized = goal.trim().to_lowercase();
    let mut h = DefaultHasher::new();
    normalized.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn row_to_plan(row: &rusqlite::Row<'_>) -> rusqlite::Result<CachedPlan> {
    let steps_str: String = row.get(3)?;
    let steps: Vec<PlanStep> = serde_json::from_str(&steps_str).unwrap_or_default();
    Ok(CachedPlan {
        id: row.get(0)?,
        goal_hash: row.get(1)?,
        goal_text: row.get(2)?,
        steps,
        hit_count: row.get::<_, i64>(4)? as u32,
        created_at: row.get(5)?,
    })
}

fn lock(conn: &Arc<Mutex<Connection>>) -> Result<MutexGuard<'_, Connection>, PlanCacheError> {
    conn.lock().map_err(|_| PlanCacheError::Lock)
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn run_migrations(conn: &Connection) -> Result<(), PlanCacheError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS plan_cache (
            id          TEXT PRIMARY KEY,
            goal_hash   TEXT NOT NULL UNIQUE,
            goal_text   TEXT NOT NULL,
            steps       TEXT NOT NULL DEFAULT '[]',
            hit_count   INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
        );
        CREATE INDEX IF NOT EXISTS idx_plan_cache_hash ON plan_cache(goal_hash);
        CREATE INDEX IF NOT EXISTS idx_plan_cache_hits ON plan_cache(hit_count DESC);",
    )
    .map_err(PlanCacheError::Sqlite)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{PlanCache, PlanStep, goal_hash};

    fn cache() -> PlanCache {
        PlanCache::open_in_memory().unwrap()
    }

    fn steps(tools: &[&str]) -> Vec<PlanStep> {
        tools
            .iter()
            .map(|t| PlanStep {
                tool: t.to_string(),
                description: format!("run {t}"),
                input_template: json!({}),
            })
            .collect()
    }

    #[test]
    fn put_and_get_happy_path() {
        let c = cache();
        c.put("deploy nginx", &steps(&["shell", "service"])).unwrap();
        let plan = c.get("deploy nginx").unwrap().unwrap();
        assert_eq!(plan.goal_text, "deploy nginx");
        assert_eq!(plan.steps.len(), 2);
    }

    #[test]
    fn get_increments_hit_count_happy_path() {
        let c = cache();
        c.put("run tests", &steps(&["shell"])).unwrap();
        c.get("run tests").unwrap();
        c.get("run tests").unwrap();
        let plan = c.get("run tests").unwrap().unwrap();
        assert_eq!(plan.hit_count, 2);
    }

    #[test]
    fn get_miss_returns_none_edge_case() {
        let c = cache();
        assert!(c.get("nonexistent goal").unwrap().is_none());
    }

    #[test]
    fn goal_hash_is_case_insensitive_edge_case() {
        assert_eq!(goal_hash("Deploy Nginx"), goal_hash("deploy nginx"));
        assert_eq!(goal_hash("  deploy  "), goal_hash("deploy"));
    }

    #[test]
    fn put_replaces_existing_happy_path() {
        let c = cache();
        c.put("upgrade db", &steps(&["shell"])).unwrap();
        c.put("upgrade db", &steps(&["shell", "service"])).unwrap();
        assert_eq!(c.count().unwrap(), 1);
        let plan = c.get("upgrade db").unwrap().unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.hit_count, 0);
    }

    #[test]
    fn invalidate_removes_entry_edge_case() {
        let c = cache();
        c.put("temp goal", &steps(&["shell"])).unwrap();
        c.invalidate("temp goal").unwrap();
        assert!(c.get("temp goal").unwrap().is_none());
        assert_eq!(c.count().unwrap(), 0);
    }

    #[test]
    fn list_ordered_by_hits_happy_path() {
        let c = cache();
        c.put("goal a", &steps(&["shell"])).unwrap();
        c.put("goal b", &steps(&["shell"])).unwrap();
        c.get("goal b").unwrap();
        c.get("goal b").unwrap();

        let list = c.list(10).unwrap();
        assert_eq!(list[0].goal_text, "goal b");
    }
}
