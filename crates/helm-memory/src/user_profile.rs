//! User preference store — persists learned preferences across sessions.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum UserProfileError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("lock poisoned")]
    Lock,
}

// ── Domain types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Verbosity {
    Quiet,
    #[default]
    Normal,
    Verbose,
}

impl Verbosity {
    pub fn as_str(self) -> &'static str {
        match self {
            Verbosity::Quiet => "quiet",
            Verbosity::Normal => "normal",
            Verbosity::Verbose => "verbose",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "quiet" => Verbosity::Quiet,
            "verbose" => Verbosity::Verbose,
            _ => Verbosity::Normal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserPreferences {
    pub preferred_model: Option<String>,
    pub verbosity: Verbosity,
    pub timezone: Option<String>,
    pub correction_count: u32,
    pub last_goal: Option<String>,
}

// ── UserProfileStore ──────────────────────────────────────────────────────────

pub struct UserProfileStore {
    conn: Arc<Mutex<Connection>>,
}

impl UserProfileStore {
    pub fn open(path: &Path) -> Result<Self, UserProfileError> {
        let conn = Connection::open(path)?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> Result<Self, UserProfileError> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn get(&self) -> Result<UserPreferences, UserProfileError> {
        let conn = lock(&self.conn)?;
        let result = conn.query_row(
            "SELECT preferred_model, verbosity, timezone, correction_count, last_goal
             FROM user_profile WHERE id = 1",
            [],
            |row| {
                let verbosity_str: String = row.get(1)?;
                Ok(UserPreferences {
                    preferred_model: row.get(0)?,
                    verbosity: Verbosity::from_str(&verbosity_str),
                    timezone: row.get(2)?,
                    correction_count: row.get::<_, i64>(3)? as u32,
                    last_goal: row.get(4)?,
                })
            },
        );
        match result {
            Ok(p) => Ok(p),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(UserPreferences::default()),
            Err(e) => Err(UserProfileError::Sqlite(e)),
        }
    }

    pub fn set_preferred_model(&self, model: &str) -> Result<(), UserProfileError> {
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO user_profile (id, preferred_model) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET preferred_model = excluded.preferred_model",
            params![model],
        )?;
        Ok(())
    }

    pub fn set_verbosity(&self, v: Verbosity) -> Result<(), UserProfileError> {
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO user_profile (id, verbosity) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET verbosity = excluded.verbosity",
            params![v.as_str()],
        )?;
        Ok(())
    }

    pub fn set_timezone(&self, tz: &str) -> Result<(), UserProfileError> {
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO user_profile (id, timezone) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET timezone = excluded.timezone",
            params![tz],
        )?;
        Ok(())
    }

    pub fn record_correction(&self) -> Result<(), UserProfileError> {
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO user_profile (id, correction_count) VALUES (1, 1)
             ON CONFLICT(id) DO UPDATE SET correction_count = correction_count + 1",
            [],
        )?;
        Ok(())
    }

    pub fn record_goal(&self, goal: &str) -> Result<(), UserProfileError> {
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO user_profile (id, last_goal) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET last_goal = excluded.last_goal",
            params![goal],
        )?;
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn lock(conn: &Arc<Mutex<Connection>>) -> Result<MutexGuard<'_, Connection>, UserProfileError> {
    conn.lock().map_err(|_| UserProfileError::Lock)
}

fn run_migrations(conn: &Connection) -> Result<(), UserProfileError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS user_profile (
            id               INTEGER PRIMARY KEY CHECK (id = 1),
            preferred_model  TEXT,
            verbosity        TEXT NOT NULL DEFAULT 'normal',
            timezone         TEXT,
            correction_count INTEGER NOT NULL DEFAULT 0,
            last_goal        TEXT,
            updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
        );",
    )
    .map_err(UserProfileError::Sqlite)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{UserProfileStore, Verbosity};

    fn store() -> UserProfileStore {
        UserProfileStore::open_in_memory().unwrap()
    }

    #[test]
    fn get_returns_defaults_when_empty_edge_case() {
        let s = store();
        let p = s.get().unwrap();
        assert!(p.preferred_model.is_none());
        assert_eq!(p.verbosity, Verbosity::Normal);
        assert_eq!(p.correction_count, 0);
    }

    #[test]
    fn set_and_get_preferred_model_happy_path() {
        let s = store();
        s.set_preferred_model("groq/openai/gpt-oss-120b").unwrap();
        assert_eq!(
            s.get().unwrap().preferred_model.unwrap(),
            "groq/openai/gpt-oss-120b"
        );
    }

    #[test]
    fn set_verbosity_happy_path() {
        let s = store();
        s.set_verbosity(Verbosity::Verbose).unwrap();
        assert_eq!(s.get().unwrap().verbosity, Verbosity::Verbose);
    }

    #[test]
    fn record_correction_increments_happy_path() {
        let s = store();
        s.record_correction().unwrap();
        s.record_correction().unwrap();
        s.record_correction().unwrap();
        assert_eq!(s.get().unwrap().correction_count, 3);
    }

    #[test]
    fn record_goal_updates_last_goal_happy_path() {
        let s = store();
        s.record_goal("first goal").unwrap();
        s.record_goal("second goal").unwrap();
        assert_eq!(s.get().unwrap().last_goal.unwrap(), "second goal");
    }

    #[test]
    fn set_timezone_happy_path() {
        let s = store();
        s.set_timezone("America/New_York").unwrap();
        assert_eq!(s.get().unwrap().timezone.unwrap(), "America/New_York");
    }
}
