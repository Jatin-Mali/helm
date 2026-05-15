//! Failed systemd units.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct FailedServicesDetector;

impl Detector for FailedServicesDetector {
    fn id(&self) -> &'static str {
        "failed-services"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Services
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        snapshot
            .domains
            .services
            .failed_units
            .iter()
            .map(|u| {
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    &u.name,
                    &format!("{} has failed ({})", u.name, u.description),
                    Severity::Warning,
                    Confidence::High,
                    MonitorDomain::Services,
                )
                .with_evidence(
                    &format!("services.failed_units[{}].sub", u.name),
                    &u.sub,
                    "Unit is in failed state",
                )
                .with_impact("Service is not running, dependent services may be affected")
                .with_read_only_check(format!(
                    "systemctl status {} && journalctl -u {} -n 50 --no-pager",
                    u.name, u.name
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::*;

    fn snapshot_with_failed_units(units: Vec<FailedUnit>) -> SystemSnapshot {
        let mut domains = SnapshotDomains::default();
        domains.services.failed_units = units;
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

    #[test]
    fn detects_failed_services() {
        let snap = snapshot_with_failed_units(vec![FailedUnit {
            name: "nginx".into(),
            description: "nginx web server".into(),
            loaded: "loaded".into(),
            active: "failed".into(),
            sub: "failed".into(),
        }]);
        let findings = FailedServicesDetector.detect(&snap, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert_eq!(findings[0].affected_resource, "nginx");
    }

    #[test]
    fn multiple_failed_services() {
        let snap = snapshot_with_failed_units(vec![
            FailedUnit {
                name: "nginx".into(),
                description: "nginx web server".into(),
                loaded: "loaded".into(),
                active: "failed".into(),
                sub: "failed".into(),
            },
            FailedUnit {
                name: "postgresql".into(),
                description: "PostgreSQL database".into(),
                loaded: "loaded".into(),
                active: "failed".into(),
                sub: "failed".into(),
            },
        ]);
        let findings = FailedServicesDetector.detect(&snap, None);
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn skips_when_no_failed_services() {
        let snap = snapshot_with_failed_units(vec![]);
        let findings = FailedServicesDetector.detect(&snap, None);
        assert!(findings.is_empty());
    }
}
