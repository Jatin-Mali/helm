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
                if d.health.as_deref() == Some("FAIL") {
                    Some(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            &d.device,
                            &format!("SMART reports FAIL for {}", d.device),
                            Severity::Critical,
                            Confidence::High,
                            MonitorDomain::Disks,
                        )
                        .with_evidence(
                            &format!("disks.smart_devices[{}].health", d.device),
                            "FAIL",
                            "SMART self-assessment reports failure",
                        )
                        .with_impact(format!(
                            "Device {} is likely to fail. Replace immediately.",
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
