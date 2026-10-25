//! Snapshot persistence — store and retrieve typed system snapshots in SQLite.

use rusqlite::{Connection, OptionalExtension, params};

use helm_core::MemoryError;

/// Persisted snapshot row.
#[derive(Debug, Clone)]
pub struct SnapshotRecord {
    pub id: String,
    pub host_hostname: String,
    pub host_id: String,
    pub collected_at: i64,
    pub profile: String,
    pub domains_json: String,
    pub collector_errors_json: String,
    pub findings_json: String,
}

/// Store and retrieve system snapshots.
pub struct SnapshotStore;

impl SnapshotStore {
    /// Insert a snapshot into the database. `findings_json` is optional; pass "[]" if absent.
    pub fn insert(conn: &Connection, json: &str, findings_json: &str) -> Result<(), MemoryError> {
        // Persistence boundary: callers may or may not have redacted already (idempotent).
        let (json, findings_json) = (
            helm_core::redact_secrets(json),
            helm_core::redact_secrets(findings_json),
        );
        let (json, findings_json) = (json.as_str(), findings_json.as_str());
        let val: serde_json::Value =
            serde_json::from_str(json).map_err(|e| MemoryError::Other(e.to_string()))?;

        let id = val["id"].as_str().unwrap_or("").to_string();
        let host_hostname = val["host"]["hostname"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let host_id = val["host"]["id"].as_str().unwrap_or("").to_string();
        let collected_at = val["collected_at"].as_str().unwrap_or("");
        let collected_at_ts = chrono::DateTime::parse_from_rfc3339(collected_at)
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        let profile = val["profile"].as_str().unwrap_or("standard").to_string();
        let domains_json = serde_json::to_string(&val["domains"]).unwrap_or_default();
        let collector_errors_json =
            serde_json::to_string(&val["collector_errors"]).unwrap_or_default();

        conn.execute(
            "INSERT OR REPLACE INTO snapshots (id, host_hostname, host_id, collected_at, profile, domains_json, collector_errors_json, findings_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, host_hostname, host_id, collected_at_ts, profile, domains_json, collector_errors_json, findings_json],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;

        Ok(())
    }

    /// Get the most recent snapshot.
    pub fn latest(conn: &Connection) -> Result<Option<SnapshotRecord>, MemoryError> {
        let result = conn
            .query_row(
                "SELECT id, host_hostname, host_id, collected_at, profile, domains_json, collector_errors_json, findings_json FROM snapshots ORDER BY collected_at DESC LIMIT 1",
                [],
                |row| {
                    Ok(SnapshotRecord {
                        id: row.get(0)?,
                        host_hostname: row.get(1)?,
                        host_id: row.get(2)?,
                        collected_at: row.get(3)?,
                        profile: row.get(4)?,
                        domains_json: row.get(5)?,
                        collector_errors_json: row.get(6)?,
                        findings_json: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(result)
    }

    /// Get a snapshot by ID.
    pub fn get(conn: &Connection, id: &str) -> Result<Option<SnapshotRecord>, MemoryError> {
        let result = conn
            .query_row(
                "SELECT id, host_hostname, host_id, collected_at, profile, domains_json, collector_errors_json, findings_json FROM snapshots WHERE id = ?1",
                params![id],
                |row| {
                    Ok(SnapshotRecord {
                        id: row.get(0)?,
                        host_hostname: row.get(1)?,
                        host_id: row.get(2)?,
                        collected_at: row.get(3)?,
                        profile: row.get(4)?,
                        domains_json: row.get(5)?,
                        collector_errors_json: row.get(6)?,
                        findings_json: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(result)
    }

    /// List recent snapshots, newest first.
    pub fn list(conn: &Connection, limit: u32) -> Result<Vec<SnapshotRecord>, MemoryError> {
        let mut stmt = conn
            .prepare(
                "SELECT id, host_hostname, host_id, collected_at, profile, domains_json, collector_errors_json, findings_json FROM snapshots ORDER BY collected_at DESC LIMIT ?1",
            )
            .map_err(|e| MemoryError::Other(e.to_string()))?;

        let records = stmt
            .query_map(params![limit], |row| {
                Ok(SnapshotRecord {
                    id: row.get(0)?,
                    host_hostname: row.get(1)?,
                    host_id: row.get(2)?,
                    collected_at: row.get(3)?,
                    profile: row.get(4)?,
                    domains_json: row.get(5)?,
                    collector_errors_json: row.get(6)?,
                    findings_json: row.get(7)?,
                })
            })
            .map_err(|e| MemoryError::Other(e.to_string()))?;

        let mut result = Vec::new();
        for r in records {
            result.push(r.map_err(|e| MemoryError::Other(e.to_string()))?);
        }
        Ok(result)
    }

    /// Get the most recent snapshot, excluding the given ID (for diff).
    pub fn latest_except(
        conn: &Connection,
        except_id: &str,
    ) -> Result<Option<SnapshotRecord>, MemoryError> {
        let result = conn
            .query_row(
                "SELECT id, host_hostname, host_id, collected_at, profile, domains_json, collector_errors_json, findings_json FROM snapshots WHERE id != ?1 ORDER BY collected_at DESC LIMIT 1",
                params![except_id],
                |row| {
                    Ok(SnapshotRecord {
                        id: row.get(0)?,
                        host_hostname: row.get(1)?,
                        host_id: row.get(2)?,
                        collected_at: row.get(3)?,
                        profile: row.get(4)?,
                        domains_json: row.get(5)?,
                        collector_errors_json: row.get(6)?,
                        findings_json: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(result)
    }

    /// Delete snapshots older than `max_age_secs` and any beyond the newest `max_rows`, never
    /// touching the newest `MIN_KEPT`, or rows still referenced by changes (including their
    /// before/after/rollback evidence), troubleshooting plans or unresolved finding states.
    /// Returns the number of rows removed.
    pub fn prune(
        conn: &Connection,
        max_age_secs: i64,
        max_rows: u32,
    ) -> Result<usize, MemoryError> {
        const MIN_KEPT: u32 = 2;
        let table_exists = |name: &str| -> bool {
            conn.query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                params![name],
                |_| Ok(()),
            )
            .is_ok()
        };
        let mut protected = Vec::new();
        for (table, column, filter) in [
            ("change_sets", "snapshot_id", ""),
            ("change_sets", "before_snapshot_id", ""),
            ("change_sets", "after_snapshot_id", ""),
            ("change_sets", "rollback_snapshot_id", ""),
            ("troubleshooting_plans", "snapshot_id", ""),
            (
                "finding_states",
                "snapshot_id",
                " AND status NOT IN ('resolved','closed')",
            ),
        ] {
            if table_exists(table) {
                // NULLs would make NOT IN match nothing.
                protected.push(format!(
                    "SELECT {column} FROM {table} WHERE {column} IS NOT NULL{filter}"
                ));
            }
        }
        let protected = if protected.is_empty() {
            "SELECT ''".to_owned()
        } else {
            protected.join(" UNION ")
        };
        let cutoff = chrono::Utc::now().timestamp() - max_age_secs;
        let sql = format!(
            "DELETE FROM snapshots WHERE id NOT IN ({protected})
               AND id NOT IN (SELECT id FROM snapshots ORDER BY collected_at DESC LIMIT {MIN_KEPT})
               AND (collected_at < ?1
                    OR id NOT IN (SELECT id FROM snapshots ORDER BY collected_at DESC LIMIT ?2))"
        );
        conn.execute(&sql, params![cutoff, max_rows])
            .map_err(|e| MemoryError::Other(e.to_string()))
    }

    /// Delete a snapshot by ID.
    pub fn delete(conn: &Connection, id: &str) -> Result<(), MemoryError> {
        conn.execute("DELETE FROM snapshots WHERE id = ?1", params![id])
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_redacts_credentials_in_snapshot_and_findings() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE snapshots (id TEXT PRIMARY KEY, host_hostname TEXT, host_id TEXT,
                collected_at INTEGER NOT NULL, profile TEXT, domains_json TEXT,
                collector_errors_json TEXT, findings_json TEXT);",
        )
        .unwrap();
        let snapshot = r#"{"id":"s","host":{"hostname":"h"},"collected_at":"2026-01-01T00:00:00Z",
            "domains":{"logs":"login failed password=hunter2-sentinel from 10.0.0.1"},
            "collector_errors":[{"message":"postgres://app:hunter2-sentinel@db/x"}]}"#;
        let findings = r#"[{"evidence":"Authorization: Bearer hunter2sentinelvalue"}]"#;
        SnapshotStore::insert(&conn, snapshot, findings).unwrap();
        let row = SnapshotStore::get(&conn, "s").unwrap().unwrap();
        let stored = format!(
            "{} {} {}",
            row.domains_json, row.collector_errors_json, row.findings_json
        );
        assert!(!stored.contains("hunter2"), "{stored}");
        assert!(stored.contains("10.0.0.1"), "ordinary evidence survives");
    }

    #[test]
    fn test_host_id_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS snapshots (
                id TEXT PRIMARY KEY,
                host_hostname TEXT NOT NULL DEFAULT 'unknown',
                host_id TEXT DEFAULT '',
                collected_at INTEGER NOT NULL,
                profile TEXT NOT NULL DEFAULT 'standard',
                domains_json TEXT NOT NULL DEFAULT '{}',
                collector_errors_json TEXT NOT NULL DEFAULT '[]',
                findings_json TEXT NOT NULL DEFAULT '[]'
            )",
        )
        .unwrap();

        let snapshot_json = r#"{
            "id": "snap-123",
            "host": {
                "hostname": "test-host",
                "id": "uuid-12345"
            },
            "collected_at": "2026-05-18T10:00:00Z",
            "profile": "standard",
            "domains": {},
            "collector_errors": []
        }"#;

        SnapshotStore::insert(&conn, snapshot_json, "[]").unwrap();
        let retrieved = SnapshotStore::get(&conn, "snap-123")
            .unwrap()
            .expect("snapshot should exist");

        assert_eq!(retrieved.id, "snap-123");
        assert_eq!(retrieved.host_hostname, "test-host");
        assert_eq!(retrieved.host_id, "uuid-12345");
        assert_eq!(retrieved.profile, "standard");
    }

    #[test]
    fn test_multiple_hosts_snapshots() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS snapshots (
                id TEXT PRIMARY KEY,
                host_hostname TEXT NOT NULL DEFAULT 'unknown',
                host_id TEXT DEFAULT '',
                collected_at INTEGER NOT NULL,
                profile TEXT NOT NULL DEFAULT 'standard',
                domains_json TEXT NOT NULL DEFAULT '{}',
                collector_errors_json TEXT NOT NULL DEFAULT '[]',
                findings_json TEXT NOT NULL DEFAULT '[]'
            )",
        )
        .unwrap();

        let snap1 = r#"{
            "id": "snap-1",
            "host": {
                "hostname": "host-1",
                "id": "uuid-1"
            },
            "collected_at": "2026-05-18T10:00:00Z",
            "profile": "standard",
            "domains": {},
            "collector_errors": []
        }"#;

        let snap2 = r#"{
            "id": "snap-2",
            "host": {
                "hostname": "host-2",
                "id": "uuid-2"
            },
            "collected_at": "2026-05-18T10:01:00Z",
            "profile": "standard",
            "domains": {},
            "collector_errors": []
        }"#;

        SnapshotStore::insert(&conn, snap1, "[]").unwrap();
        SnapshotStore::insert(&conn, snap2, "[]").unwrap();

        let rec1 = SnapshotStore::get(&conn, "snap-1")
            .unwrap()
            .expect("snap-1 should exist");
        let rec2 = SnapshotStore::get(&conn, "snap-2")
            .unwrap()
            .expect("snap-2 should exist");

        assert_eq!(rec1.host_id, "uuid-1");
        assert_eq!(rec2.host_id, "uuid-2");

        let list = SnapshotStore::list(&conn, 10).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].host_id, "uuid-2"); // newest first
        assert_eq!(list[1].host_id, "uuid-1");
    }

    fn prune_fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE snapshots (id TEXT PRIMARY KEY, host_hostname TEXT, host_id TEXT,
                collected_at INTEGER NOT NULL, profile TEXT, domains_json TEXT,
                collector_errors_json TEXT, findings_json TEXT);
             CREATE TABLE change_sets (id TEXT, snapshot_id TEXT, before_snapshot_id TEXT,
                after_snapshot_id TEXT, rollback_snapshot_id TEXT);
             CREATE TABLE finding_states (fingerprint TEXT, status TEXT, snapshot_id TEXT);",
        )
        .unwrap();
        let now = chrono::Utc::now().timestamp();
        for n in 0..10i64 {
            // s0 is the oldest; each is one day apart.
            conn.execute(
                "INSERT INTO snapshots VALUES (?1,'h','i',?2,'quick','{}','[]','[]')",
                params![format!("s{n}"), now - (10 - n) * 86_400],
            )
            .unwrap();
        }
        conn
    }

    fn ids(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT id FROM snapshots ORDER BY collected_at")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn prune_removes_old_rows_but_protects_change_and_open_incident_evidence() {
        let conn = prune_fixture();
        conn.execute(
            "INSERT INTO change_sets VALUES ('c','s0','s1','s2',NULL)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO finding_states VALUES ('f','open','s3')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO finding_states VALUES ('g','resolved','s4')",
            [],
        )
        .unwrap();
        let removed = SnapshotStore::prune(&conn, 5 * 86_400 + 3_600, 1_000).unwrap();
        // Older than ~5 days: s0..s4. s0-s3 protected; s4 (resolved) goes.
        assert_eq!(removed, 1);
        assert_eq!(ids(&conn).len(), 9);
        assert!(!ids(&conn).contains(&"s4".to_owned()));
    }

    #[test]
    fn prune_enforces_row_cap_and_always_keeps_the_newest() {
        let conn = prune_fixture();
        SnapshotStore::prune(&conn, i64::MAX / 2, 3).unwrap();
        assert_eq!(ids(&conn), ["s7", "s8", "s9"]);
        SnapshotStore::prune(&conn, 0, 0).unwrap();
        assert_eq!(ids(&conn), ["s8", "s9"]);
    }

    #[test]
    fn prune_on_a_database_without_optional_tables_still_works() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE snapshots (id TEXT PRIMARY KEY, collected_at INTEGER NOT NULL);
             INSERT INTO snapshots VALUES ('a',1),('b',2),('c',3);",
        )
        .unwrap();
        assert_eq!(SnapshotStore::prune(&conn, 0, 10).unwrap(), 1);
    }
}
