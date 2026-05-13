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
