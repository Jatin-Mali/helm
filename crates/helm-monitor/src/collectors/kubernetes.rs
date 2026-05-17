//! Kubernetes collector: kubectl wrapper for pod metrics.
use crate::{
    collectors::{Collector, bin_exists, err, run_timed},
    snapshot::MonitorProfile,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Kubernetes cluster snapshot — pod metrics and counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KubernetesSnapshot {
    /// Pod information across all namespaces.
    pub pods: Vec<PodInfo>,
    /// Count of unique namespaces.
    pub namespace_count: usize,
    /// Whether kubectl is available and collector ran.
    pub available: bool,
}

/// Per-pod metrics extracted from kubectl output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodInfo {
    /// Kubernetes namespace.
    pub namespace: String,
    /// Pod name.
    pub name: String,
    /// Pod status (e.g., Running, Pending, Failed).
    pub status: String,
    /// Ready status (e.g., "2/3").
    pub ready: String,
    /// Container restart count.
    pub restarts: u32,
    /// Pod age (e.g., "1d", "2h30m").
    pub age: String,
    /// Event count for this pod (from kubectl describe or get events).
    pub events_count: u32,
    /// OOMKill event count.
    pub oom_kill_count: u32,
    /// Whether pod has PVC pressure signal.
    pub pvc_pressure: bool,
}

/// KubernetesCollector — wraps kubectl to collect pod metrics.
#[derive(Default)]
pub struct KubernetesCollector;

impl Collector for KubernetesCollector {
    type Output = KubernetesSnapshot;

    fn domain(&self) -> &'static str {
        "kubernetes"
    }

    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        // Check if kubectl is available
        if !bin_exists("kubectl") {
            return Ok(KubernetesSnapshot {
                pods: vec![],
                namespace_count: 0,
                available: false,
            });
        }

        // Run kubectl get pods -A -o wide
        match run_timed("kubectl", &["get", "pods", "-A", "-o", "wide"], profile).await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let pods = parse_kubectl(&stdout);

                // Count unique namespaces
                let namespaces: HashSet<String> =
                    pods.iter().map(|p| p.namespace.clone()).collect();
                let namespace_count = namespaces.len();

                Ok(KubernetesSnapshot {
                    pods,
                    namespace_count,
                    available: true,
                })
            }
            Err(e) => Err(err("kubernetes", e.message)),
        }
    }
}

/// Parse kubectl wide output into PodInfo vector.
fn parse_kubectl(s: &str) -> Vec<PodInfo> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .skip(1) // Skip header line
        .filter_map(|l| {
            let fields: Vec<&str> = l.split_whitespace().collect();
            if fields.len() < 6 {
                return None;
            }

            // kubectl get pods -A -o wide output columns:
            // NAMESPACE NAME READY STATUS RESTARTS AGE IP NODE NOMINATED_NODE READINESS_GATES
            let namespace = fields[0].to_string();
            let name = fields[1].to_string();
            let ready = fields[2].to_string();
            let status = fields[3].to_string();
            let restarts = fields[4].parse::<u32>().unwrap_or(0);
            let age = fields[5].to_string();

            // Check for OOMKilled in status
            let oom_kill_count = if status.contains("OOMKilled") { 1 } else { 0 };
            // PVC pressure detection (check status for PVC-related signals)
            let pvc_pressure = status.contains("UnmountVolume") || status.contains("MountVolume");

            Some(PodInfo {
                namespace,
                name,
                status,
                ready,
                restarts,
                age,
                events_count: 0, // Default for MVP; can be enriched later
                oom_kill_count,
                pvc_pressure,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kubectl_empty() {
        let result = parse_kubectl("");
        assert_eq!(result, vec![]);
    }

    #[test]
    fn parse_kubectl_header_only() {
        let header =
            "NAMESPACE NAME READY STATUS RESTARTS AGE IP NODE NOMINATED_NODE READINESS_GATES";
        let result = parse_kubectl(header);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn parse_kubectl_single_pod() {
        let output = "NAMESPACE NAME READY STATUS RESTARTS AGE IP NODE NOMINATED_NODE READINESS_GATES\ndefault nginx 1/1 Running 0 2h 10.0.0.1 node1 <none> <none>";
        let result = parse_kubectl(output);
        assert_eq!(result.len(), 1);

        let pod = &result[0];
        assert_eq!(pod.namespace, "default");
        assert_eq!(pod.name, "nginx");
        assert_eq!(pod.ready, "1/1");
        assert_eq!(pod.status, "Running");
        assert_eq!(pod.restarts, 0);
        assert_eq!(pod.age, "2h");
        assert_eq!(pod.oom_kill_count, 0);
        assert!(!pod.pvc_pressure);
    }

    #[test]
    fn parse_kubectl_multiple_namespaces() {
        let output = "NAMESPACE NAME READY STATUS RESTARTS AGE IP NODE NOMINATED_NODE READINESS_GATES\ndefault nginx 1/1 Running 0 2h 10.0.0.1 node1 <none> <none>\nkube-system coredns 1/1 Running 1 10d 10.0.0.2 node1 <none> <none>";
        let result = parse_kubectl(output);
        assert_eq!(result.len(), 2);

        assert_eq!(result[0].namespace, "default");
        assert_eq!(result[1].namespace, "kube-system");
        assert_eq!(result[1].restarts, 1);
    }

    #[test]
    fn parse_kubectl_oom_killed() {
        let output = "NAMESPACE NAME READY STATUS RESTARTS AGE IP NODE NOMINATED_NODE READINESS_GATES\ndefault app 0/1 OOMKilled 5 1h 10.0.0.1 node1 <none> <none>";
        let result = parse_kubectl(output);
        assert_eq!(result.len(), 1);

        let pod = &result[0];
        assert_eq!(pod.status, "OOMKilled");
        assert_eq!(pod.oom_kill_count, 1);
    }

    #[test]
    fn test_kubernetes_snapshot_default() {
        let snap = KubernetesSnapshot::default();
        assert_eq!(snap.pods, vec![]);
        assert_eq!(snap.namespace_count, 0);
        assert!(!snap.available);
    }
}
