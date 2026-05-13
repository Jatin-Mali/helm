//! v1.8 Issue Detection And Monitor Report — comprehensive test suite.

use helm_monitor::{
    Confidence, Finding, MonitorDomain, MonitorReport, MonitorReporter, Severity,
    snapshot::{
        FailedUnit, FilesystemEntry, HostIdentity, ListenerEntry, LoadAverage, MemoryInfo,
        MonitorProfile, SnapshotDomains, SystemSnapshot,
    },
};

// ── Golden fixture helpers ──────────────────────────────────────────────────

fn base_snapshot() -> SystemSnapshot {
    SystemSnapshot {
        id: "fixture-1".into(),
        host: HostIdentity {
            hostname: "testbox".into(),
            ..Default::default()
        },
        collected_at: chrono::Utc::now(),
        profile: MonitorProfile::Standard,
        domains: SnapshotDomains::default(),
        collector_errors: vec![],
        redaction_version: "0.1.0".into(),
    }
}

// ── Disk usage detector golden tests ────────────────────────────────────────

#[test]
fn golden_disk_usage_critical() {
    let mut snap = base_snapshot();
    snap.domains.disks.smart_available = true;
    snap.domains.disks.filesystems.push(FilesystemEntry {
        device: "/dev/sda1".into(),
        mount_point: "/".into(),
        fs_type: "ext4".into(),
        total_bytes: 100_000_000_000,
        used_bytes: 96_000_000_000,
        available_bytes: 4_000_000_000,
    });
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Disks]), None);
    let critical: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .collect();
    assert!(!critical.is_empty(), "should detect 96% usage as critical");
    assert!(critical[0].title.contains("/"));
}

#[test]
fn golden_disk_usage_normal_is_silent() {
    let mut snap = base_snapshot();
    snap.domains.disks.filesystems.push(FilesystemEntry {
        device: "/dev/sda1".into(),
        mount_point: "/".into(),
        fs_type: "ext4".into(),
        total_bytes: 100_000_000_000,
        used_bytes: 40_000_000_000,
        available_bytes: 60_000_000_000,
    });
    // Enable SMART to prevent the "no SMART" info finding
    snap.domains.disks.smart_available = true;
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Disks]), None);
    assert!(
        findings.is_empty(),
        "40% usage with SMART available should have no findings"
    );
}

// ── Failed services golden test ─────────────────────────────────────────────

#[test]
fn golden_failed_services() {
    let mut snap = base_snapshot();
    snap.domains.services.failed_units.push(FailedUnit {
        name: "nginx.service".into(),
        description: "nginx web server".into(),
        loaded: "loaded".into(),
        active: "failed".into(),
        sub: "failed".into(),
    });
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Services]), None);
    assert!(!findings.is_empty());
    assert_eq!(findings[0].severity, Severity::Warning);
    assert!(findings[0].title.contains("nginx"));
}

// ── Journal error burst golden test ─────────────────────────────────────────

#[test]
fn golden_journal_error_burst() {
    let mut snap = base_snapshot();
    snap.domains.logs.journal_errors_last_hour = 150;
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Logs]), None);
    let warnings: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .collect();
    assert!(
        !warnings.is_empty(),
        "150 errors in 1h should trigger warning"
    );
    assert!(warnings[0].title.contains("150"));
}

#[test]
fn golden_journal_normal_is_silent() {
    let snap = base_snapshot(); // 0 errors
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Logs]), None);
    assert!(findings.is_empty(), "0 errors should produce no findings");
}

// ── Exposed port golden test ────────────────────────────────────────────────

#[test]
fn golden_exposed_port() {
    let mut snap = base_snapshot();
    snap.domains.ports.listeners.push(ListenerEntry {
        protocol: "tcp".into(),
        local_address: "0.0.0.0".into(),
        local_port: 8080,
        process_name: Some("app".into()),
        pid: Some(1234),
    });
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Ports]), None);
    assert!(!findings.is_empty());
    assert!(findings[0].title.contains("8080"));
}

#[test]
fn golden_localhost_port_is_not_exposed() {
    let mut snap = base_snapshot();
    snap.domains.ports.listeners.push(ListenerEntry {
        protocol: "tcp".into(),
        local_address: "127.0.0.1".into(),
        local_port: 8080,
        process_name: Some("app".into()),
        pid: Some(1234),
    });
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Ports]), None);
    assert!(
        findings.is_empty(),
        "localhost ports should not flag as exposed"
    );
}

// ── High load golden test ───────────────────────────────────────────────────

#[test]
fn golden_high_load() {
    let mut snap = base_snapshot();
    snap.domains.load.cpu_logical_count = 4;
    snap.domains.load.load_average = LoadAverage {
        one: 8.0,
        five: 7.0,
        fifteen: 6.5,
    };
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Load]), None);
    let warns: Vec<_> = findings
        .iter()
        .filter(|f| f.severity >= Severity::Warning)
        .collect();
    assert!(
        !warns.is_empty(),
        "load 6.5 on 4 cores should trigger warning"
    );
}

#[test]
fn golden_normal_load_is_silent() {
    let mut snap = base_snapshot();
    snap.domains.load.cpu_logical_count = 4;
    snap.domains.load.load_average = LoadAverage {
        one: 1.0,
        five: 0.8,
        fifteen: 0.5,
    };
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Load]), None);
    let warns: Vec<_> = findings
        .iter()
        .filter(|f| f.severity >= Severity::Warning)
        .collect();
    assert!(warns.is_empty(), "normal load should be silent");
}

// ── Memory pressure golden test ─────────────────────────────────────────────

#[test]
fn golden_memory_pressure_critical() {
    let mut snap = base_snapshot();
    snap.domains.load.memory = MemoryInfo {
        total: 16_000_000_000,
        used: 15_500_000_000,
        available: Some(500_000_000),
    };
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Load]), None);
    let crit: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .collect();
    assert!(!crit.is_empty(), "96% memory usage should be critical");
}

// ── Backup detection golden test ────────────────────────────────────────────

#[test]
fn golden_no_backup_tools() {
    let snap = base_snapshot(); // no backup tools
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Backups]), None);
    let warns: Vec<_> = findings
        .iter()
        .filter(|f| f.severity >= Severity::Warning)
        .collect();
    assert!(!warns.is_empty(), "no backup tools should trigger warning");
    assert!(warns[0].title.contains("No backup"));
}

// ── MonitorReport tests ─────────────────────────────────────────────────────

#[test]
fn monitor_report_severity_counts() {
    let mut snap = base_snapshot();
    snap.domains.disks.filesystems.push(FilesystemEntry {
        device: "/dev/sda1".into(),
        mount_point: "/".into(),
        fs_type: "ext4".into(),
        total_bytes: 100_000_000_000,
        used_bytes: 97_000_000_000,
        available_bytes: 3_000_000_000,
    });
    let reporter = MonitorReporter::new();
    let findings = reporter.registry.detect(&snap, None, None);
    let report = MonitorReport {
        snapshot: snap,
        findings,
        domains_checked: vec![MonitorDomain::Disks],
        previous_snapshot_id: None,
    };
    let (info, warning, critical) = report.severity_counts();
    assert!(critical > 0);
    assert!(info + warning + critical > 0);
}

#[test]
fn monitor_report_text_contains_findings() {
    let mut snap = base_snapshot();
    snap.domains.disks.filesystems.push(FilesystemEntry {
        device: "/dev/sda1".into(),
        mount_point: "/".into(),
        fs_type: "ext4".into(),
        total_bytes: 100_000_000_000,
        used_bytes: 97_000_000_000,
        available_bytes: 3_000_000_000,
    });
    let reporter = MonitorReporter::new();
    let findings = reporter.registry.detect(&snap, None, None);
    let report = MonitorReport {
        snapshot: snap,
        findings,
        domains_checked: vec![],
        previous_snapshot_id: None,
    };
    let text = report.render_text();
    assert!(text.contains("CRITICAL"));
    assert!(text.contains("/"));
}

#[test]
fn monitor_report_json_is_valid() {
    let mut snap = base_snapshot();
    snap.domains.disks.filesystems.push(FilesystemEntry {
        device: "/dev/sda1".into(),
        mount_point: "/".into(),
        fs_type: "ext4".into(),
        total_bytes: 100_000_000_000,
        used_bytes: 97_000_000_000,
        available_bytes: 3_000_000_000,
    });
    let reporter = MonitorReporter::new();
    let findings = reporter.registry.detect(&snap, None, None);
    let report = MonitorReport {
        snapshot: snap,
        findings,
        domains_checked: vec![],
        previous_snapshot_id: None,
    };
    let json = report.render_json();
    let parsed: Vec<Finding> = serde_json::from_str(&json).unwrap();
    assert!(!parsed.is_empty());
}

#[test]
fn monitor_report_markdown_contains_table() {
    let mut snap = base_snapshot();
    snap.domains.logs.journal_errors_last_hour = 200;
    let reporter = MonitorReporter::new();
    let findings = reporter.registry.detect(&snap, None, None);
    let report = MonitorReport {
        snapshot: snap,
        findings,
        domains_checked: vec![],
        previous_snapshot_id: None,
    };
    let md = report.render_markdown();
    assert!(md.contains("HELM Monitor Report"));
    assert!(md.contains("| Severity |"));
}

// ── Domain filtering test ───────────────────────────────────────────────────

#[test]
fn domain_filter_restricts_findings() {
    let mut snap = base_snapshot();
    snap.domains.disks.filesystems.push(FilesystemEntry {
        device: "/dev/sda1".into(),
        mount_point: "/".into(),
        fs_type: "ext4".into(),
        total_bytes: 100_000_000_000,
        used_bytes: 97_000_000_000,
        available_bytes: 3_000_000_000,
    });
    let reporter = MonitorReporter::new();
    // Only check services domain — should find nothing from disks
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Services]), None);
    assert!(
        findings.is_empty(),
        "services filter should exclude disk findings"
    );
}

// ── No-mutation invariant ───────────────────────────────────────────────────

#[test]
fn monitor_report_no_mutation() {
    let snap = base_snapshot();
    let reporter = MonitorReporter::new();
    let findings = reporter.registry.detect(&snap, None, None);
    // Detectors are pure functions — snap is not mutated
    assert_eq!(snap.id, "fixture-1");
    // Findings must cite snapshot fields, not be empty
    // (if no issues found, that's valid)
    for f in &findings {
        assert!(
            !f.evidence.is_empty() || !f.title.is_empty(),
            "every finding has evidence or title"
        );
    }
}

// ── Finding model tests ─────────────────────────────────────────────────────

#[test]
fn finding_fields_present() {
    let f = Finding::new(
        "s1",
        "d1",
        "/tmp",
        "test",
        Severity::Info,
        Confidence::Low,
        MonitorDomain::Disks,
    )
    .with_evidence("disk.used", "50G", "half full")
    .with_impact("may fill up");
    assert!(!f.id.is_empty());
    assert_eq!(f.snapshot_id, "s1");
    assert_eq!(f.severity, Severity::Info);
    assert_eq!(f.confidence, Confidence::Low);
    assert_eq!(f.evidence.len(), 1);
    assert_eq!(f.evidence[0].source, "disk.used");
}

// ── Redaction in monitor output ─────────────────────────────────────────────

#[test]
fn monitor_json_redacts_secrets() {
    let snap = base_snapshot();
    let reporter = MonitorReporter::new();
    let findings = reporter.registry.detect(&snap, None, None);
    let report = MonitorReport {
        snapshot: snap,
        findings,
        domains_checked: vec![],
        previous_snapshot_id: None,
    };
    let raw = report.render_json();
    let redacted = helm_core::redact_secrets(&raw);
    assert_eq!(
        raw, redacted,
        "monitor JSON should not contain secrets in fixture"
    );
}

// ── Baseline comparison tests ───────────────────────────────────────────────

#[test]
fn baseline_previous_snapshot_id_is_set() {
    let mut snap = base_snapshot();
    snap.domains.logs.journal_errors_last_hour = 30;
    let mut prev = base_snapshot();
    prev.id = "previous-snap".into();
    prev.domains.logs.journal_errors_last_hour = 10;

    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Logs]), Some(&prev));
    // Previous had 10 errors, current has 30 (>2x) -> should trigger elevated
    let infos: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Info)
        .collect();
    assert!(
        !infos.is_empty(),
        "30 errors with 10 baseline should trigger elevated info"
    );
    assert!(infos[0].title.contains("was 10"));
}

#[test]
fn baseline_stable_noisy_system_avoids_false_burst() {
    let mut snap = base_snapshot();
    snap.domains.logs.journal_errors_last_hour = 25;
    let mut prev = base_snapshot();
    prev.id = "baseline".into();
    prev.domains.logs.journal_errors_last_hour = 20;

    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Logs]), Some(&prev));
    // 25 vs 20 is not >2x, so no info either
    let infos: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Info)
        .collect();
    assert!(
        infos.is_empty(),
        "25 errors vs 20 baseline should NOT flag as elevated info (<2x increase)"
    );
}

#[test]
fn baseline_critical_burst_when_2x_prior() {
    let mut snap = base_snapshot();
    snap.domains.logs.journal_errors_last_hour = 200;
    let mut prev = base_snapshot();
    prev.id = "baseline".into();
    prev.domains.logs.journal_errors_last_hour = 80;

    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Logs]), Some(&prev));
    let crit: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .collect();
    assert!(
        !crit.is_empty(),
        "200 errors with 80 baseline (>2x) should be critical"
    );
}

#[test]
fn baseline_report_shows_previous_snapshot_id_in_text() {
    let mut snap = base_snapshot();
    snap.domains.logs.journal_errors_last_hour = 200;
    let reporter = MonitorReporter::new();
    let findings = reporter.registry.detect(&snap, None, None);
    let report = MonitorReport {
        snapshot: snap,
        findings,
        domains_checked: vec![MonitorDomain::Logs],
        previous_snapshot_id: Some("baseline-snap-123".into()),
    };
    let text = report.render_text();
    assert!(text.contains("Baseline: baseline-snap-123"));
}

#[test]
fn baseline_without_previous_does_not_show_baseline_line() {
    let snap = base_snapshot();
    let reporter = MonitorReporter::new();
    let findings = reporter.registry.detect(&snap, None, None);
    let report = MonitorReport {
        snapshot: snap,
        findings,
        domains_checked: vec![],
        previous_snapshot_id: None,
    };
    let text = report.render_text();
    assert!(!text.contains("Baseline:"));
}

// ── Monitor snapshot persistence test ────────────────────────────────────────

#[test]
fn monitor_snapshot_is_persisted_and_reloaded_as_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("monitor-test.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snapshots (
            id TEXT PRIMARY KEY,
            host_hostname TEXT NOT NULL DEFAULT 'unknown',
            collected_at INTEGER NOT NULL,
            profile TEXT NOT NULL DEFAULT 'standard',
            domains_json TEXT NOT NULL DEFAULT '{}',
            collector_errors_json TEXT NOT NULL DEFAULT '[]'
        )",
    )
    .unwrap();

    // First run: persist a snapshot
    let mut snap1 = base_snapshot();
    snap1.id = "monitor-run-1".into();
    snap1.collected_at = chrono::DateTime::from_timestamp(100_000, 0).unwrap();
    snap1.domains.logs.journal_errors_last_hour = 10;
    let json1 = serde_json::to_string(&snap1).unwrap();
    helm_memory::SnapshotStore::insert(&conn, &json1).unwrap();

    // Verify it's stored
    let latest = helm_memory::SnapshotStore::list(&conn, 10).unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].id, "monitor-run-1");

    // Second run: persist another snapshot with higher error count
    let mut snap2 = base_snapshot();
    snap2.id = "monitor-run-2".into();
    snap2.collected_at = chrono::DateTime::from_timestamp(200_000, 0).unwrap();
    snap2.domains.logs.journal_errors_last_hour = 200;
    let json2 = serde_json::to_string(&snap2).unwrap();
    helm_memory::SnapshotStore::insert(&conn, &json2).unwrap();

    // Latest is now snap2
    let latest2 = helm_memory::SnapshotStore::latest(&conn).unwrap().unwrap();
    assert_eq!(latest2.id, "monitor-run-2");

    // Load snap1 as previous baseline, detect using snap2
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap2, Some(&[MonitorDomain::Logs]), Some(&snap1));
    let crit: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .collect();
    assert!(
        !crit.is_empty(),
        "200 errors vs 10 baseline (>2x) should be critical"
    );
    assert!(crit[0].title.contains("previous: 10"));
}

#[test]
fn monitor_persistence_advances_baseline_across_runs() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("monitor-test2.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snapshots (
            id TEXT PRIMARY KEY,
            host_hostname TEXT NOT NULL DEFAULT 'unknown',
            collected_at INTEGER NOT NULL,
            profile TEXT NOT NULL DEFAULT 'standard',
            domains_json TEXT NOT NULL DEFAULT '{}',
            collector_errors_json TEXT NOT NULL DEFAULT '[]'
        )",
    )
    .unwrap();

    // Simulate three consecutive monitor runs
    let runs = [
        ("run-1", 50u64, 100_000i64),
        ("run-2", 55u64, 200_000i64),
        ("run-3", 200u64, 300_000i64),
    ];

    for (i, (id, errors, ts)) in runs.iter().enumerate() {
        let mut snap = base_snapshot();
        snap.id = id.to_string();
        snap.collected_at = chrono::DateTime::from_timestamp(*ts, 0).unwrap();
        snap.domains.logs.journal_errors_last_hour = *errors;
        let json = serde_json::to_string(&snap).unwrap();
        helm_memory::SnapshotStore::insert(&conn, &json).unwrap();

        let count = helm_memory::SnapshotStore::list(&conn, 10).unwrap().len();
        assert_eq!(count, i + 1, "snapshot count should increase each run");
    }

    // After 3 runs, 3 snapshots exist
    let list = helm_memory::SnapshotStore::list(&conn, 10).unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].id, "run-3"); // newest first
    assert_eq!(list[2].id, "run-1"); // oldest last
}

// ── Watch mode remains read-only ────────────────────────────────────────────

#[test]
fn watch_mode_detectors_dont_mutate_snapshot() {
    let mut snap = base_snapshot();
    snap.domains.disks.smart_available = true;
    let snap_clone = snap.clone();
    let reporter = MonitorReporter::new();
    // Run detect multiple times — snapshot should not change
    for _ in 0..3 {
        let _ = reporter.registry.detect(&snap, None, None);
    }
    assert_eq!(
        snap.domains.disks.filesystems.len(),
        snap_clone.domains.disks.filesystems.len()
    );
    assert_eq!(
        snap.domains.logs.journal_errors_last_hour,
        snap_clone.domains.logs.journal_errors_last_hour
    );
}

// ── Restore-test gap golden test ────────────────────────────────────────────

#[test]
fn golden_restore_test_missing_when_tools_no_evidence() {
    let mut snap = base_snapshot();
    snap.domains
        .backups
        .tools_detected
        .push(helm_monitor::BackupTool {
            name: "restic".into(),
            binary_path: Some("/usr/bin/restic".into()),
            config_path: None,
            repo_path: None,
            restore_test_evidence: None,
        });
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Backups]), None);
    let warns: Vec<_> = findings
        .iter()
        .filter(|f| f.severity >= Severity::Warning)
        .collect();
    assert!(
        !warns.is_empty(),
        "restic without restore evidence should trigger warning"
    );
    // Filter to restore-test finding specifically (other backup findings may also warn)
    let restore = warns.iter().find(|f| f.title.contains("restore-test"));
    assert!(
        restore.is_some(),
        "restore-test finding should exist among warns"
    );
    let restore = restore.unwrap();
    assert!(restore.evidence[0].source.contains("restore_test_evidence"));
}

#[test]
fn golden_restore_test_ok_when_evidence_present() {
    let mut snap = base_snapshot();
    snap.domains
        .backups
        .tools_detected
        .push(helm_monitor::BackupTool {
            name: "restic".into(),
            binary_path: Some("/usr/bin/restic".into()),
            config_path: None,
            repo_path: None,
            restore_test_evidence: Some("restic cache present".into()),
        });
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Backups]), None);
    let restore_warns: Vec<_> = findings
        .iter()
        .filter(|f| f.title.contains("restore-test"))
        .collect();
    assert!(
        restore_warns.is_empty(),
        "restic with evidence should NOT trigger restore-test finding"
    );
}

#[test]
fn golden_no_tools_does_not_fire_restore_test() {
    let snap = base_snapshot(); // no tools
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Backups]), None);
    let restore_warns: Vec<_> = findings
        .iter()
        .filter(|f| f.title.contains("restore-test"))
        .collect();
    assert!(
        restore_warns.is_empty(),
        "no tools should not trigger restore-test finding"
    );
}

// ── OOM detector hard rule compliance tests ─────────────────────────────────

#[test]
fn golden_oom_kernel_log_with_oom() {
    let mut snap = base_snapshot();
    snap.domains
        .logs
        .kernel_errors
        .push("2026-05-14 OOM killer: Killed process 1234 (nginx) total-vm:500000kB".into());
    snap.domains
        .logs
        .kernel_errors
        .push("2026-05-14 Out of memory: httpserver invoked oom-killer".into());
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Load]), None);
    let crit: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .filter(|f| f.title.contains("OOM"))
        .collect();
    assert!(
        !crit.is_empty(),
        "OOM traces should trigger critical finding"
    );
    // Must contain window info
    assert!(
        crit[0].title.contains("hour"),
        "OOM finding must specify time window"
    );
    // Must contain count
    let ev = &crit[0].evidence[0];
    assert!(
        ev.value.contains("2") || ev.value.contains("trace(s)"),
        "OOM evidence must contain count of traces: {}",
        ev.value
    );
    // Must cite snapshot field
    assert_eq!(
        ev.source, "logs.kernel_errors",
        "OOM must cite exact snapshot field"
    );
}

#[test]
fn golden_oom_no_traces_no_finding() {
    let snap = base_snapshot();
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Load]), None);
    let oom: Vec<_> = findings
        .iter()
        .filter(|f| f.title.contains("OOM"))
        .collect();
    assert!(
        oom.is_empty(),
        "no OOM traces should not trigger OOM finding"
    );
}

#[test]
fn golden_oom_evidence_has_window_and_count() {
    let mut snap = base_snapshot();
    snap.domains
        .logs
        .kernel_errors
        .push("2026-05-14 Out of memory: Killed process 5678".into());
    let reporter = MonitorReporter::new();
    let findings = reporter
        .registry
        .detect(&snap, Some(&[MonitorDomain::Load]), None);
    let oom: Vec<_> = findings
        .iter()
        .filter(|f| f.title.contains("OOM"))
        .collect();
    assert_eq!(oom.len(), 1, "1 OOM trace -> 1 finding");
    // Per ROADMAP.md:343-344: source, window, count, confidence must be present
    assert!(
        oom[0].evidence[0].note.contains("window"),
        "OOM evidence must mention the collection time window"
    );
    assert!(
        oom[0].evidence[0].value.chars().any(|c| c.is_ascii_digit()),
        "OOM evidence value must contain a numeric count"
    );
}
