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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::*;

    fn snapshot_with_kernel_errors(errors: Vec<String>) -> SystemSnapshot {
        let mut domains = SnapshotDomains::default();
        domains.logs.kernel_errors = errors;
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
    fn detects_oom_killer_events() {
        let snap = snapshot_with_kernel_errors(vec!["Out of memory: killed process".into()]);
        let findings = OomKillerDetector.detect(&snap, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[0].confidence, Confidence::High);
        assert!(findings[0].title.contains("1 time"));
    }

    #[test]
    fn detects_multiple_oom_events() {
        let snap = snapshot_with_kernel_errors(vec![
            "Out of memory: killed process 123 (nginx)".into(),
            "oom-kill: constraint=CONSTRAINT_MEMCG".into(),
            "Killed process 456 (postgres)".into(),
        ]);
        let findings = OomKillerDetector.detect(&snap, None);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].title.contains("3 time"));
    }

    #[test]
    fn skips_when_no_oom_events() {
        let snap = snapshot_with_kernel_errors(vec![]);
        let findings = OomKillerDetector.detect(&snap, None);
        assert!(findings.is_empty());
    }
}
