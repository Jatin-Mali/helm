//! Config drift detector: compares file hashes between snapshots.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};
use std::collections::HashMap;

pub struct ConfigDriftDetector;

impl Detector for ConfigDriftDetector {
    fn id(&self) -> &'static str {
        "config-drift"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Logs
    }

    fn detect(&self, snap: &SystemSnapshot, previous: Option<&SystemSnapshot>) -> Vec<Finding> {
        let mut findings = Vec::new();
        let prev = match previous {
            Some(p) => p,
            None => return findings,
        };

        // Compare critical config files
        let watch_paths = [
            "/etc/ssh/sshd_config",
            "/etc/nginx/nginx.conf",
            "/etc/caddy/Caddyfile",
            "/etc/fstab",
            "/etc/sysctl.conf",
        ];

        for path in &watch_paths {
            let current_hash = file_hash(path);
            if let Some(h) = &current_hash {
                // We store a simple hash — if same file exists in prev snapshot log domain, compare
                let prev_hash = previous_snapshot_path_hash(prev, path);
                if let Some(ph) = prev_hash {
                    if h != &ph {
                        findings.push(
                            Finding::new(
                                &snap.id,
                                self.id(),
                                path,
                                &format!("Config file {} has changed since last snapshot", path),
                                Severity::Info,
                                Confidence::High,
                                MonitorDomain::Logs,
                            )
                            .with_evidence(
                                path,
                                &format!("hash: {} → {}", ph, h),
                                "File content changed between snapshots",
                            )
                            .with_impact("Configuration change may affect system behavior"),
                        );
                    }
                }
            }
        }

        // Compare running services
        let prev_services: HashMap<&str, &str> = prev
            .domains
            .services
            .units
            .iter()
            .map(|u| (u.name.as_str(), u.active.as_str()))
            .collect();
        for unit in &snap.domains.services.units {
            if let Some(prev_state) = prev_services.get(unit.name.as_str()) {
                if *prev_state != unit.active {
                    let sev = if unit.active == "failed" {
                        Severity::Critical
                    } else {
                        Severity::Info
                    };
                    findings.push(
                        Finding::new(
                            &snap.id,
                            self.id(),
                            &unit.name,
                            &format!(
                                "Service {} changed state: {} → {}",
                                unit.name, prev_state, unit.active
                            ),
                            sev,
                            Confidence::High,
                            MonitorDomain::Services,
                        )
                        .with_evidence(
                            &unit.name,
                            &format!("{} → {}", prev_state, unit.active),
                            "Service state changed between snapshots",
                        )
                        .with_impact("Service behavior may have changed"),
                    );
                }
            }
        }

        findings
    }
}

fn file_hash(path: &str) -> Option<String> {
    use std::hash::{Hash, Hasher};
    std::fs::read(path).ok().map(|data| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut h);
        format!("{:016x}", h.finish())
    })
}

fn previous_snapshot_path_hash(_prev: &SystemSnapshot, _path: &str) -> Option<String> {
    // We can't actually read the file from previous snapshot —
    // instead we store hash in the logs domain's metadata.
    // For now, return None to avoid false positives.
    // Future: store file hashes in snapshot logs domain.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::*;

    #[test]
    fn no_finding_without_previous() {
        let snap = SystemSnapshot {
            id: "t1".into(),
            host: HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: MonitorProfile::Standard,
            domains: SnapshotDomains::default(),
            collector_errors: vec![],
            redaction_version: "0.1".into(),
        };
        let findings = ConfigDriftDetector.detect(&snap, None);
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_service_state_changes() {
        let mut prev_doms = SnapshotDomains::default();
        prev_doms.services.units = vec![SystemdUnit {
            name: "nginx".into(),
            load: "loaded".into(),
            active: "active".into(),
            sub: "running".into(),
            description: "nginx".into(),
        }];
        let prev = SystemSnapshot {
            id: "t0".into(),
            host: HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: MonitorProfile::Standard,
            domains: prev_doms,
            collector_errors: vec![],
            redaction_version: "0.1".into(),
        };

        let mut curr_doms = SnapshotDomains::default();
        curr_doms.services.units = vec![SystemdUnit {
            name: "nginx".into(),
            load: "loaded".into(),
            active: "failed".into(),
            sub: "failed".into(),
            description: "nginx".into(),
        }];
        let curr = SystemSnapshot {
            id: "t1".into(),
            host: HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: MonitorProfile::Standard,
            domains: curr_doms,
            collector_errors: vec![],
            redaction_version: "0.1".into(),
        };

        let findings = ConfigDriftDetector.detect(&curr, Some(&prev));
        assert!(findings.iter().any(|f| f.title.contains("changed state")));
    }
}
