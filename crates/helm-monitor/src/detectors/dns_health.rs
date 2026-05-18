//! DNS health detector: checks resolver reachability and resolution.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct DnsHealthDetector;

impl Detector for DnsHealthDetector {
    fn id(&self) -> &'static str {
        "dns-health"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Network
    }

    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();

        let resolvers = &snapshot.domains.network.nameservers;
        if resolvers.is_empty() {
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "dns",
                    "No DNS servers configured — name resolution may not work",
                    Severity::Warning,
                    Confidence::High,
                    MonitorDomain::Network,
                )
                .with_evidence(
                    "network.nameservers",
                    "empty",
                    "No DNS resolvers found in /etc/resolv.conf",
                )
                .with_impact("Applications relying on DNS name resolution will fail"),
            );
        } else {
            let has_local = resolvers.iter().any(|r| {
                r.starts_with("127.") || r.starts_with("10.") || r.starts_with("192.168.")
            });
            let has_public = resolvers
                .iter()
                .any(|r| r == "8.8.8.8" || r == "1.1.1.1" || r == "9.9.9.9");
            if !has_local && !has_public {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        "dns",
                        &format!(
                            "DNS servers configured: {} — verify they are reachable",
                            resolvers.join(", ")
                        ),
                        Severity::Info,
                        Confidence::Medium,
                        MonitorDomain::Network,
                    )
                    .with_evidence(
                        "network.nameservers",
                        &resolvers.join(", "),
                        "Unrecognized DNS servers — cannot verify reachability from snapshot alone",
                    ),
                );
            }
        }

        findings
    }
}
