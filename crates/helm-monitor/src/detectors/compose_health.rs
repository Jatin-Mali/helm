//! Docker Compose project health detection — flags unhealthy or degraded projects.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct ComposeHealthDetector;

impl Detector for ComposeHealthDetector {
    fn id(&self) -> &'static str {
        "compose-health"
    }

    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Compose
    }

    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        snapshot
            .domains
            .compose
            .projects
            .iter()
            .filter_map(|project| {
                match project.status.as_str() {
                    "healthy" => None,
                    "degraded" => Some(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            &project.name,
                            "Compose project is degraded",
                            Severity::Warning,
                            Confidence::High,
                            MonitorDomain::Compose,
                        )
                        .with_evidence(
                            &format!("compose.projects[{}].status", project.name),
                            "degraded",
                            "Project has unhealthy containers",
                        )
                        .with_impact(format!(
                            "Compose project {} is degraded — {} of {} containers running, {} unhealthy",
                            project.name, project.running_count, project.container_count, project.unhealthy_count
                        ))
                        .with_read_only_check(format!(
                            "docker-compose -p {} ps",
                            project.name
                        )),
                    ),
                    "down" => Some(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            &project.name,
                            "Compose project is down",
                            Severity::Critical,
                            Confidence::High,
                            MonitorDomain::Compose,
                        )
                        .with_evidence(
                            &format!("compose.projects[{}].status", project.name),
                            "down",
                            "Project has no running containers",
                        )
                        .with_impact(format!(
                            "Compose project {} is down — all {} containers are stopped",
                            project.name, project.container_count
                        ))
                        .with_read_only_check(format!(
                            "docker-compose -p {} ps && docker-compose -p {} logs --tail=50",
                            project.name, project.name
                        )),
                    ),
                    _ => None,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::compose::{ComposeProject, ComposeSnapshot};
    use crate::snapshot::{HostIdentity, MonitorProfile, SnapshotDomains, SystemSnapshot};

    fn make_snapshot(projects: Vec<ComposeProject>) -> SystemSnapshot {
        let doms = SnapshotDomains {
            host: HostIdentity::default(),
            load: Default::default(),
            disks: Default::default(),
            services: Default::default(),
            containers: Default::default(),
            ports: Default::default(),
            logs: Default::default(),
            backups: Default::default(),
            packages: Default::default(),
            timers: Default::default(),
            network: Default::default(),
            processes: Default::default(),
            firewall: Default::default(),
            kubernetes: Default::default(),
            libvirt: Default::default(),
            compose: ComposeSnapshot {
                projects,
                total_container_count: 0,
                available: true,
            },
        };
        SystemSnapshot::new(
            "test-snapshot".to_string(),
            HostIdentity::default(),
            MonitorProfile::Standard,
            doms,
        )
    }

    #[test]
    fn test_empty_snapshot() {
        let detector = ComposeHealthDetector;
        let snapshot = make_snapshot(vec![]);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_healthy_project() {
        let detector = ComposeHealthDetector;
        let projects = vec![ComposeProject {
            name: "web-stack".to_string(),
            status: "healthy".to_string(),
            container_count: 3,
            running_count: 3,
            unhealthy_count: 0,
        }];
        let snapshot = make_snapshot(projects);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_degraded_project() {
        let detector = ComposeHealthDetector;
        let projects = vec![ComposeProject {
            name: "app-stack".to_string(),
            status: "degraded".to_string(),
            container_count: 4,
            running_count: 3,
            unhealthy_count: 1,
        }];
        let snapshot = make_snapshot(projects);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn test_down_project() {
        let detector = ComposeHealthDetector;
        let projects = vec![ComposeProject {
            name: "db-stack".to_string(),
            status: "down".to_string(),
            container_count: 2,
            running_count: 0,
            unhealthy_count: 2,
        }];
        let snapshot = make_snapshot(projects);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
    }

    #[test]
    fn test_mixed_projects() {
        let detector = ComposeHealthDetector;
        let projects = vec![
            ComposeProject {
                name: "proj1".to_string(),
                status: "healthy".to_string(),
                container_count: 3,
                running_count: 3,
                unhealthy_count: 0,
            },
            ComposeProject {
                name: "proj2".to_string(),
                status: "degraded".to_string(),
                container_count: 4,
                running_count: 3,
                unhealthy_count: 1,
            },
            ComposeProject {
                name: "proj3".to_string(),
                status: "healthy".to_string(),
                container_count: 2,
                running_count: 2,
                unhealthy_count: 0,
            },
            ComposeProject {
                name: "proj4".to_string(),
                status: "down".to_string(),
                container_count: 5,
                running_count: 0,
                unhealthy_count: 5,
            },
        ];
        let snapshot = make_snapshot(projects);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().any(|f| f.severity == Severity::Warning));
        assert!(findings.iter().any(|f| f.severity == Severity::Critical));
    }
}
