//! Missing backup schedule detection.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct MissingBackupScheduleDetector;

impl Detector for MissingBackupScheduleDetector {
    fn id(&self) -> &'static str {
        "backup-schedule"
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
        // Check if there are backup tools but no cron jobs or systemd timers for them
        let has_backup_timer = snapshot
            .domains
            .services
            .timers
            .iter()
            .any(|t| tools.iter().any(|tool| t.activates.contains(&tool.name)));
        let has_backup_cron = snapshot
            .domains
            .timers
            .cron_jobs
            .iter()
            .any(|j| tools.iter().any(|tool| j.path.contains(&tool.name)));
        if has_backup_timer || has_backup_cron {
            return vec![];
        }
        vec![
            Finding::new(
                &snapshot.id,
                self.id(),
                "system",
                "Backup tools found but no scheduled backup timers detected",
                Severity::Warning,
                Confidence::Medium,
                MonitorDomain::Backups,
            )
            .with_evidence(
                "backups.tools_detected",
                &format!("{} tools, 0 scheduled", tools.len()),
                "Backup tools installed but may not run automatically",
            )
            .with_assumption("Backups may run via external orchestration not visible here")
            .with_missing_data("External backup scheduler status, manual backup execution history")
            .with_impact("Backups may not be running on schedule — verify manually"),
        ]
    }
}
