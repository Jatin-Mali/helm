//! Journal error burst and log spike detection.

use std::collections::HashMap;

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    log_analysis::log_pattern_ids_new_since_previous,
    snapshot::{JournalPattern, LogSeverity, SystemSnapshot},
    thresholds::DetectorThresholdOverride,
};

pub struct JournalErrorBurstDetector {
    spike_info_threshold: u64,
    spike_warning_threshold: u64,
    spike_critical_threshold: u64,
}

impl JournalErrorBurstDetector {
    pub fn from_thresholds(override_cfg: Option<&DetectorThresholdOverride>) -> Self {
        Self {
            spike_info_threshold: override_cfg.and_then(|cfg| cfg.info).unwrap_or(3.0).round()
                as u64,
            spike_warning_threshold: override_cfg
                .and_then(|cfg| cfg.warning)
                .unwrap_or(5.0)
                .round() as u64,
            spike_critical_threshold: override_cfg
                .and_then(|cfg| cfg.critical)
                .unwrap_or(12.0)
                .round() as u64,
        }
    }
}

impl Default for JournalErrorBurstDetector {
    fn default() -> Self {
        Self::from_thresholds(None)
    }
}

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

        if let Some(rate) = snapshot.domains.logs.error_rate_per_minute
            && rate >= 10.0
        {
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

        findings.extend(self.promote_log_spikes(snapshot, previous));
        findings
    }
}

impl JournalErrorBurstDetector {
    fn promote_log_spikes(
        &self,
        snapshot: &SystemSnapshot,
        previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let previous_patterns = previous
            .map(|prev| prev.domains.logs.patterns.as_slice())
            .unwrap_or(&[]);
        let previous_counts = previous_patterns
            .iter()
            .map(|pattern| (pattern.pattern_id.as_str(), pattern.occurrence_count))
            .collect::<HashMap<_, _>>();
        let new_pattern_ids =
            log_pattern_ids_new_since_previous(&snapshot.domains.logs.patterns, previous_patterns);

        snapshot
            .domains
            .logs
            .patterns
            .iter()
            .filter_map(|pattern| {
                self.promoted_pattern_finding(
                    &snapshot.id,
                    pattern,
                    &previous_counts,
                    &new_pattern_ids,
                )
            })
            .collect()
    }

    fn promoted_pattern_finding(
        &self,
        snapshot_id: &str,
        pattern: &JournalPattern,
        previous_counts: &HashMap<&str, u64>,
        new_pattern_ids: &std::collections::HashSet<String>,
    ) -> Option<Finding> {
        if pattern.occurrence_count < self.spike_warning_threshold {
            return None;
        }

        let severity = if pattern.occurrence_count >= self.spike_critical_threshold
            || (pattern.severity == LogSeverity::Critical
                && pattern.occurrence_count >= self.spike_warning_threshold)
        {
            Severity::Critical
        } else {
            Severity::Warning
        };
        let previous_count = previous_counts
            .get(pattern.pattern_id.as_str())
            .copied()
            .unwrap_or_default();
        let title_budget = 80usize
            .saturating_sub(pattern.unit.chars().count())
            .saturating_sub(pattern.occurrence_count.to_string().chars().count())
            .saturating_sub(25)
            .max(16);
        let sample = truncate_message(&pattern.sample_message, title_budget);
        let title = format!(
            "{} repeated log pattern x{}: {}",
            pattern.unit, pattern.occurrence_count, sample
        );

        let mut finding = Finding::new(
            snapshot_id,
            self.id(),
            &pattern.unit,
            &title,
            severity,
            Confidence::High,
            MonitorDomain::Logs,
        )
        .with_evidence(
            "logs.patterns.occurrence_count",
            &pattern.occurrence_count.to_string(),
            "Pattern count crossed the configured spike threshold",
        )
        .with_evidence(
            "logs.patterns.normalized_message",
            &pattern.normalized_message,
            "Normalized journal pattern used for grouping",
        )
        .with_evidence(
            "logs.patterns.last_seen_unix_ms",
            &pattern.last_seen_unix_ms.to_string(),
            "Most recent matching journal entry",
        )
        .with_impact("Repeated journal errors suggest an active fault in this unit")
        .with_read_only_check(format!(
            "journalctl -u {} --since '1 hour ago' --no-pager",
            pattern.unit
        ));

        if previous_count >= self.spike_info_threshold {
            finding = finding.with_assumption(format!(
                "Previous cycle already saw this pattern {} times",
                previous_count
            ));
        }
        if new_pattern_ids.contains(&pattern.pattern_id) {
            finding = finding.with_assumption("Pattern did not appear in the previous snapshot");
        }

        Some(finding)
    }
}

fn truncate_message(message: &str, max_chars: usize) -> String {
    let mut truncated = message.chars().take(max_chars).collect::<String>();
    if message.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{JournalContextLine, LogSnapshot, SnapshotDomains, SystemSnapshot};
    use chrono::Utc;
    use proptest::prelude::*;

    fn snapshot_with_patterns(patterns: Vec<JournalPattern>) -> SystemSnapshot {
        let domains = SnapshotDomains {
            logs: LogSnapshot {
                journal_errors_last_hour: 0,
                kernel_errors: Vec::new(),
                auth_failures: Vec::new(),
                error_rate_per_minute: None,
                patterns,
            },
            ..SnapshotDomains::default()
        };
        let mut snapshot = SystemSnapshot::new(
            "snap-1".into(),
            crate::snapshot::HostIdentity {
                hostname: "testbox".into(),
                kernel_name: "Linux".into(),
                kernel_release: "6.8.0".into(),
                machine: "x86_64".into(),
                os_pretty_name: Some("TestOS".into()),
                os_id: Some("test".into()),
                os_version_id: Some("1".into()),
                uptime_seconds: 42,
            },
            crate::snapshot::MonitorProfile::Quick,
            domains,
        );
        snapshot.collected_at = Utc::now();
        snapshot
    }

    fn pattern(unit: &str, occurrences: u64, severity: LogSeverity) -> JournalPattern {
        JournalPattern {
            pattern_id: format!("{unit}-{occurrences}"),
            unit: unit.into(),
            severity,
            normalized_message: "failed password for invalid user <n> from <ip>".into(),
            sample_message: "Failed password for invalid user deploy from 10.0.0.5".into(),
            occurrence_count: occurrences,
            first_seen_unix_ms: 1_000,
            last_seen_unix_ms: 2_000,
            context_lines: vec![JournalContextLine {
                offset: 0,
                timestamp: "12:00:00".into(),
                unit: unit.into(),
                severity,
                message: "Failed password".into(),
                raw: "12:00:00 ERR sshd Failed password".into(),
            }],
        }
    }

    proptest! {
        #[test]
        fn property_30_log_spike_promotion(
            warning in 2u64..10,
            critical in 11u64..25,
            occurrences in 0u64..30,
        ) {
            prop_assume!(critical > warning);
            let detector = JournalErrorBurstDetector {
                spike_info_threshold: 1,
                spike_warning_threshold: warning,
                spike_critical_threshold: critical,
            };
            let snapshot = snapshot_with_patterns(vec![pattern("sshd.service", occurrences, LogSeverity::Error)]);
            let findings = detector.detect(&snapshot, None);
            let promoted = findings.iter().find(|finding| finding.affected_resource == "sshd.service");

            if occurrences >= critical {
                let finding = promoted.expect("critical spike should promote");
                prop_assert_eq!(finding.severity, Severity::Critical);
            } else if occurrences >= warning {
                let finding = promoted.expect("warning spike should promote");
                prop_assert_eq!(finding.severity, Severity::Warning);
            } else {
                prop_assert!(promoted.is_none());
            }
        }
    }
}
