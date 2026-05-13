//! Journal error burst detection.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct JournalErrorBurstDetector;

impl Detector for JournalErrorBurstDetector {
    fn id(&self) -> &'static str {
        "journal-errors"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Logs
    }
    fn detect(&self, snapshot: &SystemSnapshot, previous: Option<&SystemSnapshot>) -> Vec<Finding> {
        let mut findings = Vec::new();
        let count = snapshot.domains.logs.journal_errors_last_hour;

        // Get baseline error count from previous snapshot for drift detection
        let prev_count = previous
            .map(|p| p.domains.logs.journal_errors_last_hour)
            .unwrap_or(0);
        let has_baseline = previous.is_some();

        if count >= 100 {
            let severity = if has_baseline && count > prev_count * 2 {
                Severity::Critical
            } else {
                Severity::Warning
            };
            let base_note = if has_baseline {
                format!(" (previous: {prev_count}/h)")
            } else {
                String::new()
            };
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    &format!(
                        "{count} journal errors in the last hour{base_note} — error burst detected",
                    ),
                    severity,
                    if has_baseline {
                        Confidence::High
                    } else {
                        Confidence::Medium
                    },
                    MonitorDomain::Logs,
                )
                .with_evidence(
                    "logs.journal_errors_last_hour",
                    &count.to_string(),
                    "Error rate exceeds normal threshold",
                )
                .with_impact("High error rate indicates system or application problems")
                .with_read_only_check("journalctl -p err -n 50 --no-pager"),
            );
        } else if count >= 20
            && previous.is_some_and(|p| count > p.domains.logs.journal_errors_last_hour * 2)
        {
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    &format!(
                        "{count} journal errors in the last hour (was {prev_count}) — elevated error rate",
                    ),
                    Severity::Info,
                    Confidence::Medium,
                    MonitorDomain::Logs,
                )
                .with_evidence(
                    "logs.journal_errors_last_hour",
                    &count.to_string(),
                    "Error count has doubled compared to previous snapshot",
                )
                .with_impact("Monitor for recurring error patterns")
                .with_read_only_check("journalctl -p err --since '1 hour ago' --no-pager"),
            );
        } else if count >= 20 && !has_baseline {
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    &format!("{count} journal errors in the last hour — elevated error rate"),
                    Severity::Info,
                    Confidence::Low,
                    MonitorDomain::Logs,
                )
                .with_evidence(
                    "logs.journal_errors_last_hour",
                    &count.to_string(),
                    "Error count is above baseline threshold but below known prior levels",
                )
                .with_impact("Monitor for recurring error patterns")
                .with_read_only_check("journalctl -p err --since '1 hour ago' --no-pager"),
            );
        }

        if let Some(rate) = snapshot.domains.logs.error_rate_per_minute {
            if rate >= 10.0 {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        "system",
                        &format!("Error rate: {rate:.1}/min — sustained error burst"),
                        Severity::Warning,
                        Confidence::High,
                        MonitorDomain::Logs,
                    )
                    .with_evidence(
                        "logs.error_rate_per_minute",
                        &format!("{rate:.1}"),
                        "Error rate per minute exceeds safe threshold",
                    )
                    .with_impact("Sustained error rate indicates active failure mode"),
                );
            }
        }

        if !snapshot.domains.logs.auth_failures.is_empty() {
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "auth",
                    &format!(
                        "{} authentication failures detected",
                        snapshot.domains.logs.auth_failures.len()
                    ),
                    Severity::Warning,
                    Confidence::Medium,
                    MonitorDomain::Logs,
                )
                .with_evidence(
                    "logs.auth_failures",
                    &snapshot.domains.logs.auth_failures.len().to_string(),
                    "Failed authentication attempts may indicate brute force or misconfiguration",
                )
                .with_impact("Unauthorized access attempts may be in progress")
                .with_read_only_check(
                    "journalctl -u sshd --since '1 hour ago' --no-pager | grep Failed | tail -20",
                ),
            );
        }

        findings
    }
}
