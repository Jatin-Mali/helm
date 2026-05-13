//! OOM killer event detection.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct OomKillerDetector;

impl Detector for OomKillerDetector {
    fn id(&self) -> &'static str {
        "oom-event"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Load
    }
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        // Check kernel error logs for OOM traces
        let has_oom = snapshot.domains.logs.kernel_errors.iter().any(|l| {
            l.contains("Out of memory") || l.contains("oom-kill") || l.contains("Killed process")
        });
        if has_oom {
            vec![
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    "OOM killer has been invoked recently",
                    Severity::Critical,
                    Confidence::High,
                    MonitorDomain::Load,
                )
                .with_evidence(
                    "logs.kernel_errors",
                    "OOM killer traces found in kernel log",
                    "The kernel Out-Of-Memory killer terminated processes to free memory",
                )
                .with_impact(
                    "Critical processes may have been terminated. Investigate memory usage.",
                )
                .with_read_only_check("dmesg | grep -i 'out of memory' | tail -20"),
            ]
        } else {
            vec![]
        }
    }
}
