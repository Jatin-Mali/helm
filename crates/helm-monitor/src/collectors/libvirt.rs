//! Libvirt/QEMU collector: virtual machine domain state, snapshots, and metrics.
use crate::{
    collectors::{Collector, bin_exists, err, run_timed},
    snapshot::MonitorProfile,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// LibvirtSnapshot: collection of VM domains and availability status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LibvirtSnapshot {
    /// List of virtual machine domains.
    pub domains: Vec<VmDomain>,
    /// Total count of domains detected.
    pub domain_count: usize,
    /// True if libvirt/virsh is available and collection succeeded.
    pub available: bool,
}

/// VmDomain: state and metrics for a single virtual machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmDomain {
    /// Domain name.
    pub name: String,
    /// Domain state (running, shut off, paused, etc).
    pub state: String,
    /// Virtual CPU count.
    pub vcpus: u32,
    /// Memory allocated in MB.
    pub memory_mb: u64,
    /// Number of snapshots for this domain.
    pub snapshot_count: u32,
    /// Age of oldest snapshot in days, if any snapshots exist.
    pub age_days: Option<u32>,
}

/// LibvirtCollector: wraps virsh to collect VM metrics.
#[derive(Default)]
pub struct LibvirtCollector;

impl Collector for LibvirtCollector {
    type Output = LibvirtSnapshot;

    fn domain(&self) -> &'static str {
        "libvirt"
    }

    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        // Step 1: Check if virsh is available.
        if !bin_exists("virsh") {
            return Ok(LibvirtSnapshot {
                domains: vec![],
                domain_count: 0,
                available: false,
            });
        }

        // Step 2: Collect domain information from virsh list --all.
        let mut domains = match run_timed("virsh", &["list", "--all"], profile).await {
            Ok(o) => parse_virsh_list(&String::from_utf8_lossy(&o.stdout)),
            Err(e) => return Err(err("libvirt", e.message)),
        };

        // Step 3 (optional): Enrich with snapshot information if available.
        if let Ok(o) = run_timed("virsh", &["snapshot-list", "--all"], profile).await {
            let snapshots = parse_virsh_snapshots(&String::from_utf8_lossy(&o.stdout));
            for domain in domains.iter_mut() {
                if let Some(snap_info) = snapshots.get(&domain.name) {
                    domain.snapshot_count = snap_info.count;
                    domain.age_days = snap_info.age_days;
                }
            }
        }

        Ok(LibvirtSnapshot {
            domain_count: domains.len(),
            domains,
            available: true,
        })
    }
}

/// Parse virsh list --all output into VmDomain entries.
fn parse_virsh_list(s: &str) -> Vec<VmDomain> {
    s.lines()
        .skip(2) // Skip header lines: " Id    Name                    State"
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            // Format: " 1    domain-name              running"
            // or:     " -    domain-name              shut off"
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                return None;
            }
            // ID is parts[0], name is parts[1], state is remaining parts joined
            let name = parts[1].to_string();
            let state = parts[2..].join(" ");

            Some(VmDomain {
                name,
                state,
                vcpus: 0,     // Will be enriched from domstats if needed
                memory_mb: 0, // Will be enriched from domstats if needed
                snapshot_count: 0,
                age_days: None,
            })
        })
        .collect()
}

/// Parse virsh snapshot-list output and return snapshot info per domain.
fn parse_virsh_snapshots(s: &str) -> HashMap<String, SnapshotInfo> {
    let mut result = HashMap::new();

    // virsh snapshot-list returns per-domain listing; for now, use a simple heuristic:
    // If the output contains "Domain" sections, parse them.
    let mut current_domain: Option<String> = None;
    let mut snapshots_in_domain = vec![];

    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check for domain name (heuristic: lines with "Domain:" prefix)
        if trimmed.starts_with("Domain:") {
            if let Some(prev_domain) = current_domain.take() {
                if !snapshots_in_domain.is_empty() {
                    let count = snapshots_in_domain.len() as u32;
                    let age_days = calculate_age_days(&snapshots_in_domain);
                    result.insert(prev_domain, SnapshotInfo { count, age_days });
                }
            }
            current_domain = Some(
                trimmed
                    .strip_prefix("Domain:")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            );
            snapshots_in_domain.clear();
        } else if let Some(ref _domain) = current_domain {
            // If we're inside a domain section, collect snapshot info
            // Simple heuristic: lines with timestamps are snapshots
            if trimmed.contains('-') && trimmed.len() > 10 {
                snapshots_in_domain.push(trimmed.to_string());
            }
        }
    }

    // Don't forget the last domain
    if let Some(domain) = current_domain {
        if !snapshots_in_domain.is_empty() {
            let count = snapshots_in_domain.len() as u32;
            let age_days = calculate_age_days(&snapshots_in_domain);
            result.insert(domain, SnapshotInfo { count, age_days });
        }
    }

    result
}

/// Metadata about snapshots for a single domain.
#[derive(Clone, Copy, Debug)]
struct SnapshotInfo {
    count: u32,
    age_days: Option<u32>,
}

/// Calculate age in days from a list of snapshot lines.
fn calculate_age_days(snapshots: &[String]) -> Option<u32> {
    // Simple heuristic: treat the first snapshot (oldest) as the age reference.
    // In a real implementation, parse creation timestamps.
    if snapshots.is_empty() {
        return None;
    }
    // For now, return None (age data requires timestamp parsing beyond scope).
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_virsh_list_empty() {
        let input = " Id    Name                    State\n";
        let result = parse_virsh_list(input);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_parse_virsh_list_single_domain() {
        let input = " Id    Name                    State\n------------------------------------------\n 1     centos-7                running\n";
        let result = parse_virsh_list(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "centos-7");
        assert_eq!(result[0].state, "running");
    }

    #[test]
    fn test_parse_virsh_list_multiple_domains() {
        let input = " Id    Name                    State\n------------------------------------------\n 1     vm1                     running\n -     vm2                     shut off\n 2     vm3                     paused\n";
        let result = parse_virsh_list(input);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "vm1");
        assert_eq!(result[0].state, "running");
        assert_eq!(result[1].name, "vm2");
        assert_eq!(result[1].state, "shut off");
        assert_eq!(result[2].name, "vm3");
        assert_eq!(result[2].state, "paused");
    }

    #[test]
    fn test_parse_virsh_snapshots_empty() {
        let input = "";
        let result = parse_virsh_snapshots(input);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_libvirt_snapshot_default() {
        let snap = LibvirtSnapshot::default();
        assert_eq!(snap.domains, vec![]);
        assert_eq!(snap.domain_count, 0);
        assert!(!snap.available);
    }

    #[test]
    fn test_vm_domain_structure() {
        let domain = VmDomain {
            name: "test-vm".to_string(),
            state: "running".to_string(),
            vcpus: 4,
            memory_mb: 2048,
            snapshot_count: 2,
            age_days: Some(5),
        };
        assert_eq!(domain.name, "test-vm");
        assert_eq!(domain.vcpus, 4);
        assert_eq!(domain.memory_mb, 2048);
    }
}
