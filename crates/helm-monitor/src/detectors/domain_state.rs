//! Libvirt VM domain state detection — flags stopped, paused, or crashed VMs.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct DomainStateDetector;

impl Detector for DomainStateDetector {
    fn id(&self) -> &'static str {
        "domain-state"
    }

    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Libvirt
    }

    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        snapshot
            .domains
            .libvirt
            .domains
            .iter()
            .filter_map(|domain| match domain.state.as_str() {
                "shut off" => Some(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        &domain.name,
                        "VM domain is shut off",
                        Severity::Critical,
                        Confidence::High,
                        MonitorDomain::Libvirt,
                    )
                    .with_evidence(
                        &format!("libvirt.domains[{}].state", domain.name),
                        "shut off",
                        "VM is not running",
                    )
                    .with_impact(format!(
                        "VM {} is powered off — services running on it are unavailable",
                        domain.name
                    ))
                    .with_read_only_check(format!("virsh domstate {}", domain.name)),
                ),
                "paused" => Some(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        &domain.name,
                        "VM domain is paused",
                        Severity::Warning,
                        Confidence::High,
                        MonitorDomain::Libvirt,
                    )
                    .with_evidence(
                        &format!("libvirt.domains[{}].state", domain.name),
                        "paused",
                        "VM is paused and not executing",
                    )
                    .with_impact(format!(
                        "VM {} is paused — workloads are frozen, may need to be resumed",
                        domain.name
                    ))
                    .with_read_only_check(format!("virsh domstate {}", domain.name)),
                ),
                "crashed" => Some(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        &domain.name,
                        "VM domain has crashed",
                        Severity::Critical,
                        Confidence::High,
                        MonitorDomain::Libvirt,
                    )
                    .with_evidence(
                        &format!("libvirt.domains[{}].state", domain.name),
                        "crashed",
                        "VM crashed unexpectedly",
                    )
                    .with_impact(format!(
                        "VM {} crashed — services are unavailable, investigate logs",
                        domain.name
                    ))
                    .with_read_only_check(format!(
                        "virsh domstate {} && journalctl -u libvirtd",
                        domain.name
                    )),
                ),
                _ => None,
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
        let detector = DomainStateDetector;
        let snapshot = make_snapshot(vec![]);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_running_domain() {
        let detector = DomainStateDetector;
        let domains = vec![VmDomain {
            name: "web-server".to_string(),
            state: "running".to_string(),
            vcpus: 4,
            memory_mb: 8192,
            snapshot_count: 0,
            age_days: None,
        }];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_shut_off_domain() {
        let detector = DomainStateDetector;
        let domains = vec![VmDomain {
            name: "db-server".to_string(),
            state: "shut off".to_string(),
            vcpus: 8,
            memory_mb: 16384,
            snapshot_count: 0,
            age_days: None,
        }];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
    }

    #[test]
    fn test_paused_domain() {
        let detector = DomainStateDetector;
        let domains = vec![VmDomain {
            name: "test-vm".to_string(),
            state: "paused".to_string(),
            vcpus: 2,
            memory_mb: 4096,
            snapshot_count: 1,
            age_days: Some(1),
        }];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn test_crashed_domain() {
        let detector = DomainStateDetector;
        let domains = vec![VmDomain {
            name: "prod-app".to_string(),
            state: "crashed".to_string(),
            vcpus: 4,
            memory_mb: 8192,
            snapshot_count: 0,
            age_days: None,
        }];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
    }

    #[test]
    fn test_mixed_domains() {
        let detector = DomainStateDetector;
        let domains = vec![
            VmDomain {
                name: "vm1".to_string(),
                state: "running".to_string(),
                vcpus: 2,
                memory_mb: 4096,
                snapshot_count: 0,
                age_days: None,
            },
            VmDomain {
                name: "vm2".to_string(),
                state: "paused".to_string(),
                vcpus: 2,
                memory_mb: 4096,
                snapshot_count: 1,
                age_days: Some(1),
            },
            VmDomain {
                name: "vm3".to_string(),
                state: "shut off".to_string(),
                vcpus: 4,
                memory_mb: 8192,
                snapshot_count: 0,
                age_days: None,
            },
            VmDomain {
                name: "vm4".to_string(),
                state: "crashed".to_string(),
                vcpus: 2,
                memory_mb: 4096,
                snapshot_count: 0,
                age_days: None,
            },
        ];
        let snapshot = make_snapshot(domains);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 3);
        assert!(findings.iter().any(|f| f.severity == Severity::Warning));
        assert!(
            findings
                .iter()
                .filter(|f| f.severity == Severity::Critical)
                .count()
                == 2
        );
    }
}
