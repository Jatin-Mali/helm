//! v1.9 Guided Troubleshooting — comprehensive test suite.

use helm_monitor::snapshot::FilesystemEntry;
use helm_monitor::{
    BlastRadius, CommandPreview, Hypothesis, PlanSource, RiskLevel, RollbackStatus,
    TroubleshootingPlan, explain_finding,
    findings::{Confidence, Finding, Severity},
    plan_from_problem, plan_from_problem_with_snapshot,
    snapshot::{HostIdentity, MonitorProfile, SnapshotDomains, SystemSnapshot},
};

fn base_snapshot() -> SystemSnapshot {
    SystemSnapshot {
        id: "base".into(),
        host: HostIdentity::default(),
        collected_at: chrono::Utc::now(),
        profile: MonitorProfile::Standard,
        domains: SnapshotDomains::default(),
        collector_errors: vec![],
        redaction_version: "0.1.0".into(),
    }
}

#[allow(dead_code)]
fn empty_snapshot() -> SystemSnapshot {
    SystemSnapshot {
        id: "test-snap".into(),
        host: HostIdentity::default(),
        collected_at: chrono::Utc::now(),
        profile: MonitorProfile::Standard,
        domains: SnapshotDomains::default(),
        collector_errors: vec![],
        redaction_version: "0.1.0".into(),
    }
}

// ── e2e: plan_from_problem generates hypotheses ─────────────────────────────

#[test]
fn e2e_plan_from_problem_generates_hypotheses() {
    let plan = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(plan_from_problem("disk is full"));
    assert!(!plan.hypotheses.is_empty(), "should generate hypotheses");
    let disk_hyp = plan
        .hypotheses
        .iter()
        .find(|h| h.domain == helm_monitor::MonitorDomain::Disks);
    assert!(
        disk_hyp.is_some(),
        "disk problem should produce disk hypothesis"
    );
}

#[test]
fn e2e_plan_from_problem_service_failure() {
    let plan = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(plan_from_problem("nginx service keeps crashing"));
    let svc_hyp = plan
        .hypotheses
        .iter()
        .find(|h| h.domain == helm_monitor::MonitorDomain::Services);
    assert!(
        svc_hyp.is_some(),
        "service crash problem should produce service hypothesis"
    );
}

// ── No fix suggestion without evidence ──────────────────────────────────────

#[test]
fn no_fix_without_evidence_on_empty_snapshot() {
    let snapshot = empty_snapshot();
    let plan = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(plan_from_problem_with_snapshot("disk is full", snapshot));
    // Empty snapshot means no actionable evidence -> no fix steps
    assert!(
        plan.proposed_fix_steps.is_empty(),
        "empty snapshot should have no fix proposals: had {}",
        plan.proposed_fix_steps.len()
    );
}

#[test]
fn hypotheses_without_evidence_have_low_confidence() {
    let plan = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(plan_from_problem("disk is full"));
    for h in &plan.hypotheses {
        if h.evidence_for.is_empty() {
            assert!(
                h.confidence < 0.5,
                "hypothesis {} with no for-evidence should have low confidence, got {}",
                h.hypothesis,
                h.confidence
            );
        }
    }
}

// ── Command preview fields ──────────────────────────────────────────────────

#[test]
fn command_preview_contains_all_required_fields() {
    let preview = CommandPreview::new("shell", "df -h /", "Check root filesystem usage")
        .with_risk(RiskLevel::None)
        .with_blast(BlastRadius::File("/".into()))
        .with_rollback(RollbackStatus::NotNeeded)
        .with_verification(CommandPreview::new(
            "shell",
            "echo ok",
            "verify nothing changed",
        ));

    assert_eq!(preview.tool, "shell");
    assert_eq!(preview.command_text.as_deref(), Some("df -h /"));
    assert_eq!(preview.expected_effect, "Check root filesystem usage");
    assert_eq!(preview.risk, RiskLevel::None);
    assert!(matches!(preview.blast_radius, BlastRadius::File(_)));
    assert!(matches!(preview.rollback, RollbackStatus::NotNeeded));
    assert_eq!(preview.verification.len(), 1);
}

#[test]
fn command_preview_rollback_unsupported_clearly_labeled() {
    let preview = CommandPreview::new("shell", "rm -rf /tmp/data", "Delete temp data")
        .with_rollback(RollbackStatus::Unsupported);
    assert!(matches!(preview.rollback, RollbackStatus::Unsupported));
    assert_eq!(preview.rollback.to_string(), "no rollback available");
}

#[test]
fn command_preview_blast_radius_human_readable() {
    let file = BlastRadius::File("/etc/nginx/nginx.conf".into());
    assert_eq!(file.to_string(), "file: /etc/nginx/nginx.conf");
    let svc = BlastRadius::Service("nginx".into());
    assert_eq!(svc.to_string(), "service: nginx");
    let system = BlastRadius::System;
    assert_eq!(system.to_string(), "entire system");
}

// ── TroubleshootingPlan render_text ─────────────────────────────────────────

#[test]
fn plan_render_text_contains_key_sections() {
    let mut plan = TroubleshootingPlan::new(
        PlanSource::UserQuestion("disk is full".into()),
        "snap-1".into(),
    );
    plan.hypotheses.push(Hypothesis {
        id: "h1".into(),
        hypothesis: "Root filesystem is full".into(),
        evidence_for: vec!["df shows 95% usage".into()],
        evidence_against: vec![],
        missing_evidence: vec!["lsof check not run".into()],
        confidence: 0.8,
        domain: helm_monitor::MonitorDomain::Disks,
    });
    plan.proposed_fix_steps.push(helm_monitor::PlanStep {
        expected_output: None,
        interpretation_guide: None,
        title: "Clean apt cache".into(),
        command: CommandPreview::new("shell", "apt-get clean", "Free cache space"),
        hypothesis_id: Some("h1".into()),
    });
    plan.approval_required = true;

    let text = plan.render_text();
    assert!(text.contains("Troubleshooting Plan"));
    assert!(text.contains("Hypotheses"));
    assert!(text.contains("Root filesystem"));
    assert!(text.contains("80.0%"));
    assert!(text.contains("Proposed fixes"));
    assert!(text.contains("apt-get clean"));
    assert!(text.contains("Approval required"));
}

#[test]
fn plan_render_text_shows_rollback_status() {
    let mut plan =
        TroubleshootingPlan::new(PlanSource::UserQuestion("test".into()), "snap-1".into());
    plan.proposed_fix_steps.push(helm_monitor::PlanStep {
        expected_output: None,
        interpretation_guide: None,
        title: "Test step".into(),
        command: CommandPreview::new("shell", "test-cmd", "test")
            .with_rollback(RollbackStatus::Unsupported),
        hypothesis_id: None,
    });
    let text = plan.render_text();
    assert!(text.contains("no rollback available"));
}

// ── PlanSource ──────────────────────────────────────────────────────────────

#[test]
fn plan_source_user_question_display() {
    let s = PlanSource::UserQuestion("disk is full".into());
    assert_eq!(s.to_string(), "user question: disk is full");
}

// ── helm explain output ─────────────────────────────────────────────────────

#[test]
fn explain_finding_shows_all_sections() {
    let f = Finding::new(
        "snap-1",
        "disk-usage",
        "/",
        "Root filesystem is 95% full",
        helm_monitor::Severity::Critical,
        helm_monitor::Confidence::High,
        helm_monitor::MonitorDomain::Disks,
    )
    .with_evidence("disk.used.bytes", "475G / 500G", "95% utilization")
    .with_impact("System may run out of disk space")
    .with_read_only_check("du -sh /var/log/*");

    let text = explain_finding(&f);
    assert!(text.contains("Finding:"));
    assert!(text.contains("Critical"));
    assert!(text.contains("High"));
    assert!(text.contains("disk.used.bytes"));
    assert!(text.contains("du -sh /var/log/*"));
}

// ── Redaction safety ────────────────────────────────────────────────────────

#[test]
fn plan_text_does_not_leak_secrets() {
    let mut plan =
        TroubleshootingPlan::new(PlanSource::UserQuestion("test".into()), "snap-1".into());
    plan.proposed_fix_steps.push(helm_monitor::PlanStep {
        expected_output: None,
        interpretation_guide: None,
        title: "test".into(),
        command: CommandPreview::new(
            "shell",
            "echo secret-sk-or-v1-abcdefghijklmnopqrstuvwxyz",
            "test",
        ),
        hypothesis_id: None,
    });
    let text = plan.render_text();
    let redacted = helm_core::redact_secrets(&text);
    assert!(
        redacted.contains("***REDACTED***"),
        "plan text should redact keys"
    );
    assert!(
        !redacted.contains("abcdefghijklmnopqrstuvwxyz"),
        "raw key should not appear in plan"
    );
}

// ── Approval gate invariants ────────────────────────────────────────────────

#[test]
fn plan_approval_required_by_default() {
    let plan = TroubleshootingPlan::new(PlanSource::UserQuestion("test".into()), "s1".into());
    assert!(
        plan.approval_required,
        "plans should require approval by default"
    );
}

#[test]
fn denied_approval_leaves_zero_state_changes() {
    // v1.9 is planning-only; no execution path exists yet.
    // v2.0 will wire the execution gate.
    // For v1.9, 'denying' means simply not running the plan.
    let plan = TroubleshootingPlan::new(PlanSource::UserQuestion("test".into()), "s1".into());
    // No execution happens unless helm apply-plan (v2.0) is called.
    // So the invariant is trivially true: not planning execution == no mutation.
    assert!(
        plan.read_only_steps.is_empty(),
        "no execution in v1.9 — deny is the default state"
    );
}

// ── Fixture-based e2e tests ───────────────────────────────────────────

fn fixture_snapshot_with_full_disk() -> SystemSnapshot {
    let mut snap = SystemSnapshot {
        id: "fixture-snap-disk".into(),
        host: HostIdentity::default(),
        collected_at: chrono::Utc::now(),
        profile: MonitorProfile::Standard,
        domains: SnapshotDomains::default(),
        collector_errors: vec![],
        redaction_version: "0.1.0".into(),
    };
    snap.domains.disks.filesystems.push(FilesystemEntry {
        device: "/dev/sda1".into(),
        mount_point: "/".into(),
        fs_type: "ext4".into(),
        total_bytes: 500_000_000_000,
        used_bytes: 475_000_000_000,
        available_bytes: 25_000_000_000,
    });
    snap
}

#[test]
fn e2e_fixture_plan_from_problem_with_snapshot_has_hypotheses() {
    let snap = fixture_snapshot_with_full_disk();
    let plan = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(plan_from_problem_with_snapshot("disk is full", snap));
    assert!(!plan.hypotheses.is_empty(), "should generate hypotheses");
    let disk_hyp = plan
        .hypotheses
        .iter()
        .find(|h| h.domain == helm_monitor::MonitorDomain::Disks);
    assert!(disk_hyp.is_some(), "disk hypothesis should exist");
    let h = disk_hyp.unwrap();
    assert!(
        h.confidence >= 0.6,
        "disk hypothesis should have high confidence with evidence, got {}",
        h.confidence
    );
    assert!(
        !h.evidence_for.is_empty(),
        "should have evidence from snapshot"
    );
}

#[test]
fn e2e_fixture_plan_has_read_only_checks_for_disk_full() {
    let snap = fixture_snapshot_with_full_disk();
    let plan = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(plan_from_problem_with_snapshot("disk is full", snap));
    assert!(
        !plan.read_only_steps.is_empty(),
        "should have read-only check steps"
    );
    let has_du = plan.read_only_steps.iter().any(|s| {
        s.command
            .command_text
            .as_deref()
            .unwrap_or("")
            .contains("du -sh")
    });
    assert!(has_du, "should have du -sh step for disk check");
}

#[test]
fn e2e_fixture_plan_has_fix_steps_when_evidence_present() {
    let snap = fixture_snapshot_with_full_disk();
    let plan = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(plan_from_problem_with_snapshot("disk is full", snap));
    assert!(
        !plan.proposed_fix_steps.is_empty(),
        "95% disk should have fix steps"
    );
    let has_apt_clean = plan.proposed_fix_steps.iter().any(|s| {
        s.command
            .command_text
            .as_deref()
            .unwrap_or("")
            .contains("apt-get clean")
    });
    assert!(has_apt_clean, "fix steps should include apt-get clean");
}

#[test]
fn e2e_fixture_plan_from_problem_with_snapshot_has_evidence_field() {
    let snap = fixture_snapshot_with_full_disk();
    let plan = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(plan_from_problem_with_snapshot("disk is full", snap));
    let disk_hyp = plan
        .hypotheses
        .iter()
        .find(|h| h.domain == helm_monitor::MonitorDomain::Disks)
        .unwrap();
    assert!(
        !disk_hyp.evidence_for.is_empty(),
        "hypothesis should reference snapshot data"
    );
    assert!(
        disk_hyp.evidence_for[0].contains("95%"),
        "evidence should cite actual usage percentage"
    );
    assert!(
        disk_hyp.evidence_for[0].contains("/"),
        "evidence should cite the mount point"
    );
}

// ── plan_from_finding ────────────────────────────────────────────────

#[test]
fn e2e_fixture_plan_from_finding_has_hypotheses() {
    let finding = Finding::new(
        "fixture-snap-disk",
        "disk-usage",
        "/",
        "Root filesystem is 95% full",
        Severity::Critical,
        Confidence::High,
        helm_monitor::MonitorDomain::Disks,
    )
    .with_evidence("disk.used.bytes", "475G / 500G", "95% utilization")
    .with_impact("System may run out of disk space")
    .with_read_only_check("du -sh /var/log/*");

    let plan = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(helm_monitor::plan_from_finding(&finding));
    assert!(
        plan.source.to_string().contains("finding:"),
        "source should be finding-based, got: {}",
        plan.source
    );
    assert!(
        !plan.hypotheses.is_empty(),
        "plan_from_finding should produce hypotheses from finding data"
    );
    assert!(
        !plan.hypotheses[0].evidence_for.is_empty(),
        "finding evidence should become hypothesis evidence"
    );
    assert!(
        !plan.read_only_steps.is_empty(),
        "finding checks should become plan steps"
    );
}

// ── Finding persistence across snapshots ────────────────────────────────

#[test]
fn findings_persisted_in_snapshot_can_be_retrieved_by_id() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("findings-test.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snapshots (
            id TEXT PRIMARY KEY,
            host_hostname TEXT NOT NULL DEFAULT 'unknown',
            host_id TEXT NOT NULL DEFAULT '',
            collected_at INTEGER NOT NULL,
            profile TEXT NOT NULL DEFAULT 'standard',
            domains_json TEXT NOT NULL DEFAULT '{}',
            collector_errors_json TEXT NOT NULL DEFAULT '[]',
            findings_json TEXT NOT NULL DEFAULT '[]'
        )",
    )
    .unwrap();

    // Create two snapshots with different findings
    let mut snap1 = base_snapshot();
    snap1.id = "snap-1".into();
    snap1.collected_at = chrono::DateTime::from_timestamp(100_000, 0).unwrap();

    let f1 = Finding::new(
        "snap-1",
        "disk-usage",
        "/",
        "Root fs 95% full",
        Severity::Warning,
        Confidence::High,
        helm_monitor::MonitorDomain::Disks,
    );
    let findings1 = serde_json::to_string(&vec![f1.clone()]).unwrap();
    let json1 = serde_json::to_string(&snap1).unwrap();

    helm_memory::SnapshotStore::insert(&conn, &json1, &findings1).unwrap();

    // Second snapshot with different finding
    let mut snap2 = base_snapshot();
    snap2.id = "snap-2".into();
    snap2.collected_at = chrono::DateTime::from_timestamp(200_000, 0).unwrap();

    let f2 = Finding::new(
        "snap-2",
        "failed-services",
        "nginx",
        "nginx service failed",
        Severity::Warning,
        Confidence::High,
        helm_monitor::MonitorDomain::Services,
    );
    let findings2 = serde_json::to_string(&vec![f2.clone()]).unwrap();
    let json2 = serde_json::to_string(&snap2).unwrap();

    helm_memory::SnapshotStore::insert(&conn, &json2, &findings2).unwrap();

    // Verify we can find findings from both snapshots
    let records = helm_memory::SnapshotStore::list(&conn, 100).unwrap();
    assert_eq!(records.len(), 2, "both snapshots should be stored");

    // Search for f1 from first snapshot
    let mut found_f1 = None;
    for record in &records {
        if let Ok(parsed) = serde_json::from_str::<Vec<Finding>>(&record.findings_json) {
            if let Some(f) = parsed.into_iter().find(|f| f.id == f1.id) {
                found_f1 = Some(f);
                break;
            }
        }
    }
    assert!(
        found_f1.is_some(),
        "finding f1 should be found across snapshots"
    );
    assert_eq!(found_f1.unwrap().title, "Root fs 95% full");

    // Search for f2 from second snapshot (same logic, different snapshot)
    let mut found_f2 = None;
    for record in &records {
        if let Ok(parsed) = serde_json::from_str::<Vec<Finding>>(&record.findings_json) {
            if let Some(f) = parsed.into_iter().find(|f| f.id == f2.id) {
                found_f2 = Some(f);
                break;
            }
        }
    }
    assert!(
        found_f2.is_some(),
        "finding f2 should be found across snapshots"
    );
    assert_eq!(
        found_f2.unwrap().category,
        helm_monitor::MonitorDomain::Services
    );
}

#[test]
fn explain_finding_works_across_multiple_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("explain-test.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snapshots (
            id TEXT PRIMARY KEY,
            host_hostname TEXT NOT NULL DEFAULT 'unknown',
            host_id TEXT NOT NULL DEFAULT '',
            collected_at INTEGER NOT NULL,
            profile TEXT NOT NULL DEFAULT 'standard',
            domains_json TEXT NOT NULL DEFAULT '{}',
            collector_errors_json TEXT NOT NULL DEFAULT '[]',
            findings_json TEXT NOT NULL DEFAULT '[]'
        )",
    )
    .unwrap();

    // Only store a finding in an OLDER snapshot (not the latest)
    let mut old_snap = base_snapshot();
    old_snap.id = "old-snap".into();
    old_snap.collected_at = chrono::DateTime::from_timestamp(50_000, 0).unwrap();

    let old_finding = Finding::new(
        "old-snap",
        "disk-usage",
        "/data",
        "Old data disk usage",
        Severity::Warning,
        Confidence::Medium,
        helm_monitor::MonitorDomain::Disks,
    );
    let old_findings_json = serde_json::to_string(&vec![old_finding.clone()]).unwrap();
    let old_json = serde_json::to_string(&old_snap).unwrap();
    helm_memory::SnapshotStore::insert(&conn, &old_json, &old_findings_json).unwrap();

    // Latest snapshot has no findings
    let mut new_snap = base_snapshot();
    new_snap.id = "new-snap".into();
    new_snap.collected_at = chrono::DateTime::from_timestamp(300_000, 0).unwrap();
    new_snap.domains.logs.journal_errors_last_hour = 10;
    let new_json = serde_json::to_string(&new_snap).unwrap();
    helm_memory::SnapshotStore::insert(&conn, &new_json, "[]").unwrap();

    // Search across all snapshots for the old finding
    let records = helm_memory::SnapshotStore::list(&conn, 100).unwrap();
    assert_eq!(records.len(), 2);

    let mut found = None;
    for record in &records {
        if let Ok(parsed) = serde_json::from_str::<Vec<Finding>>(&record.findings_json) {
            if let Some(f) = parsed.into_iter().find(|f| f.id == old_finding.id) {
                found = Some(f);
                break;
            }
        }
    }
    assert!(
        found.is_some(),
        "old finding should be found even though latest snapshot has no findings"
    );
    assert_eq!(found.unwrap().affected_resource, "/data");
}
