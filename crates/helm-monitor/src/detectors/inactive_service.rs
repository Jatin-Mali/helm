//! Enabled but inactive services.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct EnabledInactiveServiceDetector;

impl Detector for EnabledInactiveServiceDetector {
    fn id(&self) -> &'static str {
        "inactive-service"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Services
    }
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        snapshot
            .domains
            .services
            .units
            .iter()
            .filter(|u| u.load == "loaded" && u.active == "inactive" && u.sub != "dead")
            .map(|u| {
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    &u.name,
                    &format!("{} is loaded but inactive", u.name),
                    Severity::Info,
                    Confidence::Medium,
                    MonitorDomain::Services,
                )
                .with_evidence(
                    &format!("services.units[{}].active", u.name),
                    &u.active,
                    "Unit loaded but not running",
                )
                .with_assumption("Service may have been stopped intentionally or failed to start")
                .with_impact("Service is not providing its intended functionality")
                .with_read_only_check(format!("systemctl status {}", u.name))
            })
            .collect()
    }
}
