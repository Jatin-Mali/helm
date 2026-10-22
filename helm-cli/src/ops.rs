use std::{collections::BTreeSet, path::Path};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use helm_memory::{
    ChangeSetStepRecord, ChangeSetStore, FullChangeSet, SnapshotStore, TroubleshootingPlanRecord,
    TroubleshootingPlanStore,
};
use helm_monitor::{Finding, SnapshotDomains, SystemSnapshot, SystemdTimer};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSetStepView {
    pub title: String,
    pub status: String,
    pub tool: String,
    pub command: String,
    pub expected_effect: String,
    pub risk: String,
    pub stdout: String,
    pub stderr: String,
    pub verification: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSetBackupView {
    pub file_path: String,
    pub checksum_before: String,
    pub restored: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSetView {
    pub id: String,
    pub plan_id: String,
    pub plan_title: String,
    pub status: String,
    pub summary: String,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub rolled_back_at: Option<String>,
    pub affected_resources: Vec<String>,
    pub trigger_finding_id: Option<String>,
    pub trigger_finding_fingerprint: Option<String>,
    pub trigger_finding_title: Option<String>,
    pub trigger_affected_resource: Option<String>,
    pub steps: Vec<ChangeSetStepView>,
    pub backups: Vec<ChangeSetBackupView>,
    pub diff_lines: Vec<String>,
    pub rollback_supported: bool,
    pub rollback_summary: String,
}

pub async fn read_service_journal_excerpt(unit: &str, lines: usize) -> Vec<String> {
    let unit = unit.trim();
    if unit.is_empty() {
        return vec!["No service selected.".to_owned()];
    }
    let output = helm_monitor::process_runner::run_program(
        "journalctl",
        &[
            "-u",
            unit,
            "-n",
            &lines.clamp(10, 200).to_string(),
            "--no-pager",
            "-o",
            "short-iso",
        ],
        std::time::Duration::from_secs(5),
    )
    .await;
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let rows = stdout
                .lines()
                .map(str::trim_end)
                .filter(|line| !line.is_empty())
                .map(helm_core::redact_secrets)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                vec![format!("No recent journal output for {unit}.")]
            } else {
                rows
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            vec![format!(
                "journalctl -u {unit} failed: {}",
                if stderr.is_empty() {
                    "unknown error".to_owned()
                } else {
                    stderr
                }
            )]
        }
        Err(error) => vec![format!("journalctl unavailable for {unit}: {error}")],
    }
}

pub fn timers_for_service<'a>(timers: &'a [SystemdTimer], unit: &str) -> Vec<&'a SystemdTimer> {
    let service_unit = if unit.ends_with(".service") {
        unit.to_owned()
    } else {
        format!("{unit}.service")
    };
    timers
        .iter()
        .filter(|timer| timer.activates == service_unit || timer.unit == unit || timer.name == unit)
        .collect()
}

pub fn timer_overdue(timer: &SystemdTimer) -> bool {
    let next = timer.next_trigger.to_ascii_lowercase();
    next == "n/a" || next == "-" || next.contains("elapsed")
}

pub fn load_change_set_views(conn: &rusqlite::Connection, limit: u32) -> Vec<ChangeSetView> {
    let Ok(records) = ChangeSetStore::list(conn, limit) else {
        return Vec::new();
    };
    records
        .into_iter()
        .filter_map(|record| load_change_set_view(conn, &record.id).ok().flatten())
        .collect()
}

pub fn load_change_set_view(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<Option<ChangeSetView>> {
    let Some(full) = ChangeSetStore::get_full(conn, id).map_err(|e| anyhow!("{e}"))? else {
        return Ok(None);
    };
    Ok(Some(build_change_set_view(conn, &full)))
}

pub async fn rollback_change_set(
    db_path: &Path,
    id: &str,
    policy: helm_monitor::ExecutionPolicy,
    secrets: &crate::secrets::SecretsStore,
) -> Result<Vec<String>> {
    if !policy.check_mutation().map_err(anyhow::Error::msg)? {
        return Ok(vec![format!(
            "Dry run: would restore change set {id}; no actions executed."
        )]);
    }
    let _lock = crate::execution_journal::lock_database(db_path)?;
    let conn = rusqlite::Connection::open(db_path)?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    let full =
        ChangeSetStore::get_full(&conn, id)?.ok_or_else(|| anyhow!("change set not found"))?;
    if full.record.status == "rolled_back" {
        return Ok(vec!["Already rolled back; no actions executed.".into()]);
    }
    let mut results = Vec::new();
    let mut failures = Vec::new();
    let mut recoveries = Vec::new();
    for step in full.steps.iter().rev() {
        let backups = full
            .backups
            .iter()
            .filter(|backup| backup.step_id == step.id)
            .collect::<Vec<_>>();
        if backups.is_empty()
            && matches!(
                step.status.as_str(),
                "succeeded" | "running" | "interrupted" | "failed"
            )
        {
            failures.push(format!(
                "{}: no verified inverse or backup; manual recovery required",
                step.plan_step_title
            ));
        }
        for backup in backups {
            if backup.restored {
                continue;
            }
            match crate::execution_journal::open_backup(secrets, backup) {
                Ok(body) => recoveries.push((backup, body)),
                Err(error) => {
                    ChangeSetStore::update_status(&conn, id, "rollback_failed")?;
                    return Err(error.context(
                        "recovery material could not be authenticated; no files restored",
                    ));
                }
            }
        }
    }
    if recoveries.is_empty() && (full.backups.is_empty() || !failures.is_empty()) {
        ChangeSetStore::update_status(&conn, id, "rollback_failed")?;
        return Err(anyhow!(
            "no verified file recovery data; {}",
            failures.join("; ")
        ));
    }
    ChangeSetStore::update_status(&conn, id, "rolling_back")?;
    for (backup, body) in recoveries {
        match helm_monitor::file_change::restore_file_change(
            &backup.file_path,
            &backup.checksum_before,
            &body,
        ) {
            Ok(()) => {
                conn.execute(
                    "UPDATE change_set_backups SET restored=1 WHERE id=?1",
                    rusqlite::params![backup.id],
                )?;
                results.push(format!("restored and verified {}", backup.file_path));
            }
            Err(error) => failures.push(format!("{}: {error}", backup.file_path)),
        }
    }
    if !failures.is_empty() {
        ChangeSetStore::update_status(&conn, id, "rollback_failed")?;
        return Err(anyhow!(
            "restoration incomplete: {}\n{}",
            failures.join("; "),
            results.join("\n")
        ));
    }
    let snapshot = helm_monitor::collect_snapshot(helm_monitor::MonitorProfile::Quick).await;
    let encoded = helm_core::redact_secrets(&serde_json::to_string(&snapshot)?);
    SnapshotStore::insert(&conn, &encoded, "[]")?;
    conn.execute("UPDATE change_sets SET status='rolled_back', rolled_back_at=?2, rollback_snapshot_id=?3 WHERE id=?1",
        rusqlite::params![id, Utc::now().timestamp(), snapshot.id])?;
    Ok(results)
}

fn build_change_set_view(conn: &rusqlite::Connection, full: &FullChangeSet) -> ChangeSetView {
    let plan = TroubleshootingPlanStore::get(conn, &full.record.plan_id)
        .ok()
        .flatten();
    let trigger_finding = plan
        .as_ref()
        .and_then(|plan| trigger_finding_for_plan(conn, plan));
    let diff_lines = snapshot_diff_lines(
        conn,
        &full.record.before_snapshot_id,
        full.record.after_snapshot_id.as_deref(),
    );
    let affected_resources = affected_resources(full, trigger_finding.as_ref());
    let backups = full
        .backups
        .iter()
        .map(|backup| ChangeSetBackupView {
            file_path: backup.file_path.clone(),
            checksum_before: backup.checksum_before.clone(),
            restored: backup.restored,
        })
        .collect::<Vec<_>>();
    let steps = full
        .steps
        .iter()
        .map(step_view_from_record)
        .collect::<Vec<_>>();
    let rollback_supported = full
        .backups
        .iter()
        .any(crate::execution_journal::has_verified_backup);
    let rollback_summary = if rollback_supported {
        "Encrypted file restoration requires the recovery key and matching applied state. Unbacked commands require manual recovery.".to_owned()
    } else {
        "No verified automatic recovery data is stored for this change set.".to_owned()
    };

    ChangeSetView {
        id: full.record.id.clone(),
        plan_id: full.record.plan_id.clone(),
        plan_title: full.record.plan_title.clone(),
        status: full.record.status.clone(),
        summary: full.record.summary.clone(),
        created_at: format_timestamp(full.record.created_at),
        completed_at: full.record.completed_at.map(format_timestamp),
        rolled_back_at: full.record.rolled_back_at.map(format_timestamp),
        affected_resources,
        trigger_finding_id: plan
            .as_ref()
            .and_then(|record| non_empty(&record.finding_id)),
        trigger_finding_fingerprint: plan
            .as_ref()
            .and_then(|record| non_empty(&record.finding_fingerprint)),
        trigger_finding_title: trigger_finding
            .as_ref()
            .map(|finding| finding.title.clone()),
        trigger_affected_resource: trigger_finding
            .as_ref()
            .map(|finding| finding.affected_resource.clone()),
        steps,
        backups,
        diff_lines,
        rollback_supported,
        rollback_summary,
    }
}

fn step_view_from_record(step: &ChangeSetStepRecord) -> ChangeSetStepView {
    ChangeSetStepView {
        title: step.plan_step_title.clone(),
        status: step.status.clone(),
        tool: step.tool.clone(),
        command: step
            .command_text
            .clone()
            .unwrap_or_else(|| step.input_json.clone()),
        expected_effect: step.expected_effect.clone(),
        risk: step.risk.clone(),
        stdout: helm_core::redact_secrets(&step.output_text),
        stderr: helm_core::redact_secrets(&step.error_text),
        verification: helm_core::redact_secrets(&step.verification_result),
        started_at: step.started_at.map(format_timestamp),
        completed_at: step.completed_at.map(format_timestamp),
    }
}

fn snapshot_diff_lines(
    conn: &rusqlite::Connection,
    before_id: &str,
    after_id: Option<&str>,
) -> Vec<String> {
    let Some(before) = load_snapshot(conn, before_id) else {
        return vec!["Pre-change snapshot unavailable.".to_owned()];
    };
    let Some(after_id) = after_id else {
        return vec!["Post-change snapshot has not been recorded yet.".to_owned()];
    };
    let Some(after) = load_snapshot(conn, after_id) else {
        return vec!["Post-change snapshot unavailable.".to_owned()];
    };

    let before_failed = before
        .domains
        .services
        .units
        .iter()
        .filter(|unit| unit.active == "failed" || unit.sub == "failed")
        .count() as isize;
    let after_failed = after
        .domains
        .services
        .units
        .iter()
        .filter(|unit| unit.active == "failed" || unit.sub == "failed")
        .count() as isize;
    let before_containers = before
        .domains
        .containers
        .containers
        .iter()
        .filter(|container| container.status == "running")
        .count() as isize;
    let after_containers = after
        .domains
        .containers
        .containers
        .iter()
        .filter(|container| container.status == "running")
        .count() as isize;
    let before_ports = before.domains.ports.listeners.len() as isize;
    let after_ports = after.domains.ports.listeners.len() as isize;
    let before_findings = serde_json::from_str::<Vec<Finding>>(
        &snapshot_record_findings(conn, before_id).unwrap_or_default(),
    )
    .unwrap_or_default()
    .len() as isize;
    let after_findings = serde_json::from_str::<Vec<Finding>>(
        &snapshot_record_findings(conn, after_id).unwrap_or_default(),
    )
    .unwrap_or_default()
    .len() as isize;
    vec![
        format!(
            "Services failed: {} → {} ({:+})",
            before_failed,
            after_failed,
            after_failed - before_failed
        ),
        format!(
            "Running containers: {} → {} ({:+})",
            before_containers,
            after_containers,
            after_containers - before_containers
        ),
        format!(
            "Listening ports: {} → {} ({:+})",
            before_ports,
            after_ports,
            after_ports - before_ports
        ),
        format!(
            "Findings in snapshot: {} → {} ({:+})",
            before_findings,
            after_findings,
            after_findings - before_findings
        ),
    ]
}

fn load_snapshot(conn: &rusqlite::Connection, id: &str) -> Option<SystemSnapshot> {
    let record = SnapshotStore::get(conn, id).ok().flatten()?;
    let domains = serde_json::from_str::<SnapshotDomains>(&record.domains_json).ok()?;
    Some(crate::reconstruct_snapshot(&record, domains))
}

fn snapshot_record_findings(conn: &rusqlite::Connection, id: &str) -> Option<String> {
    SnapshotStore::get(conn, id)
        .ok()
        .flatten()
        .map(|record| record.findings_json)
}

fn trigger_finding_for_plan(
    conn: &rusqlite::Connection,
    plan: &TroubleshootingPlanRecord,
) -> Option<Finding> {
    if !plan.finding_id.trim().is_empty() {
        return crate::find_finding_by_id(conn, &plan.finding_id);
    }
    if plan.finding_fingerprint.trim().is_empty() {
        return None;
    }
    let records = SnapshotStore::list(conn, 200).ok()?;
    for record in records {
        let findings = serde_json::from_str::<Vec<Finding>>(&record.findings_json).ok()?;
        if let Some(finding) = findings
            .into_iter()
            .find(|finding| finding.fingerprint == plan.finding_fingerprint)
        {
            return Some(finding);
        }
    }
    None
}

fn affected_resources(full: &FullChangeSet, finding: Option<&Finding>) -> Vec<String> {
    let mut resources = BTreeSet::new();
    for backup in &full.backups {
        resources.insert(backup.file_path.clone());
    }
    for step in &full.steps {
        collect_step_resources(step, &mut resources);
    }
    if let Some(finding) = finding
        && !finding.affected_resource.trim().is_empty()
    {
        resources.insert(finding.affected_resource.clone());
    }
    resources.into_iter().take(8).collect()
}

fn collect_step_resources(step: &ChangeSetStepRecord, resources: &mut BTreeSet<String>) {
    if let Some(command) = &step.command_text {
        for token in command.split_whitespace() {
            let token =
                token.trim_matches(|ch: char| ch == '"' || ch == '\'' || ch == ',' || ch == ';');
            if token.ends_with(".service") || token.ends_with(".timer") || token.contains('/') {
                resources.insert(token.to_owned());
            }
        }
        if command.contains("systemctl ") {
            resources.insert(step.plan_step_title.clone());
        }
    }
}

fn format_timestamp(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recovery_refuses_unbacked_shell_changes() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("helm.db");
        let _store = helm_memory::MemoryStore::open(&db).await.unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        ChangeSetStore::insert(
            &conn,
            "unbacked",
            "plan",
            "fixture",
            "snapshot",
            "before",
            "completed",
            1,
            "",
        )
        .unwrap();
        ChangeSetStore::insert_step(
            &conn,
            "step",
            "unbacked",
            "unbacked shell",
            "shell",
            "{}",
            Some("printf harmless"),
            "fixture",
            "low",
            "succeeded",
        )
        .unwrap();
        let result = rollback_change_set(
            &db,
            "unbacked",
            Default::default(),
            &crate::secrets::SecretsStore::open_at(dir.path().join("secrets.toml")).unwrap(),
        )
        .await;
        assert!(
            result.is_err(),
            "unsupported shell reversal was reported as successful"
        );
        assert_ne!(
            ChangeSetStore::get(&conn, "unbacked")
                .unwrap()
                .unwrap()
                .status,
            "rolled_back"
        );
    }

    #[tokio::test]
    async fn rollback_modes_do_not_open_a_database_or_change_files() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("never-created.db");
        let error = rollback_change_set(
            &db,
            "fixture",
            helm_monitor::ExecutionPolicy {
                read_only: true,
                ..Default::default()
            },
            &crate::secrets::SecretsStore::open_at(dir.path().join("secrets.toml")).unwrap(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("read-only"));
        let preview = rollback_change_set(
            &db,
            "fixture",
            helm_monitor::ExecutionPolicy {
                dry_run: true,
                ..Default::default()
            },
            &crate::secrets::SecretsStore::open_at(dir.path().join("secrets.toml")).unwrap(),
        )
        .await
        .unwrap();
        assert!(preview[0].contains("Dry run"));
        assert!(!db.exists());
    }

    #[test]
    fn service_timer_matching_handles_unit_and_activates_edges() {
        let timers = vec![
            SystemdTimer {
                name: "apt-daily.timer".into(),
                next_trigger: "n/a".into(),
                last_trigger: "2026-01-01 00:00".into(),
                passed: "1h".into(),
                unit: "apt-daily.timer".into(),
                activates: "apt-daily.service".into(),
            },
            SystemdTimer {
                name: "fstrim.timer".into(),
                next_trigger: "2026-01-02 00:00".into(),
                last_trigger: "2026-01-01 00:00".into(),
                passed: "1d".into(),
                unit: "fstrim.timer".into(),
                activates: "fstrim.service".into(),
            },
        ];
        let matched = timers_for_service(&timers, "apt-daily.service");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "apt-daily.timer");
        assert!(timer_overdue(matched[0]));
    }
}
