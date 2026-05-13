//! Unexpected exposed listener detection (ports on 0.0.0.0).

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct ExposedPortDetector;

impl Detector for ExposedPortDetector {
    fn id(&self) -> &'static str {
        "exposed-port"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Ports
    }
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        // Flag non-localhost listeners as exposed
        snapshot
            .domains
            .ports
            .listeners
            .iter()
            .filter(|l| l.local_address != "127.0.0.1" && l.local_address != "::1")
            .map(|l| {
                let title = format!(
                    "Port {} ({}) is listening on {} — exposed to network",
                    l.local_port, l.protocol, l.local_address
                );
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    &format!("{}:{}", l.local_address, l.local_port),
                    &title,
                    Severity::Info,
                    Confidence::High,
                    MonitorDomain::Ports,
                )
                .with_evidence(
                    &format!(
                        "ports.listeners[{}:{}].local_address",
                        l.local_address, l.local_port
                    ),
                    &format!(
                        "{}:{} by {}",
                        l.local_address,
                        l.local_port,
                        l.process_name.as_deref().unwrap_or("?")
                    ),
                    "Port bound to non-loopback address is reachable from network",
                )
                .with_impact("Service is reachable from other hosts on the network")
                .with_read_only_check(format!("ss -tulpn | grep :{}", l.local_port))
            })
            .collect()
    }
}
