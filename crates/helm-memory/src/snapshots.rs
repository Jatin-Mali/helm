//! Snapshot persistence — store and retrieve typed system snapshots in SQLite.

use rusqlite::{Connection, OptionalExtension, params};

use helm_core::MemoryError;

/// Persisted snapshot row.
#[derive(Debug, Clone)]
pub struct SnapshotRecord {
    pub id: String,
    pub host_hostname: String,
    pub collected_at: i64,
    pub profile: String,
    pub domains_json: String,
    pub collector_errors_json: String,
}

/// Store and retrieve system snapshots.
pub struct SnapshotStore;

impl SnapshotStore {
    /// Insert a snapshot into the database.
    pub fn insert(conn: &Connection, json: &str) -> Result<(), MemoryError> {
        let val: serde_json::Value =
            serde_json::from_str(json).map_err(|e| MemoryError::Other(e.to_string()))?;

        let id = val["id"].as_str().unwrap_or("").to_string();
        let host_hostname = val["host"]["hostname"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let collected_at = val["collected_at"].as_str().unwrap_or("");
        let collected_at_ts = chrono::DateTime::parse_from_rfc3339(collected_at)
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        let profile = val["profile"].as_str().unwrap_or("standard").to_string();
        let domains_json = serde_json::to_string(&val["domains"]).unwrap_or_default();
        let collector_errors_json =
            serde_json::to_string(&val["collector_errors"]).unwrap_or_default();

        conn.execute(
            "INSERT OR REPLACE INTO snapshots (id, host_hostname, collected_at, profile, domains_json, collector_errors_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, host_hostname, collected_at_ts, profile, domains_json, collector_errors_json],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;

        Ok(())
    }

    /// Get the most recent snapshot.
    pub fn latest(conn: &Connection) -> Result<Option<SnapshotRecord>, MemoryError> {
        let result = conn
            .query_row(
                "SELECT id, host_hostname, collected_at, profile, domains_json, collector_errors_json FROM snapshots ORDER BY collected_at DESC LIMIT 1",
                [],
                |row| {
                    Ok(SnapshotRecord {
                        id: row.get(0)?,
                        host_hostname: row.get(1)?,
                        collected_at: row.get(2)?,
                        profile: row.get(3)?,
                        domains_json: row.get(4)?,
                        collector_errors_json: row.get(5)?,
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
                "SELECT id, host_hostname, collected_at, profile, domains_json, collector_errors_json FROM snapshots WHERE id = ?1",
                params![id],
                |row| {
                    Ok(SnapshotRecord {
                        id: row.get(0)?,
                        host_hostname: row.get(1)?,
                        collected_at: row.get(2)?,
                        profile: row.get(3)?,
                        domains_json: row.get(4)?,
                        collector_errors_json: row.get(5)?,
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
                "SELECT id, host_hostname, collected_at, profile, domains_json, collector_errors_json FROM snapshots ORDER BY collected_at DESC LIMIT ?1",
            )
            .map_err(|e| MemoryError::Other(e.to_string()))?;

        let records = stmt
            .query_map(params![limit], |row| {
                Ok(SnapshotRecord {
                    id: row.get(0)?,
                    host_hostname: row.get(1)?,
                    collected_at: row.get(2)?,
                    profile: row.get(3)?,
                    domains_json: row.get(4)?,
                    collector_errors_json: row.get(5)?,
                })
            })
            .map_err(|e| MemoryError::Other(e.to_string()))?;

        let mut result = Vec::new();
        for r in records {
            result.push(r.map_err(|e| MemoryError::Other(e.to_string()))?);
        }
        Ok(result)
    }

    /// Delete a snapshot by ID.
    pub fn delete(conn: &Connection, id: &str) -> Result<(), MemoryError> {
        conn.execute("DELETE FROM snapshots WHERE id = ?1", params![id])
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }
}
