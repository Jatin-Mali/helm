//! Restore-test gap detection: flags backup tools with no restore-test evidence.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct RestoreTestMissingDetector;

impl Detector for RestoreTestMissingDetector {
    fn id(&self) -> &'static str {
        "restore-test-missing"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Backups
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let tools = &snapshot.domains.backups.tools_detected;
        if tools.is_empty() {
            return vec![];
        }
        let without_restore: Vec<&str> = tools
            .iter()
            .filter(|t| t.restore_test_evidence.is_none())
            .map(|t| t.name.as_str())
            .collect();
        if without_restore.is_empty() {
            return vec![];
        }
        let names = without_restore.join(", ");
        vec![Finding::new(
            &snapshot.id,
            self.id(),
            "system",
            &format!(
                "No restore-test evidence for backup tools: {names}",
            ),
            Severity::Warning,
            Confidence::Medium,
            MonitorDomain::Backups,
        )
        .with_evidence(
            "backups.tools_detected[].restore_test_evidence",
            "none",
            "Backup tools found but no cache/check snapshot indicating restore tests have been run",
        )
        .with_assumption("Restore tests may have been run without leaving detectable evidence")
        .with_missing_data("Restore test logs, manual restore verification records")
        .with_impact(format!(
            "Backups for {names} may not be restorable. Untested backups are a data-loss risk."
        ))
        .with_read_only_check(format!("restic check --read-data # verify backup integrity for {names}"))]
    }
}
