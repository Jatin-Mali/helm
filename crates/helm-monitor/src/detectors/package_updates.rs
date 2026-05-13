//! Security updates available detection.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct SecurityUpdatesDetector;

impl Detector for SecurityUpdatesDetector {
    fn id(&self) -> &'static str {
        "security-updates"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Packages
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        if let Some(count) = snapshot.domains.packages.security_count {
            if count > 0 {
                return vec![Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    &format!("{count} security update(s) available"),
                    Severity::Warning,
                    Confidence::High,
                    MonitorDomain::Packages,
                )
                .with_evidence(
                    "packages.security_count",
                    &count.to_string(),
                    "Security updates are available for installed packages",
                )
                .with_impact(
                    "System may be vulnerable to known security issues. Apply updates promptly.",
                )
                .with_read_only_check("apt list --upgradable 2>/dev/null | grep -i security")
                .with_assumption("Security update listing from package manager is accurate")];
            }
        }
        // If no security info but there are upgradable packages, that's info-level
        if let Some(count) = snapshot.domains.packages.upgradable_count {
            if count > 50 {
                return vec![
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        "system",
                        &format!("{count} upgradable packages — system may be stale"),
                        Severity::Info,
                        Confidence::High,
                        MonitorDomain::Packages,
                    )
                    .with_evidence(
                        "packages.upgradable_count",
                        &count.to_string(),
                        "Large number of packages awaiting update",
                    )
                    .with_impact("System maintenance is overdue")
                    .with_read_only_check("apt list --upgradable 2>/dev/null | head -20"),
                ];
            }
        }
        vec![]
    }
}
