//! High CPU load.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
    thresholds::DetectorThresholdOverride,
};

pub struct HighLoadDetector {
    warning_ratio: f64,
    critical_ratio: f64,
}

impl HighLoadDetector {
    pub fn from_thresholds(override_cfg: Option<&DetectorThresholdOverride>) -> Self {
        let defaults = Self::default();
        let warning_ratio = override_cfg
            .and_then(|cfg| cfg.warning)
            .unwrap_or(defaults.warning_ratio);
        let critical_ratio = override_cfg
            .and_then(|cfg| cfg.critical)
            .unwrap_or(defaults.critical_ratio)
            .max(warning_ratio);
        Self {
            warning_ratio,
            critical_ratio,
        }
    }
}

impl Default for HighLoadDetector {
    fn default() -> Self {
        Self {
            warning_ratio: 1.5,
            critical_ratio: 3.0,
        }
    }
}

impl Detector for HighLoadDetector {
    fn id(&self) -> &'static str {
        "high-load"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Load
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let cores = snapshot.domains.load.cpu_logical_count.max(1) as f64;
        let la = snapshot.domains.load.load_average;

        // Check if load average exceeds CPU cores by significant margin
        if la.fifteen > cores * self.warning_ratio {
            let severity = if la.fifteen > cores * self.critical_ratio {
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

#[cfg(test)]
mod tests {
    use super::HighLoadDetector;
    use crate::{
        detectors::Detector,
        findings::Severity,
        snapshot::{HostIdentity, MonitorProfile, SnapshotDomains, SystemSnapshot},
        thresholds::DetectorThresholdOverride,
    };

    #[test]
    fn supports_threshold_overrides() {
        let mut domains = SnapshotDomains::default();
        domains.load.cpu_logical_count = 4;
        domains.load.load_average.fifteen = 8.0;
        domains.load.load_average.one = 6.0;
        domains.load.load_average.five = 7.0;
        let snapshot = SystemSnapshot {
            id: "test".into(),
            host: HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: MonitorProfile::Standard,
            domains,
            collector_errors: Vec::new(),
            redaction_version: "test".into(),
        };

        let findings = HighLoadDetector::from_thresholds(Some(&DetectorThresholdOverride {
            info: None,
            warning: Some(1.0),
            critical: Some(2.0),
        }))
        .detect(&snapshot, None);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
    }
}
