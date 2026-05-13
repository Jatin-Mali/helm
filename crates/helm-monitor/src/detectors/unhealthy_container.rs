//! Unhealthy container detection.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct UnhealthyContainerDetector;

impl Detector for UnhealthyContainerDetector {
    fn id(&self) -> &'static str {
        "unhealthy-container"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Containers
    }
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        snapshot
            .domains
            .containers
            .containers
            .iter()
            .filter_map(|c| {
                let is_unhealthy = c.health.as_deref() == Some("unhealthy");
                let is_exited = c.status.contains("Exited");
                let is_restarting = c.status.contains("Restarting");
                if !is_unhealthy && !is_exited && !is_restarting {
                    return None;
                }
                let (severity, title) = if is_unhealthy {
                    (
                        Severity::Warning,
                        format!("{} health check is unhealthy", c.name),
                    )
                } else if is_restarting {
                    (Severity::Warning, format!("{} is in restart loop", c.name))
                } else {
                    (
                        Severity::Info,
                        format!("{} has exited (status: {})", c.name, c.status),
                    )
                };
                Some(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        &c.name,
                        &title,
                        severity,
                        Confidence::High,
                        MonitorDomain::Containers,
                    )
                    .with_evidence(
                        &format!("containers.containers[{}].status", c.name),
                        &c.status,
                        "Container is not running healthily",
                    )
                    .with_impact(format!(
                        "Container {} is not serving traffic correctly",
                        c.name
                    ))
                    .with_read_only_check(format!("docker logs --tail 50 {}", c.name)),
                )
            })
            .collect()
    }
}
