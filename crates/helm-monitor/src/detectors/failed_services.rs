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
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
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
