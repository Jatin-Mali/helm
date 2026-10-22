//! SMART health warnings.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct SmartHealthDetector;

impl Detector for SmartHealthDetector {
    fn id(&self) -> &'static str {
        "smart-health"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Disks
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        if !snapshot.domains.disks.smart_available {
            return vec![
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    "SMART data unavailable — cannot check disk health",
                    Severity::Info,
                    Confidence::Medium,
                    MonitorDomain::Disks,
                )
                .with_evidence(
                    "disks.smart_available",
                    "false",
                    "smartctl not found or not available",
                )
                .with_assumption("Disk health cannot be verified without SMART")
                .with_missing_data("SMART health attributes for all block devices")
                .with_impact("Undetected disk failures may cause data loss")
                .with_read_only_check("smartctl -a /dev/sda"),
            ];
        }
        snapshot
            .domains
            .disks
            .smart_devices
            .iter()
            .filter_map(|d| {
                if let Some(health) = d
                    .health
                    .as_deref()
                    .filter(|health| *health == "FAIL" || health.starts_with("WARN:"))
                {
                    let failed = health == "FAIL";
                    Some(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            &d.device,
                            &format!("SMART reports {health} for {}", d.device),
                            if failed {
                                Severity::Critical
                            } else {
                                Severity::Warning
                            },
                            Confidence::High,
                            MonitorDomain::Disks,
                        )
                        .with_evidence(
                            &format!("disks.smart_devices[{}].health", d.device),
                            health,
                            "SMART self-assessment or history flags require attention",
                        )
                        .with_impact(format!(
                            "Device {} requires investigation and a verified backup.",
                            d.device
                        ))
                        .with_read_only_check(format!("smartctl -a {}", d.device)),
                    )
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_health_and_history_flags_emit_distinct_severities() {
        let mut snapshot = SystemSnapshot::new(
            "fixture".into(),
            Default::default(),
            crate::MonitorProfile::Quick,
            Default::default(),
        );
        snapshot.domains.disks.smart_available = true;
        for (health, severity) in [
            ("FAIL", Severity::Critical),
            (
                "WARN: SMART history or error log flags (64)",
                Severity::Warning,
            ),
        ] {
            snapshot.domains.disks.smart_devices = vec![crate::snapshot::SmartDevice {
                device: "/dev/fixture".into(),
                model: None,
                health: Some(health.into()),
                temperature_celsius: None,
            }];
            let findings = SmartHealthDetector.detect(&snapshot, None);
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].severity, severity);
            assert!(findings[0].title.contains(health));
        }
    }
}
