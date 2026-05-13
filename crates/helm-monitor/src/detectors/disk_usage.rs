//! Root filesystem usage high (>90%).

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct DiskUsageDetector;

impl Detector for DiskUsageDetector {
    fn id(&self) -> &'static str {
        "disk-usage"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Disks
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();
        for fs in &snapshot.domains.disks.filesystems {
            if fs.total_bytes == 0 {
                continue;
            }
            let usage_pct = (fs.used_bytes as f64 / fs.total_bytes as f64) * 100.0;
            let severity = if usage_pct >= 95.0 {
                Severity::Critical
            } else if usage_pct >= 90.0 {
                Severity::Warning
            } else if usage_pct >= 80.0 {
                Severity::Info
            } else {
                continue;
            };
            let title = format!(
                "{} usage is {:.0}% ({})",
                fs.mount_point,
                usage_pct,
                severity.as_str()
            );
            let f = Finding::new(
                &snapshot.id,
                self.id(),
                &fs.mount_point,
                &title,
                severity,
                Confidence::High,
                MonitorDomain::Disks,
            )
            .with_evidence(
                &format!("disks.filesystems[{}].used_bytes", fs.mount_point),
                &format!(
                    "{} / {} ({:.0}%)",
                    human_bytes(fs.used_bytes),
                    human_bytes(fs.total_bytes),
                    usage_pct
                ),
                "Disk usage exceeds threshold",
            )
            .with_impact(format!(
                "Filesystem {} may become full, causing writes to fail",
                fs.mount_point
            ))
            .with_read_only_check(format!("du -sh {}/* | sort -rh | head -20", fs.mount_point));
            findings.push(f);
        }
        findings
    }
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}G", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}M", bytes as f64 / 1_048_576.0)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::*;

    #[test]
    fn detects_high_disk_usage() {
        let snap = snapshot_with_fs("/", 500_000_000_000, 475_000_000_000);
        let findings = DiskUsageDetector.detect(&snap, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert!(findings[0].title.contains("95%"));
    }

    #[test]
    fn skips_normal_usage() {
        let snap = snapshot_with_fs("/", 500_000_000_000, 200_000_000_000);
        let findings = DiskUsageDetector.detect(&snap, None);
        assert!(findings.is_empty());
    }

    fn snapshot_with_fs(mount: &str, total: u64, used: u64) -> SystemSnapshot {
        let mut domains = SnapshotDomains::default();
        domains.disks.filesystems.push(FilesystemEntry {
            device: "/dev/sda1".into(),
            mount_point: mount.into(),
            fs_type: "ext4".into(),
            total_bytes: total,
            used_bytes: used,
            available_bytes: total - used,
        });
        SystemSnapshot {
            id: "test".into(),
            host: HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: MonitorProfile::Standard,
            domains,
            collector_errors: vec![],
            redaction_version: "0.1.0".into(),
        }
    }
}
