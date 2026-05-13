//! Service restart loop detection (>5 restarts recently, heuristic via unit state).

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct ServiceRestartLoopDetector;

impl Detector for ServiceRestartLoopDetector {
    fn id(&self) -> &'static str {
        "restart-loop"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Services
    }
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        // Detect via auto-restart sub-state in systemd units
        snapshot
            .domains
            .services
            .units
            .iter()
            .filter(|u| u.sub == "auto-restart" || u.sub.contains("restart"))
            .map(|u| {
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    &u.name,
                    &format!("{} is restarting repeatedly (sub: {})", u.name, u.sub),
                    Severity::Warning,
                    Confidence::Medium,
                    MonitorDomain::Services,
                )
                .with_evidence(
                    &format!("services.units[{}].sub", u.name),
                    &u.sub,
                    "Unit is in a restart loop state",
                )
                .with_assumption(
                    "Restart loop may indicate configuration error or resource exhaustion",
                )
                .with_impact("Service is unstable and may be unavailable intermittently")
                .with_read_only_check(format!(
                    "journalctl -u {} -n 100 --no-pager | grep -i 'fail\\|error\\|restart'",
                    u.name
                ))
            })
            .collect()
    }
}
