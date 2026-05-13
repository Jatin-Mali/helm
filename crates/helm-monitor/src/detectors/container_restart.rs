//! Container restart loop (high restart count).

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct ContainerRestartLoopDetector;

impl Detector for ContainerRestartLoopDetector {
    fn id(&self) -> &'static str {
        "container-restart"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Containers
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        snapshot
            .domains
            .containers
            .containers
            .iter()
            .filter_map(|c| {
                if let Some(count) = c.restart_count {
                    if count >= 5 {
                        Some(
                            Finding::new(
                                &snapshot.id,
                                self.id(),
                                &c.name,
                                &format!("{} has restarted {} times", c.name, count),
                                Severity::Warning,
                                Confidence::Medium,
                                MonitorDomain::Containers,
                            )
                            .with_evidence(
                                &format!("containers.containers[{}].restart_count", c.name),
                                &count.to_string(),
                                "Container restart count exceeds threshold",
                            )
                            .with_impact(format!("Container {} is unstable", c.name))
                            .with_read_only_check(format!(
                                "docker inspect {} | grep -A5 RestartCount",
                                c.name
                            )),
                        )
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }
}
