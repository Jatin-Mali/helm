//! Filesystem remounted read-only.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct FilesystemReadOnlyDetector;

impl Detector for FilesystemReadOnlyDetector {
    fn id(&self) -> &'static str {
        "fs-readonly"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Disks
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        snapshot
            .domains
            .disks
            .mounts
            .iter()
            .filter_map(|m| {
                if m.options.split(',').any(|option| option == "ro")
                    && !m.target.starts_with("/sys")
                {
                    Some(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            &m.target,
                            &format!("{} is mounted read-only", m.target),
                            Severity::Warning,
                            Confidence::High,
                            MonitorDomain::Disks,
                        )
                        .with_evidence(
                            &format!("disks.mounts[{}].options", m.target),
                            &m.options,
                            "Filesystem is mounted read-only — writes will fail",
                        )
                        .with_impact(format!("Filesystem {} cannot accept writes", m.target))
                        .with_read_only_check(format!("findmnt {}", m.target)),
                    )
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{MonitorProfile, MountEntry};

    #[test]
    fn read_only_mount_option_is_exact_not_a_remount_policy() {
        let mut snapshot = SystemSnapshot::new(
            "fixture".into(),
            Default::default(),
            MonitorProfile::Quick,
            Default::default(),
        );
        for (options, expected) in [("rw,relatime,errors=remount-ro", 0), ("ro,relatime", 1)] {
            snapshot.domains.disks.mounts = vec![MountEntry {
                source: "/dev/test".into(),
                target: "/".into(),
                fs_type: "ext4".into(),
                options: options.into(),
            }];
            assert_eq!(
                FilesystemReadOnlyDetector.detect(&snapshot, None).len(),
                expected,
                "{options}"
            );
        }
    }
}
