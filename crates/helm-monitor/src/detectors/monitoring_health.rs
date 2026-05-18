//! Monitoring system health detector: checks Grafana, InfluxDB, Prometheus.
//! Works from existing service and port data in the standard snapshot.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct MonitoringHealthDetector;

impl Detector for MonitoringHealthDetector {
    fn id(&self) -> &'static str {
        "monitoring-health"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Services
    }

    fn detect(&self, snapshot: &SystemSnapshot, _prev: Option<&SystemSnapshot>) -> Vec<Finding> {
        let mut findings = Vec::new();

        let monitoring_services = [
            ("grafana", 3000u16),
            ("influxdb", 8086u16),
            ("prometheus", 9090u16),
            ("alertmanager", 9093u16),
        ];

        for (name, port) in &monitoring_services {
            let svc = snapshot
                .domains
                .services
                .units
                .iter()
                .find(|u| u.name.to_lowercase().contains(name));

            let port_open = snapshot
                .domains
                .ports
                .listeners
                .iter()
                .any(|l| l.local_port == *port);

            if let Some(svc) = svc {
                if svc.active != "active" || svc.sub != "running" {
                    findings.push(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            name,
                            &format!(
                                "Monitoring service {} is {} ({})",
                                name, svc.active, svc.sub
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
                            &svc.name,
                            &format!("{} {}", svc.active, svc.sub),
                            "Monitoring system is not running",
                        )
                        .with_impact("Metrics and dashboards may be unavailable"),
                    );
                }
            } else if port_open {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        name,
                        &format!(
                            "{} port {} is listening but no systemd service found",
                            name, port
                        ),
                        Severity::Info,
                        Confidence::Medium,
                        MonitorDomain::Ports,
                    )
                    .with_evidence(
                        "ports.listeners",
                        &port.to_string(),
                        "Monitoring tool running outside systemd",
                    ),
                );
            }
        }

        findings
    }
}
