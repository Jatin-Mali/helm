//! SSH exposure and auth posture detector.
//! Checks listening SSH ports and sshd_config for hardening gaps.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct SshExposureDetector;

impl Detector for SshExposureDetector {
    fn id(&self) -> &'static str {
        "ssh-exposure"
    }

    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Ports
    }

    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();

        // Check if SSH is listening
        let ssh_listeners: Vec<_> = snapshot
            .domains
            .ports
            .listeners
            .iter()
            .filter(|l| {
                l.local_port == 22 || l.process_name.as_ref().is_some_and(|n| n.contains("sshd"))
            })
            .collect();

        if ssh_listeners.is_empty() {
            return findings; // No SSH — no risk
        }

        // Check for non-loopback exposure
        let exposed: Vec<_> = ssh_listeners
            .iter()
            .filter(|l| l.local_address != "127.0.0.1" && l.local_address != "::1")
            .collect();

        if !exposed.is_empty() {
            let addresses: Vec<_> = exposed
                .iter()
                .map(|l| format!("{}:{}", l.local_address, l.local_port))
                .collect();
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "sshd",
                    &format!(
                        "SSH exposed on non-loopback interface(s): {}",
                        addresses.join(", ")
                    ),
                    Severity::Warning,
                    Confidence::High,
                    MonitorDomain::Ports,
                )
                .with_evidence(
                    "ports.listeners",
                    &addresses.join(", "),
                    "SSH is accessible from the network",
                )
                .with_impact(
                    "SSH accessible from the network increases attack surface. \
                     Restrict with firewall rules if not needed.",
                )
                .with_read_only_check("ss -tlnp | grep :22"),
            );
        }

        // Try to read sshd_config for auth posture
        if let Ok(config) = std::fs::read_to_string("/etc/ssh/sshd_config") {
            let includes = config
                .lines()
                .any(|l| l.trim_start().starts_with("Include "));
            if includes {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        "sshd_config",
                        "sshd_config uses Include directives — full auth posture may not be visible",
                        Severity::Info,
                        Confidence::Medium,
                        MonitorDomain::Ports,
                    )
                    .with_evidence(
                        "sshd_config.Include",
                        "true",
                        "sshd_config includes additional config files",
                    )
                    .with_assumption("Included configs not fully parsed")
                    .with_missing_data("Contents of included sshd config files"),
                );
            }

            let effective: Vec<&str> = config
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !t.starts_with('#') && !t.is_empty() && !t.starts_with("Include ")
                })
                .collect();

            let effective_text = effective.join("\n");

            // Check PasswordAuthentication
            let password_auth = find_config_value(&effective_text, "PasswordAuthentication");
            let challenge_response =
                find_config_value(&effective_text, "ChallengeResponseAuthentication");
            let kbd_interactive =
                find_config_value(&effective_text, "KbdInteractiveAuthentication");

            let password_enabled = password_auth.is_none_or(|v| v == "yes")
                || (challenge_response == Some("yes"))
                || (kbd_interactive == Some("yes"));

            if password_enabled && password_auth != Some("no") {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        "sshd_config",
                        "SSH password authentication is enabled — consider key-only auth",
                        Severity::Warning,
                        Confidence::High,
                        MonitorDomain::Ports,
                    )
                    .with_evidence(
                        "sshd_config.PasswordAuthentication",
                        password_auth.unwrap_or("yes (default)"),
                        "Password auth allows brute-force attacks",
                    )
                    .with_impact("Password-based SSH auth is vulnerable to credential stuffing and brute-force")
                    .with_read_only_check("grep -i '^PasswordAuthentication' /etc/ssh/sshd_config"),
                );
            }

            // Check PermitRootLogin
            let permit_root = find_config_value(&effective_text, "PermitRootLogin");
            if permit_root == Some("yes") {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        "sshd_config",
                        "SSH root login is permitted — consider disabling",
                        Severity::Critical,
                        Confidence::High,
                        MonitorDomain::Ports,
                    )
                    .with_evidence(
                        "sshd_config.PermitRootLogin",
                        "yes",
                        "Root can log in directly via SSH",
                    )
                    .with_impact("Root SSH login bypasses audit trail and enables full system compromise via a single credential")
                    .with_read_only_check("grep -i '^PermitRootLogin' /etc/ssh/sshd_config"),
                );
            } else if permit_root.is_none() && password_enabled {
                findings.push(
                    Finding::new(
                        &snapshot.id,
                        self.id(),
                        "sshd_config",
                        "SSH root login may be permitted (PermitRootLogin not explicitly set, default varies)",
                        Severity::Info,
                        Confidence::Medium,
                        MonitorDomain::Ports,
                    )
                    .with_evidence(
                        "sshd_config.PermitRootLogin",
                        "not set",
                        "Default PermitRootLogin varies by distribution",
                    )
                    .with_assumption("Default sshd config may permit root login")
                    .with_missing_data("Distribution-specific sshd defaults"),
                );
            }
        } else {
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "sshd_config",
                    "Cannot read /etc/ssh/sshd_config — SSH auth posture unknown",
                    Severity::Info,
                    Confidence::Medium,
                    MonitorDomain::Ports,
                )
                .with_evidence(
                    "sshd_config",
                    "unreadable",
                    "Unable to assess SSH auth configuration",
                )
                .with_missing_data("sshd_config contents")
                .with_impact("SSH may be configured insecurely but cannot be verified"),
            );
        }

        findings
    }
}

/// Extract a config value from sshd_config lines.
/// Handles "Key value" and "Key=value" formats, case-insensitive key matching.
fn find_config_value<'a>(lines: &'a str, key: &str) -> Option<&'a str> {
    let key_lower = key.to_lowercase();
    for line in lines.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (k, v) = if let Some(pos) = trimmed.find(char::is_whitespace) {
            (trimmed[..pos].trim(), trimmed[pos..].trim())
        } else if let Some(pos) = trimmed.find('=') {
            (trimmed[..pos].trim(), trimmed[pos + 1..].trim())
        } else {
            continue;
        };
        if k.to_lowercase() == key_lower {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        HostIdentity, ListenerEntry, MonitorProfile, SnapshotDomains, SystemSnapshot,
    };

    #[allow(clippy::field_reassign_with_default)]
    fn make_snapshot(listeners: Vec<ListenerEntry>) -> SystemSnapshot {
        let mut domains = SnapshotDomains::default();
        let mut port_snap = crate::snapshot::PortSnapshot::default();
        port_snap.listeners = listeners;
        domains.ports = port_snap;
        SystemSnapshot {
            id: "test-1".into(),
            host: HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: MonitorProfile::Standard,
            domains,
            collector_errors: Vec::new(),
            redaction_version: "0.1.0".into(),
        }
    }

    fn local_listener() -> ListenerEntry {
        ListenerEntry {
            protocol: "tcp".into(),
            local_address: "127.0.0.1".into(),
            local_port: 22,
            process_name: Some("sshd".into()),
            pid: Some(1234),
        }
    }

    #[test]
    fn no_finding_when_ssh_not_listening() {
        let snapshot = make_snapshot(vec![]);
        let detector = SshExposureDetector;
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn detects_ssh_exposed_on_network() {
        let snapshot = make_snapshot(vec![ListenerEntry {
            local_address: "0.0.0.0".into(),
            ..local_listener()
        }]);
        let detector = SshExposureDetector;
        let findings = detector.detect(&snapshot, None);
        // At minimum finds the network exposure (sshd_config reading may also produce findings)
        assert!(findings.iter().any(|f| f.title.contains("non-loopback")));
    }

    #[test]
    fn no_exposure_finding_for_localhost() {
        let snapshot = make_snapshot(vec![local_listener()]);
        let detector = SshExposureDetector;
        let findings = detector.detect(&snapshot, None);
        // Should not have a non-loopback finding
        assert!(!findings.iter().any(|f| f.title.contains("non-loopback")));
    }

    #[test]
    fn test_find_config_value() {
        let config = "PasswordAuthentication no\nPermitRootLogin no\n#PubkeyAuthentication yes\n";
        assert_eq!(
            find_config_value(config, "PasswordAuthentication"),
            Some("no")
        );
        assert_eq!(find_config_value(config, "PermitRootLogin"), Some("no"));
        assert_eq!(find_config_value(config, "PubkeyAuthentication"), None); // commented out
        assert_eq!(find_config_value(config, "nonexistent"), None);
    }
}
