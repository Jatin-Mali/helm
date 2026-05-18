//! Libvirt VM snapshot age detection — flags old snapshots that may indicate stale backups.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct SnapshotAgeDetector;

impl Detector for SnapshotAgeDetector {
    fn id(&self) -> &'static str {
        "snapshot-age"
    }

    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Libvirt
    }

    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let threshold_days = 7;

        snapshot
            .domains
            .libvirt
            .domains
            .iter()
            .filter_map(|domain| {
                if let Some(age_days) = domain.age_days {
                    if age_days > threshold_days {
                        let formatted_age = format!("{} days", age_days);
                        Some(
                            Finding::new(
                                &snapshot.id,
                                self.id(),
                                &domain.name,
                                &format!("VM snapshot is {} old", formatted_age),
                                Severity::Warning,
                                Confidence::High,
                                MonitorDomain::Libvirt,
                            )
                            .with_evidence(
                                &format!("libvirt.domains[{}].age_days", domain.name),
                                &age_days.to_string(),
                                "Snapshot age exceeds threshold",
                            )
                            .with_impact(format!(
                                "VM {} has a snapshot {} old — may indicate backup not taken recently",
                                domain.name, formatted_age
                            ))
                            .with_read_only_check(format!(
                                "virsh snapshot-list {} | head -5",
                                domain.name
                            )),
                        )
                    } else {
                        None
                    }
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
    use crate::collectors::libvirt::{LibvirtSnapshot, VmDomain};
    use crate::snapshot::{HostIdentity, MonitorProfile, SnapshotDomains, SystemSnapshot};

    fn make_snapshot(domains: Vec<VmDomain>) -> SystemSnapshot {
        let doms = SnapshotDomains {
            host: HostIdentity::default(),
            load: Default::default(),
            disks: Default::default(),
            services: Default::default(),
            containers: Default::default(),
            ports: Default::default(),
            logs: Default::default(),
            backups: Default::default(),
            packages: Default::default(),
            timers: Default::default(),
            network: Default::default(),
            processes: Default::default(),
            firewall: Default::default(),
            kubernetes: Default::default(),
            libvirt: LibvirtSnapshot {
                domains,
                domain_count: 0,
                available: true,
            },
            compose: Default::default(),
        };
        SystemSnapshot::new(
            "test-snapshot".to_string(),
            HostIdentity::default(),
            MonitorProfile::Standard,
            doms,
        )
    }

    #[test]
    fn test_empty_snapshot() {
        let detector = SnapshotAgeDetector;
        let snapshot = make_snapshot(vec![]);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_domain_no_snapshot() {
        let detector = SnapshotAgeDetector;
        let domains = vec![VmDomain {
            name: "vm-no-snap".to_string(),
            state: "running".to_string(),
            vcpus: 2,
            memory_mb: 4096,
            snapshot_count: 0,
            age_days: None,
        }];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_snapshot_recent() {
        let detector = SnapshotAgeDetector;
        let domains = vec![VmDomain {
            name: "vm-recent".to_string(),
            state: "running".to_string(),
            vcpus: 2,
            memory_mb: 4096,
            snapshot_count: 1,
            age_days: Some(3),
        }];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_snapshot_old() {
        let detector = SnapshotAgeDetector;
        let domains = vec![VmDomain {
            name: "vm-old-snap".to_string(),
            state: "running".to_string(),
            vcpus: 4,
            memory_mb: 8192,
            snapshot_count: 2,
            age_days: Some(14),
        }];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn test_snapshot_very_old() {
        let detector = SnapshotAgeDetector;
        let domains = vec![VmDomain {
            name: "vm-very-old".to_string(),
            state: "running".to_string(),
            vcpus: 2,
            memory_mb: 4096,
            snapshot_count: 1,
            age_days: Some(30),
        }];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn test_mixed_domains() {
        let detector = SnapshotAgeDetector;
        let domains = vec![
            VmDomain {
                name: "vm1".to_string(),
                state: "running".to_string(),
                vcpus: 2,
                memory_mb: 4096,
                snapshot_count: 1,
                age_days: Some(2),
            },
            VmDomain {
                name: "vm2".to_string(),
                state: "running".to_string(),
                vcpus: 2,
                memory_mb: 4096,
                snapshot_count: 0,
                age_days: None,
            },
            VmDomain {
                name: "vm3".to_string(),
                state: "running".to_string(),
                vcpus: 4,
                memory_mb: 8192,
                snapshot_count: 3,
                age_days: Some(10),
            },
            VmDomain {
                name: "vm4".to_string(),
                state: "running".to_string(),
                vcpus: 2,
                memory_mb: 4096,
                snapshot_count: 1,
                age_days: Some(25),
            },
        ];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 2);
    }
}
