//! Database health detector: checks PostgreSQL/MySQL instance health.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct DbHealthDetector;

impl Detector for DbHealthDetector {
    fn id(&self) -> &'static str {
        "db-health"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Services
    }
    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();
        // We access db_health via the snapshot — but it's not a standard domain.
        // Instead, this detector checks for database services in the services list.
        // For direct DB health, we'd need the snapshot integrated.
        // For now, we check if postgresql/mysql services exist and their state.
        let db_services: Vec<_> = snapshot
            .domains
            .services
            .units
            .iter()
            .filter(|u| {
                let name = u.name.to_lowercase();
                name.contains("postgres") || name.contains("mysql") || name.contains("mariadb")
            })
            .collect();

        for svc in &db_services {
            if svc.active != "active" || svc.sub != "running" {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        &svc.name,
                        &format!(
                            "Database service {} is {} ({})",
                            svc.name, svc.active, svc.sub
                        ),
                        if svc.active == "failed" {
                            Severity::Critical
                        } else {
                            Severity::Warning
                        },
                        Confidence::High,
                        MonitorDomain::Services,
                    )
                    .with_evidence(
                        &format!("services.units[{}]", svc.name),
                        &format!("active={} sub={}", svc.active, svc.sub),
                        "Database service is not running",
                    )
                    .with_impact("Database-dependent applications may be unavailable".to_string())
                    .with_read_only_check(format!("systemctl status {}", svc.name)),
                );
            }
        }

        // Also check for database processes listening
        let db_listeners: Vec<_> = snapshot
            .domains
            .ports
            .listeners
            .iter()
            .filter(|l| l.local_port == 5432 || l.local_port == 3306)
            .collect();

        // Check listening ports — if service is not running but port is listening
        for l in &db_listeners {
            let expected = match l.local_port {
                5432 => "postgresql",
                3306 => "mysql",
                _ => continue,
            };

            let has_service = db_services
                .iter()
                .any(|s| s.active == "active" && s.sub == "running");

            if !has_service {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        expected,
                        &format!(
                            "{} port {} is listening but no systemd service is active",
                            expected, l.local_port
                        ),
                        Severity::Info,
                        Confidence::Medium,
                        MonitorDomain::Ports,
                    )
                    .with_evidence(
                        &format!("ports.listeners[{}]", l.local_port),
                        &format!(
                            "{}:{} {}",
                            l.local_address,
                            l.local_port,
                            l.process_name.as_deref().unwrap_or("unknown")
                        ),
                        "Database port is listening without a managed service",
                    )
                    .with_assumption("Database may be running under a different service name"),
                );
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        HostIdentity, ListenerEntry, MonitorProfile, PortSnapshot, ServiceSnapshot,
        SnapshotDomains, SystemSnapshot, SystemdUnit,
    };

    #[allow(clippy::field_reassign_with_default)]
    fn make_snapshot(units: Vec<SystemdUnit>, listeners: Vec<ListenerEntry>) -> SystemSnapshot {
        let mut domains = SnapshotDomains::default();
        domains.services = ServiceSnapshot {
            units,
            failed_units: Vec::new(),
            timers: Vec::new(),
        };
        domains.ports = PortSnapshot { listeners };
        SystemSnapshot {
            id: "test".into(),
            host: HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: MonitorProfile::Standard,
            domains,
            collector_errors: Vec::new(),
            redaction_version: "0.1".into(),
        }
    }

    #[test]
    fn detects_failed_postgresql() {
        let snapshot = make_snapshot(
            vec![SystemdUnit {
                name: "postgresql".into(),
                load: "loaded".into(),
                active: "failed".into(),
                sub: "failed".into(),
                description: "PostgreSQL".into(),
            }],
            vec![],
        );
        let findings = DbHealthDetector.detect(&snapshot, None);
        assert!(findings.iter().any(|f| f.severity == Severity::Critical));
    }

    #[test]
    fn no_finding_for_healthy_db() {
        let snapshot = make_snapshot(
            vec![SystemdUnit {
                name: "postgresql".into(),
                load: "loaded".into(),
                active: "active".into(),
                sub: "running".into(),
                description: "PostgreSQL".into(),
            }],
            vec![],
        );
        let findings = DbHealthDetector.detect(&snapshot, None);
        assert!(findings.is_empty());
    }
}
