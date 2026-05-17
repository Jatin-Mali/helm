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
}
