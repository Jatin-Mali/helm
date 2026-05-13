//! Inode usage high (>85%).

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct InodeUsageDetector;

impl Detector for InodeUsageDetector {
    fn id(&self) -> &'static str {
        "inode-usage"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Disks
    }
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        snapshot
            .domains
            .disks
            .inodes
            .iter()
            .filter_map(|inode| {
                if inode.total == 0 {
                    return None;
                }
                let pct = (inode.used as f64 / inode.total as f64) * 100.0;
                if pct < 85.0 {
                    return None;
                }
                let severity = if pct >= 95.0 {
                    Severity::Critical
                } else {
                    Severity::Warning
                };
                Some(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        &inode.mount_point,
                        &format!("{} inode usage is {:.0}%", inode.mount_point, pct),
                        severity,
                        Confidence::High,
                        MonitorDomain::Disks,
                    )
                    .with_evidence(
                        &format!("disks.inodes[{}].used", inode.mount_point),
                        &format!("{} / {} inodes ({:.0}%)", inode.used, inode.total, pct),
                        "Inode exhaustion prevents file creation even with free space",
                    )
                    .with_impact(format!(
                        "Filesystem {} cannot create new files once inodes are exhausted",
                        inode.mount_point
                    ))
                    .with_read_only_check(format!(
                        "find {} -xdev -type f | wc -l",
                        inode.mount_point
                    )),
                )
            })
            .collect()
    }
}
