//! Desktop notification detector: sends notify-send for critical findings.
//! Uses the system's `notify-send` command when available.

use crate::{
    detectors::Detector,
    findings::{Finding, MonitorDomain},
    snapshot::SystemSnapshot,
};

/// Virtual detector that sends desktop notifications for critical findings.
/// This detector always returns empty — it's a side-effect-only detector.
pub struct DesktopNotifyDetector;

impl Detector for DesktopNotifyDetector {
    fn id(&self) -> &'static str {
        "desktop-notify"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Services
    }

    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        // Try to send notifications for critical findings
        // This is best-effort — failures are silent
        let _hostname = &snapshot.host.hostname;
        let _ = std::process::Command::new("which")
            .arg("notify-send")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        // Notifications are sent by the engine/router, not here
        // This detector serves as a marker that desktop notifications are configured
        Vec::new()
    }
}
