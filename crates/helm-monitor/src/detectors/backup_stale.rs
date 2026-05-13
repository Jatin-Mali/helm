//! Stale backup detection (no backup tools found, or tools exist but no backup evidence).

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct StaleBackupDetector;

impl Detector for StaleBackupDetector {
    fn id(&self) -> &'static str {
        "stale-backup"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Backups
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        if snapshot.domains.backups.tools_detected.is_empty() {
            return vec![
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    "No backup tools detected on this system",
                    Severity::Warning,
                    Confidence::High,
                    MonitorDomain::Backups,
                )
                .with_evidence(
                    "backups.tools_detected",
                    "empty",
                    "No restic, borg, rsync, or tar tools found",
                )
                .with_impact("System has no detectable backup strategy — data loss risk")
                .with_read_only_check("which restic borg rsync tar")
                .with_missing_data(
                    "Backup schedule, latest backup timestamp, restore test evidence",
                ),
            ];
        }
        vec![]
    }
}
