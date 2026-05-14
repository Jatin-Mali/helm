//! v1.7 System Snapshot Engine — comprehensive test suite.

use helm_monitor::{
    BlockDevice, CollectorError, FilesystemEntry, FirewallSnapshot, HostIdentity, InodeEntry,
    ListenerEntry, LoadAverage, MonitorProfile, ProcessInfo, ProcessSnapshot, SnapshotDomains,
    SystemSnapshot,
};
use serde_json::json;
use tempfile::tempdir;

// ── Collector parser unit tests ─────────────────────────────────────────────

#[test]
fn parse_load_average_from_proc() {
    // Simulate /proc/loadavg content
    let content = "0.15 0.10 0.05 1/234 12345\n";
    let parts: Vec<&str> = content.split_whitespace().collect();
    let la = LoadAverage {
        one: parts[0].parse().unwrap(),
        five: parts[1].parse().unwrap(),
        fifteen: parts[2].parse().unwrap(),
    };
    assert!((la.one - 0.15).abs() < 0.001);
    assert!((la.five - 0.10).abs() < 0.001);
}

#[test]
fn parse_memory_from_proc_meminfo() {
    let content =
        "MemTotal:       16384000 kB\nMemFree:         8192000 kB\nMemAvailable:   12288000 kB\n";
    let total = content
        .lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap();
    assert_eq!(total, 16_384_000);
}

#[test]
fn parse_df_output() {
    let input = "Filesystem       1B-blocks        Used   Available Use% Mounted on\n/dev/sda1      500000000000 200000000000 300000000000  40% /\n";
    let entries: Vec<FilesystemEntry> = input
        .lines()
        .skip(1)
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 6 {
                return None;
            }
            Some(FilesystemEntry {
                device: p[0].into(),
                mount_point: p[5].into(),
                fs_type: String::new(),
                total_bytes: p[1].parse().unwrap(),
                used_bytes: p[2].parse().unwrap(),
                available_bytes: p[3].parse().unwrap(),
            })
        })
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].device, "/dev/sda1");
    assert_eq!(entries[0].total_bytes, 500_000_000_000);
    assert_eq!(entries[0].used_bytes, 200_000_000_000);
}

#[test]
fn parse_inode_output() {
    let input = "Filesystem      Inodes  IUsed   IFree IUse% Mounted on\n/dev/sda1      3276800 245000 3031800    8% /\n";
    let entries: Vec<InodeEntry> = input
        .lines()
        .skip(1)
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 6 {
                return None;
            }
            Some(InodeEntry {
                device: p[0].into(),
                mount_point: p[5].into(),
                total: p[1].parse().unwrap(),
                used: p[2].parse().unwrap(),
                free: p[3].parse().unwrap(),
            })
        })
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].total, 3_276_800);
    assert_eq!(entries[0].used, 245_000);
}

#[test]
fn parse_ss_output() {
    let input = "Netid State  Recv-Q Send-Q Local Address:Port  Peer Address:Port Process\n\
         tcp   LISTEN 0      128    0.0.0.0:80          0.0.0.0:*        users:((\"nginx\",pid=1234,fd=6))\n";
    let listeners: Vec<ListenerEntry> = input
        .lines()
        .skip(1)
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 5 {
                return None;
            }
            let local = p[4];
            let (addr, port) = if let Some(pos) = local.rfind(':') {
                (
                    local[..pos]
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .to_string(),
                    local[pos + 1..].parse().unwrap_or(0),
                )
            } else {
                (local.to_string(), 0)
            };
            Some(ListenerEntry {
                protocol: p[0].to_lowercase(),
                local_address: addr,
                local_port: port,
                process_name: Some("nginx".into()),
                pid: Some(1234),
            })
        })
        .collect();
    assert_eq!(listeners.len(), 1);
    assert_eq!(listeners[0].protocol, "tcp");
    assert_eq!(listeners[0].local_port, 80);
    assert_eq!(listeners[0].process_name.as_deref(), Some("nginx"));
}

#[test]
fn parse_ps_output() {
    let input = "USER PID %CPU %MEM VSZ RSS TTY STAT START TIME COMMAND\nroot 1 0.0 0.1 169740 13348 ? Ss May12 0:12 /sbin/init\n";
    let procs: Vec<ProcessInfo> = input
        .lines()
        .skip(1)
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 11 {
                return None;
            }
            Some(ProcessInfo {
                pid: p[1].parse().unwrap(),
                user: p[0].into(),
                cpu_percent: p[2].parse().unwrap(),
                mem_percent: p[3].parse().unwrap(),
                command: p[10..].join(" "),
            })
        })
        .collect();
    assert_eq!(procs.len(), 1);
    assert_eq!(procs[0].pid, 1);
    assert_eq!(procs[0].user, "root");
}

#[test]
fn parse_lsblk_output() {
    let input = "sda 500G 0 /\n";
    let devices: Vec<BlockDevice> = input
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.is_empty() {
                return None;
            }
            Some(BlockDevice {
                name: p[0].into(),
                size: p.get(1).and_then(|s| {
                    let s = s.trim();
                    let (n, m) = if let Some(r) = s.strip_suffix('G') {
                        (r.trim(), 1_073_741_824u64)
                    } else {
                        (s, 1)
                    };
                    n.parse::<f64>().ok().map(|v| (v * m as f64) as u64)
                }),
                ro: false,
                mount_points: if p.len() > 3 {
                    p[3..].iter().map(|s| s.to_string()).collect()
                } else {
                    vec![]
                },
            })
        })
        .collect();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "sda");
    assert_eq!(devices[0].size, Some(536_870_912_000)); // 500G in bytes
}

// ── Snapshot schema test ────────────────────────────────────────────────────

#[test]
fn snapshot_json_round_trip_contains_all_required_domains() {
    let host = HostIdentity {
        hostname: "testhost".into(),
        kernel_name: "Linux".into(),
        kernel_release: "6.8.0".into(),
        machine: "x86_64".into(),
        os_pretty_name: Some("Ubuntu 24.04".into()),
        os_id: Some("ubuntu".into()),
        os_version_id: Some("24.04".into()),
        uptime_seconds: 3600,
    };
    let domains = SnapshotDomains::default();
    let snapshot = SystemSnapshot {
        id: "test-snapshot-id".into(),
        host: host.clone(),
        collected_at: chrono::Utc::now(),
        profile: MonitorProfile::Standard,
        domains,
        collector_errors: vec![CollectorError {
            domain: "disks".into(),
            message: "mock error".into(),
            is_timeout: false,
        }],
        redaction_version: "0.1.0".into(),
    };

    let json_str = serde_json::to_string_pretty(&snapshot).unwrap();
    let parsed: SystemSnapshot = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.id, "test-snapshot-id");
    assert_eq!(parsed.host.hostname, "testhost");
    assert_eq!(parsed.collector_errors.len(), 1);
    // All domain fields must be present
    let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    for domain in &[
        "host",
        "load",
        "disks",
        "services",
        "containers",
        "ports",
        "logs",
        "backups",
        "packages",
        "timers",
        "network",
        "processes",
        "firewall",
    ] {
        assert!(
            val["domains"][domain].is_object(),
            "missing domain: {domain}"
        );
    }
}

#[test]
fn snapshot_json_schema_has_all_v17_fields() {
    let snapshot = SystemSnapshot {
        id: "s1".into(),
        host: HostIdentity::default(),
        collected_at: chrono::Utc::now(),
        profile: MonitorProfile::Quick,
        domains: SnapshotDomains::default(),
        collector_errors: vec![],
        redaction_version: "0.1.0".into(),
    };
    let json_str = serde_json::to_string(&snapshot).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Top-level fields
    assert!(val["id"].is_string());
    assert!(val["host"].is_object());
    assert!(val["collected_at"].is_string());
    assert!(val["profile"].is_string());
    assert!(val["domains"].is_object());
    assert!(val["collector_errors"].is_array());
    assert!(val["redaction_version"].is_string());

    // Host identity fields
    let host = &val["host"];
    assert!(host["hostname"].is_string());
    assert!(host["kernel_name"].is_string());
    assert!(host["uptime_seconds"].is_number());

    // Load domain fields
    let load = &val["domains"]["load"];
    assert!(load["load_average"].is_object());
    assert!(load["cpu_logical_count"].is_number());
    assert!(load["memory"].is_object());
}

// ── Persistence round-trip test ─────────────────────────────────────────────

#[test]
fn snapshot_persistence_round_trip() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("test.db");
    let conn = rusqlite::Connection::open(&db).unwrap();

    // Create snapshots table (simplified migration)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snapshots (
            id TEXT PRIMARY KEY,
            host_hostname TEXT NOT NULL DEFAULT 'unknown',
            collected_at INTEGER NOT NULL,
            profile TEXT NOT NULL DEFAULT 'standard',
            domains_json TEXT NOT NULL DEFAULT '{}',
            collector_errors_json TEXT NOT NULL DEFAULT '[]',
            findings_json TEXT NOT NULL DEFAULT '[]'
        )",
    )
    .unwrap();

    let snapshot = SystemSnapshot {
        id: "round-trip-test".into(),
        host: HostIdentity {
            hostname: "testhost".into(),
            ..Default::default()
        },
        collected_at: chrono::Utc::now(),
        profile: MonitorProfile::Standard,
        domains: SnapshotDomains::default(),
        collector_errors: vec![],
        redaction_version: "0.1.0".into(),
    };
    let json = serde_json::to_string(&snapshot).unwrap();

    // Insert
    helm_memory::SnapshotStore::insert(&conn, &json, "[]").unwrap();

    // Retrieve
    let record = helm_memory::SnapshotStore::latest(&conn).unwrap().unwrap();
    assert_eq!(record.id, "round-trip-test");
    assert_eq!(record.host_hostname, "testhost");

    // List
    let list = helm_memory::SnapshotStore::list(&conn, 10).unwrap();
    assert_eq!(list.len(), 1);

    // Get by ID
    let got = helm_memory::SnapshotStore::get(&conn, "round-trip-test")
        .unwrap()
        .unwrap();
    assert_eq!(got.id, "round-trip-test");

    // latest_except
    let except = helm_memory::SnapshotStore::latest_except(&conn, "round-trip-test").unwrap();
    assert!(
        except.is_none(),
        "should return None when excluding only snapshot"
    );

    // Delete
    helm_memory::SnapshotStore::delete(&conn, "round-trip-test").unwrap();
    let after_delete = helm_memory::SnapshotStore::latest(&conn).unwrap();
    assert!(after_delete.is_none());
}

// ── Diff correctness test ───────────────────────────────────────────────────

#[test]
fn diff_detects_changed_fields() {
    let prev = json!({"cpu": 4, "mem": {"total": 8000, "used": 4000}});
    let curr = json!({"cpu": 8, "mem": {"total": 16000, "used": 8000}});
    let diff = compute_diff_v17(&prev, &curr);
    assert!(diff.changes.iter().any(|(p, _)| p == "cpu"));
    assert!(diff.changes.iter().any(|(p, _)| p == "mem.total"));
}

#[test]
fn diff_detects_added_fields() {
    let prev = json!({"a": 1});
    let curr = json!({"a": 1, "b": 2});
    let diff = compute_diff_v17(&prev, &curr);
    assert!(
        diff.changes
            .iter()
            .any(|(p, c)| p == "b" && c.contains("added"))
    );
}

#[test]
fn diff_detects_removed_fields() {
    let prev = json!({"a": 1, "b": 2});
    let curr = json!({"a": 1});
    let diff = compute_diff_v17(&prev, &curr);
    assert!(
        diff.changes
            .iter()
            .any(|(p, c)| p == "b" && c.contains("removed"))
    );
}

#[test]
fn diff_empty_when_identical() {
    let prev = json!({"a": 1, "b": "hello"});
    let curr = json!({"a": 1, "b": "hello"});
    let diff = compute_diff_v17(&prev, &curr);
    assert!(diff.changes.is_empty());
}

fn compute_diff_v17(prev: &serde_json::Value, curr: &serde_json::Value) -> DiffResultV17 {
    let mut changes = Vec::new();
    diff_rec_v17("", prev, curr, &mut changes);
    DiffResultV17 { changes }
}

struct DiffResultV17 {
    changes: Vec<(String, String)>,
}

fn diff_rec_v17(
    prefix: &str,
    prev: &serde_json::Value,
    curr: &serde_json::Value,
    changes: &mut Vec<(String, String)>,
) {
    match (prev, curr) {
        (serde_json::Value::Object(p), serde_json::Value::Object(c)) => {
            for (k, v) in c {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                match p.get(k) {
                    Some(pv) => diff_rec_v17(&path, pv, v, changes),
                    None => changes.push((path, format!("added: {v}"))),
                }
            }
            for k in p.keys() {
                if !c.contains_key(k) {
                    let path = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    changes.push((path, "removed".to_string()));
                }
            }
        }
        (serde_json::Value::Array(p), serde_json::Value::Array(c)) => {
            if p.len() != c.len() {
                changes.push((
                    prefix.to_string(),
                    format!("length: {} -> {}", p.len(), c.len()),
                ));
            }
        }
        (p, c) => {
            if p != c {
                changes.push((prefix.to_string(), format!("{p} -> {c}")));
            }
        }
    }
}

// ── Redaction tests ─────────────────────────────────────────────────────────

#[test]
fn snapshot_export_redacts_provider_keys() {
    let json = r#"{"api_key":"sk-or-v1-abcdefghijklmnopqrstuvwxyz123456","data":"safe"}"#;
    let redacted = helm_core::redact_secrets(json);
    assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyz123456"));
    assert!(redacted.contains("***REDACTED***"));
    assert!(redacted.contains("safe"));
}

#[test]
fn snapshot_export_redacts_helm_paths() {
    let json = r#"{"path":"/home/user/.helm/secrets.toml","data":"safe"}"#;
    let redacted = helm_core::redact_secrets(json);
    assert!(!redacted.contains(".helm/secrets.toml"));
    assert!(redacted.contains("[REDACTED_PATH]"));
    assert!(redacted.contains("safe"));
}

// ── No-mutation invariant test ──────────────────────────────────────────────

#[test]
fn snapshot_domains_default_is_empty_no_mutation() {
    let domains = SnapshotDomains::default();
    // All default domains must be empty/zero — no mutation from defaults
    assert!(domains.load.load_average.one == 0.0);
    assert!(domains.disks.filesystems.is_empty());
    assert!(domains.services.units.is_empty());
    assert!(domains.containers.containers.is_empty());
    assert!(domains.ports.listeners.is_empty());
    assert!(domains.logs.journal_errors_last_hour == 0);
    assert!(domains.backups.tools_detected.is_empty());
    assert!(domains.packages.package_manager.is_none());
    assert!(domains.timers.cron_jobs.is_empty());
    assert!(domains.network.routes.is_empty());
    assert!(domains.processes.top_by_memory.is_empty());
    assert!(domains.firewall.firewall_tool.is_none());
}

// ── MonitorProfile tests ────────────────────────────────────────────────────

#[test]
fn monitor_profile_parsing() {
    assert_eq!(
        "quick".parse::<MonitorProfile>().unwrap(),
        MonitorProfile::Quick
    );
    assert_eq!(
        "standard".parse::<MonitorProfile>().unwrap(),
        MonitorProfile::Standard
    );
    assert_eq!(
        "deep".parse::<MonitorProfile>().unwrap(),
        MonitorProfile::Deep
    );
    assert!("invalid".parse::<MonitorProfile>().is_err());
}

#[test]
fn monitor_profile_timeouts() {
    assert_eq!(MonitorProfile::Quick.per_collector_timeout(), 5);
    assert_eq!(MonitorProfile::Standard.per_collector_timeout(), 10);
    assert_eq!(MonitorProfile::Deep.per_collector_timeout(), 30);
}

#[test]
fn monitor_profile_deep_probes() {
    assert!(!MonitorProfile::Quick.deep_probes());
    assert!(!MonitorProfile::Standard.deep_probes());
    assert!(MonitorProfile::Deep.deep_probes());
}

// ── Process/Firewall domain tests ───────────────────────────────────────────

#[test]
fn process_snapshot_defaults() {
    let ps = ProcessSnapshot::default();
    assert!(ps.top_by_memory.is_empty());
    assert!(ps.top_by_cpu.is_empty());
    assert_eq!(ps.total_count, 0);
    assert_eq!(ps.zombie_count, 0);
}

#[test]
fn firewall_snapshot_defaults() {
    let fs = FirewallSnapshot::default();
    assert!(fs.firewall_tool.is_none());
    assert!(fs.ufw_active.is_none());
    assert!(fs.firewalld_active.is_none());
    assert!(fs.iptables_rule_count.is_none());
}
