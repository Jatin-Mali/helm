//! Persistent finding lifecycle state for the triage dashboard.

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use helm_core::MemoryError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingStateStatus {
    Open,
    Suppressed,
    Resolved,
}

impl FindingStateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Suppressed => "suppressed",
            Self::Resolved => "resolved",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "suppressed" => Self::Suppressed,
            "resolved" => Self::Resolved,
            _ => Self::Open,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingStateRecord {
    pub fingerprint: String,
    pub status: FindingStateStatus,
    pub suppression_reason: String,
    pub note: String,
    pub updated_at: i64,
    pub snapshot_id: String,
    pub finding_id: String,
}

pub struct FindingStateStore;

impl FindingStateStore {
    pub fn set_status(
        conn: &Connection,
        fingerprint: &str,
        status: FindingStateStatus,
        suppression_reason: &str,
        note: &str,
        snapshot_id: &str,
        finding_id: &str,
    ) -> Result<(), MemoryError> {
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO finding_states (
                fingerprint, status, suppression_reason, note, updated_at, snapshot_id, finding_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(fingerprint) DO UPDATE SET
                status = excluded.status,
                suppression_reason = excluded.suppression_reason,
                note = excluded.note,
                updated_at = excluded.updated_at,
                snapshot_id = excluded.snapshot_id,
                finding_id = excluded.finding_id",
            params![
                fingerprint,
                status.as_str(),
                suppression_reason,
                note,
                now,
                snapshot_id,
                finding_id,
            ],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }

    pub fn clear(conn: &Connection, fingerprint: &str) -> Result<(), MemoryError> {
        conn.execute(
            "DELETE FROM finding_states WHERE fingerprint = ?1",
            params![fingerprint],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }

    pub fn get(
        conn: &Connection,
        fingerprint: &str,
    ) -> Result<Option<FindingStateRecord>, MemoryError> {
        conn.query_row(
            "SELECT fingerprint, status, suppression_reason, note, updated_at, snapshot_id, finding_id
             FROM finding_states
             WHERE fingerprint = ?1",
            params![fingerprint],
            |row| {
                Ok(FindingStateRecord {
                    fingerprint: row.get(0)?,
                    status: FindingStateStatus::parse(&row.get::<_, String>(1)?),
                    suppression_reason: row.get(2)?,
                    note: row.get(3)?,
                    updated_at: row.get(4)?,
                    snapshot_id: row.get(5)?,
                    finding_id: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|e| MemoryError::Other(e.to_string()))
    }

    pub fn list(conn: &Connection) -> Result<Vec<FindingStateRecord>, MemoryError> {
        let mut stmt = conn
            .prepare(
                "SELECT fingerprint, status, suppression_reason, note, updated_at, snapshot_id, finding_id
                 FROM finding_states
                 ORDER BY updated_at DESC",
            )
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(FindingStateRecord {
                    fingerprint: row.get(0)?,
                    status: FindingStateStatus::parse(&row.get::<_, String>(1)?),
                    suppression_reason: row.get(2)?,
                    note: row.get(3)?,
                    updated_at: row.get(4)?,
                    snapshot_id: row.get(5)?,
                    finding_id: row.get(6)?,
                })
            })
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| MemoryError::Other(e.to_string()))?);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_clear_state_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../migrations/0009_finding_state.sql"))
            .unwrap();

        FindingStateStore::set_status(
            &conn,
            "fp-1",
            FindingStateStatus::Suppressed,
            "known noise",
            "temporary",
            "snap-1",
            "finding-1",
        )
        .unwrap();

        let record = FindingStateStore::get(&conn, "fp-1").unwrap().unwrap();
        assert_eq!(record.status, FindingStateStatus::Suppressed);
        assert_eq!(record.suppression_reason, "known noise");

        FindingStateStore::clear(&conn, "fp-1").unwrap();
        assert!(FindingStateStore::get(&conn, "fp-1").unwrap().is_none());
    }
}
