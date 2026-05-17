use rusqlite::{Connection, OptionalExtension, params};

use helm_core::MemoryError;
use serde::Serialize;

/// Persisted change set row from the DB.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeSetRecord {
    pub id: String,
    pub plan_id: String,
    pub plan_title: String,
    pub snapshot_id: String,
    pub before_snapshot_id: String,
    pub after_snapshot_id: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub approved_at: Option<i64>,
    pub rejected_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub rolled_back_at: Option<i64>,
    pub rollback_snapshot_id: Option<String>,
    pub summary: String,
}

/// Persisted step row.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeSetStepRecord {
    pub id: String,
    pub change_set_id: String,
    pub plan_step_title: String,
    pub tool: String,
    pub input_json: String,
    pub command_text: Option<String>,
    pub expected_effect: String,
    pub risk: String,
    pub status: String,
    pub output_text: String,
    pub error_text: String,
    pub verification_result: String,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
}

/// Persisted backup row.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeSetBackupRecord {
    pub id: String,
    pub change_set_id: String,
    pub step_id: String,
    pub file_path: String,
    pub checksum_before: String,
    pub backup_content: String,
    pub restored: bool,
}

/// Full change set with all nested data.
#[derive(Debug, Clone, Serialize)]
pub struct FullChangeSet {
    pub record: ChangeSetRecord,
    pub steps: Vec<ChangeSetStepRecord>,
    pub backups: Vec<ChangeSetBackupRecord>,
}

/// Store and retrieve change sets.
pub struct ChangeSetStore;

impl ChangeSetStore {
    #[allow(clippy::too_many_arguments)]
    /// Insert a new change set with its steps.
    pub fn insert(
        conn: &Connection,
        id: &str,
        plan_id: &str,
        plan_title: &str,
        snapshot_id: &str,
        before_snapshot_id: &str,
        status: &str,
        created_at: i64,
        summary: &str,
    ) -> Result<(), MemoryError> {
        conn.execute(
            "INSERT INTO change_sets (id, plan_id, plan_title, snapshot_id, before_snapshot_id, after_snapshot_id, status, created_at, approved_at, rejected_at, completed_at, rolled_back_at, rollback_snapshot_id, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, NULL, NULL, NULL, NULL, NULL, ?8)",
            params![id, plan_id, plan_title, snapshot_id, before_snapshot_id, status, created_at, summary],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    /// Append a step to a change set.
    pub fn insert_step(
        conn: &Connection,
        id: &str,
        change_set_id: &str,
        plan_step_title: &str,
        tool: &str,
        input_json: &str,
        command_text: Option<&str>,
        expected_effect: &str,
        risk: &str,
        status: &str,
    ) -> Result<(), MemoryError> {
        conn.execute(
            "INSERT INTO change_set_steps (id, change_set_id, plan_step_title, tool, input_json, command_text, expected_effect, risk, status, output_text, error_text, verification_result, started_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '', '', '', NULL, NULL)",
            params![id, change_set_id, plan_step_title, tool, input_json, command_text, expected_effect, risk, status],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }

    /// Update change set status.
    pub fn update_status(conn: &Connection, id: &str, status: &str) -> Result<(), MemoryError> {
        let now = chrono::Utc::now().timestamp();
        let (approved, rejected, completed, rolled_back) = match status {
            "approved" => (Some(now), None, None, None),
            "rejected" => (None, Some(now), None, None),
            "completed" => (None, None, Some(now), None),
            "rolled_back" => (None, None, None, Some(now)),
            _ => (None::<i64>, None, None, None),
        };
        conn.execute(
            "UPDATE change_sets SET status = ?1, approved_at = COALESCE(?2, approved_at), rejected_at = COALESCE(?3, rejected_at), completed_at = COALESCE(?4, completed_at), rolled_back_at = COALESCE(?5, rolled_back_at) WHERE id = ?6",
            params![status, approved, rejected, completed, rolled_back, id],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }

    /// Update after_snapshot_id.
    pub fn update_after_snapshot(
        conn: &Connection,
        id: &str,
        after_snapshot_id: &str,
    ) -> Result<(), MemoryError> {
        conn.execute(
            "UPDATE change_sets SET after_snapshot_id = ?1 WHERE id = ?2",
            params![after_snapshot_id, id],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }

    /// Update a single step's outcome.
    pub fn update_step_outcome(
        conn: &Connection,
        step_id: &str,
        status: &str,
        output_text: &str,
        error_text: &str,
        verification_result: &str,
    ) -> Result<(), MemoryError> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE change_set_steps SET status = ?1, output_text = ?2, error_text = ?3, verification_result = ?4, completed_at = ?5 WHERE id = ?6",
            params![status, output_text, error_text, verification_result, now, step_id],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }

    /// Get a change set record.
    pub fn get(conn: &Connection, id: &str) -> Result<Option<ChangeSetRecord>, MemoryError> {
        let result = conn
            .query_row(
                "SELECT id, plan_id, plan_title, snapshot_id, before_snapshot_id, after_snapshot_id, status, created_at, approved_at, rejected_at, completed_at, rolled_back_at, rollback_snapshot_id, summary FROM change_sets WHERE id = ?1",
                params![id],
                |row| {
                    Ok(ChangeSetRecord {
                        id: row.get(0)?,
                        plan_id: row.get(1)?,
                        plan_title: row.get(2)?,
                        snapshot_id: row.get(3)?,
                        before_snapshot_id: row.get(4)?,
                        after_snapshot_id: row.get(5)?,
                        status: row.get(6)?,
                        created_at: row.get(7)?,
                        approved_at: row.get(8)?,
                        rejected_at: row.get(9)?,
                        completed_at: row.get(10)?,
                        rolled_back_at: row.get(11)?,
                        rollback_snapshot_id: row.get(12)?,
                        summary: row.get(13)?,
                    })
                },
            )
            .optional()
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(result)
    }

    /// Get steps for a change set.
    pub fn get_steps(
        conn: &Connection,
        change_set_id: &str,
    ) -> Result<Vec<ChangeSetStepRecord>, MemoryError> {
        let mut stmt = conn
            .prepare(
                "SELECT id, change_set_id, plan_step_title, tool, input_json, command_text, expected_effect, risk, status, output_text, error_text, verification_result, started_at, completed_at FROM change_set_steps WHERE change_set_id = ?1 ORDER BY rowid",
            )
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let records = stmt
            .query_map(params![change_set_id], |row| {
                Ok(ChangeSetStepRecord {
                    id: row.get(0)?,
                    change_set_id: row.get(1)?,
                    plan_step_title: row.get(2)?,
                    tool: row.get(3)?,
                    input_json: row.get(4)?,
                    command_text: row.get(5)?,
                    expected_effect: row.get(6)?,
                    risk: row.get(7)?,
                    status: row.get(8)?,
                    output_text: row.get(9)?,
                    error_text: row.get(10)?,
                    verification_result: row.get(11)?,
                    started_at: row.get(12)?,
                    completed_at: row.get(13)?,
                })
            })
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let mut result = Vec::new();
        for r in records {
            result.push(r.map_err(|e| MemoryError::Other(e.to_string()))?);
        }
        Ok(result)
    }

    /// Get backups for a change set.
    pub fn get_backups(
        conn: &Connection,
        change_set_id: &str,
    ) -> Result<Vec<ChangeSetBackupRecord>, MemoryError> {
        let mut stmt = conn
            .prepare(
                "SELECT id, change_set_id, step_id, file_path, checksum_before, backup_content, restored FROM change_set_backups WHERE change_set_id = ?1",
            )
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let records = stmt
            .query_map(params![change_set_id], |row| {
                Ok(ChangeSetBackupRecord {
                    id: row.get(0)?,
                    change_set_id: row.get(1)?,
                    step_id: row.get(2)?,
                    file_path: row.get(3)?,
                    checksum_before: row.get(4)?,
                    backup_content: row.get(5)?,
                    restored: row.get(6)?,
                })
            })
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let mut result = Vec::new();
        for r in records {
            result.push(r.map_err(|e| MemoryError::Other(e.to_string()))?);
        }
        Ok(result)
    }

    /// Get a full change set with steps and backups.
    pub fn get_full(conn: &Connection, id: &str) -> Result<Option<FullChangeSet>, MemoryError> {
        let record = match Self::get(conn, id)? {
            Some(r) => r,
            None => return Ok(None),
        };
        let steps = Self::get_steps(conn, id)?;
        let backups = Self::get_backups(conn, id)?;
        Ok(Some(FullChangeSet {
            record,
            steps,
            backups,
        }))
    }

    /// List change sets, newest first.
    pub fn list(conn: &Connection, limit: u32) -> Result<Vec<ChangeSetRecord>, MemoryError> {
        let mut stmt = conn
            .prepare(
                "SELECT id, plan_id, plan_title, snapshot_id, before_snapshot_id, after_snapshot_id, status, created_at, approved_at, rejected_at, completed_at, rolled_back_at, rollback_snapshot_id, summary FROM change_sets ORDER BY created_at DESC LIMIT ?1",
            )
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let records = stmt
            .query_map(params![limit], |row| {
                Ok(ChangeSetRecord {
                    id: row.get(0)?,
                    plan_id: row.get(1)?,
                    plan_title: row.get(2)?,
                    snapshot_id: row.get(3)?,
                    before_snapshot_id: row.get(4)?,
                    after_snapshot_id: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                    approved_at: row.get(8)?,
                    rejected_at: row.get(9)?,
                    completed_at: row.get(10)?,
                    rolled_back_at: row.get(11)?,
                    rollback_snapshot_id: row.get(12)?,
                    summary: row.get(13)?,
                })
            })
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let mut result = Vec::new();
        for r in records {
            result.push(r.map_err(|e| MemoryError::Other(e.to_string()))?);
        }
        Ok(result)
    }
}

/// Persisted troubleshooting plan row.
#[derive(Debug, Clone, Serialize)]
pub struct TroubleshootingPlanRecord {
    pub id: String,
    pub source: String,
    pub snapshot_id: String,
    pub finding_id: String,
    pub hypotheses_json: String,
    pub read_only_steps_json: String,
    pub proposed_fix_steps_json: String,
    pub approval_required: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub verdict_summary: String,
    pub narrative_summary: String,
    pub dashboard_plan_status: String,
    pub generation_error: String,
    pub verification_steps_json: String,
    pub reproduction_steps_json: String,
}

/// Store and retrieve troubleshooting plans.
pub struct TroubleshootingPlanStore;

impl TroubleshootingPlanStore {
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        conn: &Connection,
        id: &str,
        source: &str,
        snapshot_id: &str,
        hypotheses_json: &str,
        read_only_steps_json: &str,
        proposed_fix_steps_json: &str,
        approval_required: bool,
        verdict_summary: &str,
    ) -> Result<(), MemoryError> {
        let now = chrono::Utc::now().timestamp();
        let finding_id = source
            .strip_prefix("finding:")
            .map(str::trim)
            .unwrap_or_default();
        let dashboard_plan_status = if proposed_fix_steps_json.trim() == "[]" {
            "pending"
        } else {
            "ready"
        };
        conn.execute(
            "INSERT INTO troubleshooting_plans (
                id, source, snapshot_id, finding_id, hypotheses_json, read_only_steps_json,
                proposed_fix_steps_json, approval_required, created_at, updated_at,
                verdict_summary, narrative_summary, dashboard_plan_status, generation_error,
                verification_steps_json, reproduction_steps_json
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, '', '[]', '[]')",
            params![
                id,
                source,
                snapshot_id,
                finding_id,
                hypotheses_json,
                read_only_steps_json,
                proposed_fix_steps_json,
                approval_required,
                now,
                now,
                verdict_summary,
                verdict_summary,
                dashboard_plan_status,
            ],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_dashboard_plan(
        conn: &Connection,
        id: &str,
        finding_id: &str,
        read_only_steps_json: &str,
        proposed_fix_steps_json: &str,
        narrative_summary: &str,
        dashboard_plan_status: &str,
        generation_error: &str,
        verification_steps_json: &str,
        reproduction_steps_json: &str,
    ) -> Result<(), MemoryError> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE troubleshooting_plans
             SET finding_id = ?2,
                 read_only_steps_json = ?3,
                 proposed_fix_steps_json = ?4,
                 verdict_summary = ?5,
                 narrative_summary = ?5,
                 dashboard_plan_status = ?6,
                 generation_error = ?7,
                 verification_steps_json = ?8,
                 reproduction_steps_json = ?9,
                 updated_at = ?10
             WHERE id = ?1",
            params![
                id,
                finding_id,
                read_only_steps_json,
                proposed_fix_steps_json,
                narrative_summary,
                dashboard_plan_status,
                generation_error,
                verification_steps_json,
                reproduction_steps_json,
                now,
            ],
        )
        .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(())
    }

    pub fn get(
        conn: &Connection,
        id: &str,
    ) -> Result<Option<TroubleshootingPlanRecord>, MemoryError> {
        let result = conn
            .query_row(
                "SELECT id, source, snapshot_id, finding_id, hypotheses_json, read_only_steps_json,
                        proposed_fix_steps_json, approval_required, created_at, updated_at,
                        verdict_summary, narrative_summary, dashboard_plan_status,
                        generation_error, verification_steps_json, reproduction_steps_json
                 FROM troubleshooting_plans
                 WHERE id = ?1",
                params![id],
                |row| {
                    Ok(TroubleshootingPlanRecord {
                        id: row.get(0)?,
                        source: row.get(1)?,
                        snapshot_id: row.get(2)?,
                        finding_id: row.get(3)?,
                        hypotheses_json: row.get(4)?,
                        read_only_steps_json: row.get(5)?,
                        proposed_fix_steps_json: row.get(6)?,
                        approval_required: row.get::<_, i64>(7)? != 0,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                        verdict_summary: row.get(10)?,
                        narrative_summary: row.get(11)?,
                        dashboard_plan_status: row.get(12)?,
                        generation_error: row.get(13)?,
                        verification_steps_json: row.get(14)?,
                        reproduction_steps_json: row.get(15)?,
                    })
                },
            )
            .optional()
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(result)
    }

    pub fn latest_for_finding(
        conn: &Connection,
        finding_id: &str,
    ) -> Result<Option<TroubleshootingPlanRecord>, MemoryError> {
        let result = conn
            .query_row(
                "SELECT id, source, snapshot_id, finding_id, hypotheses_json, read_only_steps_json,
                        proposed_fix_steps_json, approval_required, created_at, updated_at,
                        verdict_summary, narrative_summary, dashboard_plan_status,
                        generation_error, verification_steps_json, reproduction_steps_json
                 FROM troubleshooting_plans
                 WHERE finding_id = ?1
                 ORDER BY updated_at DESC, created_at DESC
                 LIMIT 1",
                params![finding_id],
                |row| {
                    Ok(TroubleshootingPlanRecord {
                        id: row.get(0)?,
                        source: row.get(1)?,
                        snapshot_id: row.get(2)?,
                        finding_id: row.get(3)?,
                        hypotheses_json: row.get(4)?,
                        read_only_steps_json: row.get(5)?,
                        proposed_fix_steps_json: row.get(6)?,
                        approval_required: row.get::<_, i64>(7)? != 0,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                        verdict_summary: row.get(10)?,
                        narrative_summary: row.get(11)?,
                        dashboard_plan_status: row.get(12)?,
                        generation_error: row.get(13)?,
                        verification_steps_json: row.get(14)?,
                        reproduction_steps_json: row.get(15)?,
                    })
                },
            )
            .optional()
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        Ok(result)
    }

    pub fn list(
        conn: &Connection,
        limit: u32,
    ) -> Result<Vec<TroubleshootingPlanRecord>, MemoryError> {
        let mut stmt = conn
            .prepare(
                "SELECT id, source, snapshot_id, finding_id, hypotheses_json, read_only_steps_json,
                        proposed_fix_steps_json, approval_required, created_at, updated_at,
                        verdict_summary, narrative_summary, dashboard_plan_status,
                        generation_error, verification_steps_json, reproduction_steps_json
                 FROM troubleshooting_plans
                 ORDER BY updated_at DESC, created_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let records = stmt
            .query_map(params![limit], |row| {
                Ok(TroubleshootingPlanRecord {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    snapshot_id: row.get(2)?,
                    finding_id: row.get(3)?,
                    hypotheses_json: row.get(4)?,
                    read_only_steps_json: row.get(5)?,
                    proposed_fix_steps_json: row.get(6)?,
                    approval_required: row.get::<_, i64>(7)? != 0,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    verdict_summary: row.get(10)?,
                    narrative_summary: row.get(11)?,
                    dashboard_plan_status: row.get(12)?,
                    generation_error: row.get(13)?,
                    verification_steps_json: row.get(14)?,
                    reproduction_steps_json: row.get(15)?,
                })
            })
            .map_err(|e| MemoryError::Other(e.to_string()))?;
        let mut result = Vec::new();
        for r in records {
            result.push(r.map_err(|e| MemoryError::Other(e.to_string()))?);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../migrations/0008_changesets.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0010_dashboard_plans.sql"))
            .unwrap();
        conn
    }

    #[test]
    fn insert_and_get_change_set() {
        let conn = setup_db();
        ChangeSetStore::insert(
            &conn,
            "cs-1",
            "plan-1",
            "test plan",
            "snap-1",
            "before-1",
            "pending",
            100_000,
            "summary text",
        )
        .unwrap();
        let record = ChangeSetStore::get(&conn, "cs-1").unwrap().unwrap();
        assert_eq!(record.id, "cs-1");
        assert_eq!(record.plan_id, "plan-1");
        assert_eq!(record.status, "pending");
    }

    #[test]
    fn insert_and_list_steps() {
        let conn = setup_db();
        ChangeSetStore::insert(
            &conn, "cs-2", "plan-2", "", "snap-2", "before-2", "pending", 200_000, "",
        )
        .unwrap();
        ChangeSetStore::insert_step(
            &conn,
            "step-1",
            "cs-2",
            "check disk",
            "shell",
            "{}",
            Some("df -h"),
            "check usage",
            "none",
            "pending",
        )
        .unwrap();
        ChangeSetStore::insert_step(
            &conn,
            "step-2",
            "cs-2",
            "clean cache",
            "shell",
            "{}",
            Some("apt clean"),
            "clean apt",
            "low",
            "pending",
        )
        .unwrap();
        let steps = ChangeSetStore::get_steps(&conn, "cs-2").unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].plan_step_title, "check disk");
        assert_eq!(steps[1].plan_step_title, "clean cache");
    }

    #[test]
    fn update_status() {
        let conn = setup_db();
        ChangeSetStore::insert(
            &conn, "cs-3", "plan-3", "", "snap-3", "before-3", "pending", 300_000, "",
        )
        .unwrap();
        ChangeSetStore::update_status(&conn, "cs-3", "approved").unwrap();
        let record = ChangeSetStore::get(&conn, "cs-3").unwrap().unwrap();
        assert_eq!(record.status, "approved");
        assert!(record.approved_at.is_some());
    }

    #[test]
    fn update_step_outcome() {
        let conn = setup_db();
        ChangeSetStore::insert(
            &conn, "cs-4", "plan-4", "", "snap-4", "before-4", "pending", 400_000, "",
        )
        .unwrap();
        ChangeSetStore::insert_step(
            &conn,
            "step-3",
            "cs-4",
            "fix",
            "shell",
            "{}",
            Some("echo ok"),
            "fix it",
            "low",
            "pending",
        )
        .unwrap();
        ChangeSetStore::update_step_outcome(
            &conn,
            "step-3",
            "succeeded",
            "output here",
            "",
            "verified",
        )
        .unwrap();
        let steps = ChangeSetStore::get_steps(&conn, "cs-4").unwrap();
        assert_eq!(steps[0].status, "succeeded");
        assert_eq!(steps[0].output_text, "output here");
    }

    #[test]
    fn get_full_nonexistent() {
        let conn = setup_db();
        let full = ChangeSetStore::get_full(&conn, "nonexistent").unwrap();
        assert!(full.is_none());
    }

    #[test]
    fn list_ordered_by_created_at_desc() {
        let conn = setup_db();
        ChangeSetStore::insert(&conn, "cs-a", "p-a", "", "s-a", "b-a", "completed", 100, "")
            .unwrap();
        ChangeSetStore::insert(&conn, "cs-b", "p-b", "", "s-b", "b-b", "completed", 200, "")
            .unwrap();
        ChangeSetStore::insert(&conn, "cs-c", "p-c", "", "s-c", "b-c", "completed", 300, "")
            .unwrap();
        let list = ChangeSetStore::list(&conn, 10).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, "cs-c");
        assert_eq!(list[1].id, "cs-b");
        assert_eq!(list[2].id, "cs-a");
    }

    #[test]
    fn dashboard_plan_metadata_roundtrips() {
        let conn = setup_db();
        TroubleshootingPlanStore::insert(
            &conn,
            "plan-meta",
            "finding: finding-1",
            "snap-1",
            "[]",
            "[]",
            "[]",
            true,
            "initial",
        )
        .unwrap();

        TroubleshootingPlanStore::update_dashboard_plan(
            &conn,
            "plan-meta",
            "finding-1",
            r#"["journalctl -u nginx -n 50"]"#,
            r#"[{"title":"restart service","command":{"tool":"shell","command_text":"systemctl restart nginx","expected_effect":"restart nginx","risk":"medium"}}]"#,
            "Nginx failed after a bad config reload.",
            "ready",
            "",
            r#"["systemctl is-active nginx"]"#,
            r#"["journalctl -u nginx -n 50"]"#,
        )
        .unwrap();

        let record = TroubleshootingPlanStore::latest_for_finding(&conn, "finding-1")
            .unwrap()
            .unwrap();
        assert_eq!(record.id, "plan-meta");
        assert_eq!(record.dashboard_plan_status, "ready");
        assert_eq!(
            record.narrative_summary,
            "Nginx failed after a bad config reload."
        );
        assert_eq!(
            record.verification_steps_json,
            r#"["systemctl is-active nginx"]"#
        );
    }
}
