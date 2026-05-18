//! Web server site detector: checks for TLS coverage and site configuration issues.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct WebServerDetector;

impl Detector for WebServerDetector {
    fn id(&self) -> &'static str {
        "web-server"
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

        // Check nginx/apache/caddy services
        let web_services: Vec<_> = snapshot
            .domains
            .services
            .units
            .iter()
            .filter(|u| {
                let n = u.name.to_lowercase();
                n.contains("nginx")
                    || n.contains("apache")
                    || n.contains("httpd")
                    || n.contains("caddy")
            })
            .collect();

        for svc in &web_services {
            if svc.active != "active" || svc.sub != "running" {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        &svc.name,
                        &format!("Web server {} is {} ({})", svc.name, svc.active, svc.sub),
                        if svc.active == "failed" {
                            Severity::Critical
                        } else {
                            Severity::Warning
                        },
                        Confidence::High,
                        MonitorDomain::Services,
                    )
                    .with_evidence(
                        &format!("services[{}]", svc.name),
                        &format!("{} {}", svc.active, svc.sub),
                        "Web server is not serving traffic",
                    )
                    .with_impact("Websites and APIs may be down")
                    .with_read_only_check(format!("systemctl status {}", svc.name)),
                );
            }
        }

        // Check for HTTP-only listeners (no HTTPS)
        let http_only: Vec<_> = snapshot
            .domains
            .ports
            .listeners
            .iter()
            .filter(|l| {
                l.local_port == 80
                    && !snapshot
                        .domains
                        .ports
                        .listeners
                        .iter()
                        .any(|tls| tls.local_port == 443)
            })
            .collect();

        if !http_only.is_empty() && !web_services.is_empty() {
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "web",
                    "Web server listening on HTTP (80) but no HTTPS (443) — consider enabling TLS",
                    Severity::Warning,
                    Confidence::High,
                    MonitorDomain::Ports,
                )
                .with_evidence("ports.listeners", "443 not found", "Only HTTP port is open")
                .with_impact("Traffic is unencrypted — credentials and data may be exposed"),
            );
        }

        findings
    }
}
