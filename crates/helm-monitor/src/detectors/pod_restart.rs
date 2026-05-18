//! Kubernetes pod restart detection — high restart count indicates instability.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct PodRestartDetector;

impl Detector for PodRestartDetector {
    fn id(&self) -> &'static str {
        "pod-restart"
    }

    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Kubernetes
    }

    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let threshold = 5;
        snapshot
            .domains
            .kubernetes
            .pods
            .iter()
            .filter_map(|pod| {
                if pod.restarts >= threshold {
                    let severity = if pod.restarts >= 10 {
                        Severity::Critical
                    } else {
                        Severity::Warning
                    };
                    Some(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            &format!("{}:{}", pod.namespace, pod.name),
                            &format!("Pod has high restart count ({})", pod.restarts),
                            severity,
                            Confidence::High,
                            MonitorDomain::Kubernetes,
                        )
                        .with_evidence(
                            &format!("kubernetes.pods[{}:{}].restarts", pod.namespace, pod.name),
                            &pod.restarts.to_string(),
                            "Pod restart count exceeds threshold",
                        )
                        .with_impact(format!(
                            "Pod {}:{} is unstable and may be crash-looping",
                            pod.namespace, pod.name
                        ))
                        .with_read_only_check(format!(
                            "kubectl get pod {} -n {} -o jsonpath='{{.status.containerStatuses[*].restartCount}}'",
                            pod.name, pod.namespace
                        )),
                    )
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::kubernetes::{KubernetesSnapshot, PodInfo};
    use crate::snapshot::{HostIdentity, MonitorProfile, SnapshotDomains, SystemSnapshot};

    fn make_snapshot(pods: Vec<PodInfo>) -> SystemSnapshot {
        let domains = SnapshotDomains {
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
            kubernetes: KubernetesSnapshot {
                pods,
                namespace_count: 0,
                available: true,
            },
            libvirt: Default::default(),
            compose: Default::default(),
        };
        SystemSnapshot::new(
            "test-snapshot".to_string(),
            HostIdentity::default(),
            MonitorProfile::Standard,
            domains,
        )
    }

    #[test]
    fn test_empty_snapshot() {
        let detector = PodRestartDetector;
        let snapshot = make_snapshot(vec![]);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_pod_below_threshold() {
        let detector = PodRestartDetector;
        let pods = vec![PodInfo {
            namespace: "default".to_string(),
            name: "test-pod".to_string(),
            status: "Running".to_string(),
            ready: "1/1".to_string(),
            restarts: 2,
            age: "1h".to_string(),
            events_count: 0,
            oom_kill_count: 0,
            pvc_pressure: false,
        }];
        let snapshot = make_snapshot(pods);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_pod_at_threshold() {
        let detector = PodRestartDetector;
        let pods = vec![PodInfo {
            namespace: "default".to_string(),
            name: "test-pod".to_string(),
            status: "Running".to_string(),
            ready: "1/1".to_string(),
            restarts: 5,
            age: "1h".to_string(),
            events_count: 0,
            oom_kill_count: 0,
            pvc_pressure: false,
        }];
        let snapshot = make_snapshot(pods);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn test_pod_above_critical_threshold() {
        let detector = PodRestartDetector;
        let pods = vec![PodInfo {
            namespace: "kube-system".to_string(),
            name: "crasher".to_string(),
            status: "Running".to_string(),
            ready: "0/1".to_string(),
            restarts: 15,
            age: "2h".to_string(),
            events_count: 10,
            oom_kill_count: 0,
            pvc_pressure: false,
        }];
        let snapshot = make_snapshot(pods);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
    }

    #[test]
    fn test_multiple_pods_mixed() {
        let detector = PodRestartDetector;
        let pods = vec![
            PodInfo {
                namespace: "default".to_string(),
                name: "pod1".to_string(),
                status: "Running".to_string(),
                ready: "1/1".to_string(),
                restarts: 2,
                age: "1h".to_string(),
                events_count: 0,
                oom_kill_count: 0,
                pvc_pressure: false,
            },
            PodInfo {
                namespace: "default".to_string(),
                name: "pod2".to_string(),
                status: "Running".to_string(),
                ready: "1/1".to_string(),
                restarts: 6,
                age: "2h".to_string(),
                events_count: 3,
                oom_kill_count: 0,
                pvc_pressure: false,
            },
            PodInfo {
                namespace: "prod".to_string(),
                name: "pod3".to_string(),
                status: "Running".to_string(),
                ready: "1/1".to_string(),
                restarts: 12,
                age: "3h".to_string(),
                events_count: 8,
                oom_kill_count: 0,
                pvc_pressure: false,
            },
        ];
        let snapshot = make_snapshot(pods);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().any(|f| f.severity == Severity::Warning));
        assert!(findings.iter().any(|f| f.severity == Severity::Critical));
    }
}
