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
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let oom_lines: Vec<&str> = snapshot
            .domains
            .logs
            .kernel_errors
            .iter()
            .filter(|l| {
                l.contains("Out of memory")
                    || l.contains("oom-kill")
                    || l.contains("Killed process")
            })
            .map(|s| s.as_str())
            .collect();
        let count = oom_lines.len() as u64;

        if count == 0 {
            return vec![];
        }

        vec![
            Finding::new(
                &snapshot.id,
                self.id(),
                "system",
                &format!("OOM killer invoked {count} time(s) in the last hour"),
                Severity::Critical,
                Confidence::High,
                MonitorDomain::Load,
            )
            .with_evidence(
                "logs.kernel_errors",
                &count.to_string(),
                &format!("{count} OOM trace(s) in kernel log from the last 1-hour window"),
            )
            .with_impact("Critical processes may have been terminated. Investigate memory usage.")
            .with_read_only_check("dmesg | grep -i 'out of memory' | tail -20"),
        ]
    }
}
