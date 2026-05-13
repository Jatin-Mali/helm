//! High CPU load.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct HighLoadDetector;

impl Detector for HighLoadDetector {
    fn id(&self) -> &'static str {
        "high-load"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Load
    }
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        let cores = snapshot.domains.load.cpu_logical_count.max(1) as f64;
        let la = snapshot.domains.load.load_average;

        // Check if load average exceeds CPU cores by significant margin
        if la.fifteen > cores * 1.5 {
            let severity = if la.fifteen > cores * 3.0 {
                Severity::Critical
            } else {
                Severity::Warning
            };
            vec![
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    &format!(
                        "System load ({:.2}/{:.2}/{:.2}) is high for {} CPU cores",
                        la.one, la.five, la.fifteen, snapshot.domains.load.cpu_logical_count
                    ),
                    severity,
                    Confidence::High,
                    MonitorDomain::Load,
                )
                .with_evidence(
                    "load.load_average",
                    &format!("{:.2} / {:.2} / {:.2}", la.one, la.five, la.fifteen),
                    "Load average significantly exceeds available CPU cores",
                )
                .with_impact("System may be CPU-bound. Response times may degrade.")
                .with_read_only_check("ps aux --sort=-%cpu | head -15"),
            ]
        } else {
            vec![]
        }
    }
}
