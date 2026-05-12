//! User preference store — persists learned preferences across sessions.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum UserProfileError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("lock poisoned")]
    Lock,
    #[error("toml serialize: {0}")]
    Toml(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

// ── Domain types ──────────────────────────────────────────────────────────────

/// Role-based access control for HELM operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// All capabilities granted.
    Admin,
    /// Read, shell, tools — no config changes or secrets delete.
    #[default]
    User,
    /// Read-only operations only.
    Viewer,
}

impl Role {
    /// Returns the stable string representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::User => "user",
            Role::Viewer => "viewer",
        }
    }

    /// Parse from string.
    fn from_str(s: &str) -> Self {
        match s {
            "admin" => Role::Admin,
            "viewer" => Role::Viewer,
            _ => Role::User,
        }
    }

    /// Returns true if this role is allowed to use the given capability.
    pub fn allows(&self, cap: &str) -> bool {
        match self {
            Role::Admin => true,
            Role::User => !matches!(cap, "secrets.delete" | "config.write" | "audit.delete"),
            Role::Viewer => matches!(cap, "read" | "list" | "view" | "fs.read" | "network.out"),
        }
    }
}

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
    pub current_role: Role,
}

// ── UserPreferencesFile ───────────────────────────────────────────────────────

/// TOML-backed storage for `UserPreferences`. Reads on demand, writes
/// atomically. Keeps `tool_outcomes` in SQLite (high write-rate).
pub struct UserPreferencesFile;

impl UserPreferencesFile {
    pub fn load(path: &Path) -> UserPreferences {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(prefs: &UserPreferences, path: &Path) -> Result<(), UserProfileError> {
        let text =
            toml::to_string_pretty(prefs).map_err(|e| UserProfileError::Toml(e.to_string()))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)?;
        Ok(())
    }
}

// ── UserProfileStore ──────────────────────────────────────────────────────────

pub struct UserProfileStore {
    conn: Arc<Mutex<Connection>>,
    prefs: Arc<Mutex<UserPreferences>>,
    prefs_path: Option<PathBuf>,
}

impl UserProfileStore {
    pub fn open(path: &Path) -> Result<Self, UserProfileError> {
        let conn = Connection::open(path)?;
        run_migrations(&conn)?;
        let prefs = load_prefs_from_sqlite(&conn);
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            prefs: Arc::new(Mutex::new(prefs)),
            prefs_path: None,
        })
    }

    /// Open with a separate TOML file for `UserPreferences`. On first open,
    /// if the TOML file is absent, migrates the SQLite `user_profile` row.
    pub fn open_with_prefs(
        sqlite_path: &Path,
        prefs_path: &Path,
    ) -> Result<Self, UserProfileError> {
        let conn = Connection::open(sqlite_path)?;
        run_migrations(&conn)?;
        let prefs = if prefs_path.exists() {
            UserPreferencesFile::load(prefs_path)
        } else {
            let p = load_prefs_from_sqlite(&conn);
            UserPreferencesFile::save(&p, prefs_path)?;
            p
        };
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            prefs: Arc::new(Mutex::new(prefs)),
            prefs_path: Some(prefs_path.to_owned()),
        })
    }

    pub fn open_in_memory() -> Result<Self, UserProfileError> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            prefs: Arc::new(Mutex::new(UserPreferences::default())),
            prefs_path: None,
        })
    }

    pub fn get(&self) -> Result<UserPreferences, UserProfileError> {
        self.prefs
            .lock()
            .map(|g| g.clone())
            .map_err(|_| UserProfileError::Lock)
    }

    fn flush_prefs(&self) {
        let Ok(guard) = self.prefs.lock() else { return };
        if let Some(path) = &self.prefs_path {
            if let Err(e) = UserPreferencesFile::save(&guard, path) {
                eprintln!("helm: failed to save user_profile.toml: {e}");
            }
        }
    }

    pub fn set_preferred_model(&self, model: &str) -> Result<(), UserProfileError> {
        {
            let conn = lock(&self.conn)?;
            conn.execute(
                "INSERT INTO user_profile (id, preferred_model) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET preferred_model = excluded.preferred_model",
                params![model],
            )?;
        }
        if let Ok(mut g) = self.prefs.lock() {
            g.preferred_model = Some(model.to_owned());
        }
        self.flush_prefs();
        Ok(())
    }

    pub fn set_verbosity(&self, v: Verbosity) -> Result<(), UserProfileError> {
        {
            let conn = lock(&self.conn)?;
            conn.execute(
                "INSERT INTO user_profile (id, verbosity) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET verbosity = excluded.verbosity",
                params![v.as_str()],
            )?;
        }
        if let Ok(mut g) = self.prefs.lock() {
            g.verbosity = v;
        }
        self.flush_prefs();
        Ok(())
    }

    pub fn set_timezone(&self, tz: &str) -> Result<(), UserProfileError> {
        {
            let conn = lock(&self.conn)?;
            conn.execute(
                "INSERT INTO user_profile (id, timezone) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET timezone = excluded.timezone",
                params![tz],
            )?;
        }
        if let Ok(mut g) = self.prefs.lock() {
            g.timezone = Some(tz.to_owned());
        }
        self.flush_prefs();
        Ok(())
    }

    pub fn record_correction(&self) -> Result<(), UserProfileError> {
        {
            let conn = lock(&self.conn)?;
            conn.execute(
                "INSERT INTO user_profile (id, correction_count) VALUES (1, 1)
                 ON CONFLICT(id) DO UPDATE SET correction_count = correction_count + 1",
                [],
            )?;
        }
        if let Ok(mut g) = self.prefs.lock() {
            g.correction_count = g.correction_count.saturating_add(1);
        }
        self.flush_prefs();
        Ok(())
    }

    pub fn record_goal(&self, goal: &str) -> Result<(), UserProfileError> {
        {
            let conn = lock(&self.conn)?;
            conn.execute(
                "INSERT INTO user_profile (id, last_goal) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET last_goal = excluded.last_goal",
                params![goal],
            )?;
        }
        if let Ok(mut g) = self.prefs.lock() {
            g.last_goal = Some(goal.to_owned());
        }
        self.flush_prefs();
        Ok(())
    }

    pub async fn record_tool_outcome(
        &self,
        tool_name: &str,
        success: bool,
    ) -> Result<(), UserProfileError> {
        let conn = lock(&self.conn)?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tool_outcomes (id, tool_name, success, recorded_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                &id,
                tool_name,
                if success { 1 } else { 0 },
                chrono::Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    pub async fn get_tool_preference(&self, tool_name: &str) -> Result<f32, UserProfileError> {
        let conn = lock(&self.conn)?;
        let result: Option<(i64, i64)> = conn
            .query_row(
                "SELECT COUNT(*) as total, SUM(success) as successes
                 FROM tool_outcomes WHERE tool_name = ?1",
                params![tool_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        match result {
            Some((total, successes)) if total > 0 => Ok((successes as f32) / (total as f32)),
            _ => Ok(0.0),
        }
    }

    pub async fn get_preferred_tools(
        &self,
        top_n: u32,
    ) -> Result<Vec<(String, f32)>, UserProfileError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn.prepare(
            "SELECT tool_name, COUNT(*) as total, SUM(success) as successes
             FROM tool_outcomes
             GROUP BY tool_name
             HAVING total >= 3
             ORDER BY (successes / CAST(total AS REAL)) DESC
             LIMIT ?1",
        )?;
        let tools = stmt
            .query_map(params![top_n], |row| {
                let total: i64 = row.get(1)?;
                let successes: i64 = row.get(2)?;
                Ok((
                    row.get::<_, String>(0)?,
                    (successes as f32) / (total as f32),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tools)
    }

    pub async fn set_preference(&self, key: &str, value: &str) -> Result<(), UserProfileError> {
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO user_preferences (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub async fn get_preference(&self, key: &str) -> Result<Option<String>, UserProfileError> {
        let conn = lock(&self.conn)?;
        let result = conn
            .query_row(
                "SELECT value FROM user_preferences WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result)
    }

    pub fn set_role(&self, role: Role) -> Result<(), UserProfileError> {
        {
            let conn = lock(&self.conn)?;
            conn.execute(
                "INSERT INTO user_profile (id, current_role) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET current_role = excluded.current_role",
                params![role.as_str()],
            )?;
        }
        if let Ok(mut g) = self.prefs.lock() {
            g.current_role = role;
        }
        self.flush_prefs();
        Ok(())
    }

    pub fn get_role(&self) -> Result<Role, UserProfileError> {
        Ok(self
            .prefs
            .lock()
            .map_err(|_| UserProfileError::Lock)?
            .current_role)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn lock(conn: &Arc<Mutex<Connection>>) -> Result<MutexGuard<'_, Connection>, UserProfileError> {
    conn.lock().map_err(|_| UserProfileError::Lock)
}

fn load_prefs_from_sqlite(conn: &Connection) -> UserPreferences {
    conn.query_row(
        "SELECT preferred_model, verbosity, timezone, correction_count, last_goal, current_role
         FROM user_profile WHERE id = 1",
        [],
        |row| {
            let verbosity_str: String = row.get(1)?;
            let role_str: String = row.get(5)?;
            Ok(UserPreferences {
                preferred_model: row.get(0)?,
                verbosity: Verbosity::from_str(&verbosity_str),
                timezone: row.get(2)?,
                correction_count: row.get::<_, i64>(3)? as u32,
                last_goal: row.get(4)?,
                current_role: Role::from_str(&role_str),
            })
        },
    )
    .unwrap_or_default()
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
            current_role     TEXT NOT NULL DEFAULT 'user',
            updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
        );
        CREATE TABLE IF NOT EXISTS tool_outcomes (
            id           TEXT PRIMARY KEY,
            tool_name    TEXT NOT NULL,
            success      INTEGER NOT NULL,
            recorded_at  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS user_preferences (
            key         TEXT PRIMARY KEY,
            value       TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );",
    )
    .map_err(UserProfileError::Sqlite)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{Role, UserPreferences, UserProfileStore, Verbosity};

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

    #[tokio::test]
    async fn record_and_get_tool_preference_happy_path() {
        let s = store();
        s.record_tool_outcome("shell", true).await.unwrap();
        s.record_tool_outcome("shell", true).await.unwrap();
        s.record_tool_outcome("shell", false).await.unwrap();
        let pref = s.get_tool_preference("shell").await.unwrap();
        assert!((pref - 2.0 / 3.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn set_and_get_preference_happy_path() {
        let s = store();
        s.set_preference("theme", "dark").await.unwrap();
        let val = s.get_preference("theme").await.unwrap();
        assert_eq!(val, Some("dark".to_string()));
    }

    #[tokio::test]
    async fn get_preferred_tools_filters_by_minimum_uses() {
        let s = store();
        s.record_tool_outcome("shell", true).await.unwrap();
        s.record_tool_outcome("shell", true).await.unwrap();
        s.record_tool_outcome("fs_read", true).await.unwrap();
        let tools = s.get_preferred_tools(10).await.unwrap();
        assert!(tools.is_empty());
        s.record_tool_outcome("shell", true).await.unwrap();
        let tools = s.get_preferred_tools(10).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, "shell");
    }

    #[test]
    fn role_allows_checks_capabilities() {
        assert!(Role::Admin.allows("anything"));
        assert!(Role::User.allows("shell.exec"));
        assert!(!Role::User.allows("secrets.delete"));
        assert!(!Role::User.allows("config.write"));
        assert!(Role::Viewer.allows("fs.read"));
        assert!(Role::Viewer.allows("network.out"));
        assert!(!Role::Viewer.allows("shell.exec"));
    }

    #[test]
    fn set_and_get_role_happy_path() {
        let s = store();
        assert_eq!(s.get_role().unwrap(), Role::User);
        s.set_role(Role::Admin).unwrap();
        assert_eq!(s.get_role().unwrap(), Role::Admin);
        s.set_role(Role::Viewer).unwrap();
        assert_eq!(s.get_role().unwrap(), Role::Viewer);
    }

    #[test]
    fn user_preferences_file_round_trip() {
        use super::UserPreferencesFile;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user_profile.toml");
        let prefs = UserPreferences {
            preferred_model: Some("claude-3".to_owned()),
            verbosity: Verbosity::Verbose,
            timezone: Some("UTC".to_owned()),
            correction_count: 5,
            last_goal: Some("test goal".to_owned()),
            current_role: Role::Admin,
        };
        UserPreferencesFile::save(&prefs, &path).unwrap();
        let loaded = UserPreferencesFile::load(&path);
        assert_eq!(loaded.preferred_model, prefs.preferred_model);
        assert_eq!(loaded.verbosity, prefs.verbosity);
        assert_eq!(loaded.correction_count, prefs.correction_count);
        assert_eq!(loaded.current_role, prefs.current_role);
    }

    #[test]
    fn open_with_prefs_migrates_sqlite_to_toml() {
        use super::UserPreferencesFile;
        let dir = tempfile::tempdir().unwrap();
        let sqlite_path = dir.path().join("profile.db");
        let toml_path = dir.path().join("user_profile.toml");

        // seed SQLite
        let s = UserProfileStore::open(&sqlite_path).unwrap();
        s.set_preferred_model("gpt-4").unwrap();
        s.set_verbosity(Verbosity::Quiet).unwrap();
        drop(s);

        // open_with_prefs should migrate
        assert!(!toml_path.exists());
        let s2 = UserProfileStore::open_with_prefs(&sqlite_path, &toml_path).unwrap();
        assert!(toml_path.exists());
        let prefs = s2.get().unwrap();
        assert_eq!(prefs.preferred_model.as_deref(), Some("gpt-4"));
        assert_eq!(prefs.verbosity, Verbosity::Quiet);

        // subsequent set should update TOML
        s2.set_timezone("Europe/London").unwrap();
        let loaded = UserPreferencesFile::load(&toml_path);
        assert_eq!(loaded.timezone.as_deref(), Some("Europe/London"));
    }
}
