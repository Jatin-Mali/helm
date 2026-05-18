//! Kubernetes OOMKill detection — pod has been killed by OOM, indicating memory pressure.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct OOMKillDetector;

impl Detector for OOMKillDetector {
    fn id(&self) -> &'static str {
        "oom-kill"
    }

    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Kubernetes
    }

    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        snapshot
            .domains
            .kubernetes
            .pods
            .iter()
            .filter_map(|pod| {
                if pod.oom_kill_count > 0 {
                    Some(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            &format!("{}:{}", pod.namespace, pod.name),
                            "Pod experienced OOMKill",
                            Severity::Critical,
                            Confidence::High,
                            MonitorDomain::Kubernetes,
                        )
                        .with_evidence(
                            &format!("kubernetes.pods[{}:{}].oom_kill_count", pod.namespace, pod.name),
                            &pod.oom_kill_count.to_string(),
                            "Pod was killed by OOM killer",
                        )
                        .with_impact(format!(
                            "Pod {}:{} was killed by OOM killer {} times — memory request may be too low",
                            pod.namespace, pod.name, pod.oom_kill_count
                        ))
                        .with_read_only_check(format!(
                            "kubectl get pod {} -n {} -o jsonpath='{{.status.containerStatuses[*].lastState.oom}}'",
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
        let detector = OOMKillDetector;
        let snapshot = make_snapshot(vec![]);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_pod_no_oom() {
        let detector = OOMKillDetector;
        let pods = vec![PodInfo {
            namespace: "default".to_string(),
            name: "test-pod".to_string(),
            status: "Running".to_string(),
            ready: "1/1".to_string(),
            restarts: 0,
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
    fn test_pod_with_oom() {
        let detector = OOMKillDetector;
        let pods = vec![PodInfo {
            namespace: "default".to_string(),
            name: "memory-hog".to_string(),
            status: "Running".to_string(),
            ready: "1/1".to_string(),
            restarts: 3,
            age: "1h".to_string(),
            events_count: 5,
            oom_kill_count: 1,
            pvc_pressure: false,
        }];
        let snapshot = make_snapshot(pods);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
    }

    #[test]
    fn test_multiple_oom_kills() {
        let detector = OOMKillDetector;
        let pods = vec![PodInfo {
            namespace: "prod".to_string(),
            name: "leaky-app".to_string(),
            status: "Running".to_string(),
            ready: "1/1".to_string(),
            restarts: 10,
            age: "5h".to_string(),
            events_count: 20,
            oom_kill_count: 5,
            pvc_pressure: false,
        }];
        let snapshot = make_snapshot(pods);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
    }

    #[test]
    fn test_mixed_pods() {
        let detector = OOMKillDetector;
        let pods = vec![
            PodInfo {
                namespace: "default".to_string(),
                name: "ok-pod".to_string(),
                status: "Running".to_string(),
                ready: "1/1".to_string(),
                restarts: 0,
                age: "1h".to_string(),
                events_count: 0,
                oom_kill_count: 0,
                pvc_pressure: false,
            },
            PodInfo {
                namespace: "default".to_string(),
                name: "oom-pod".to_string(),
                status: "Running".to_string(),
                ready: "1/1".to_string(),
                restarts: 2,
                age: "2h".to_string(),
                events_count: 3,
                oom_kill_count: 1,
                pvc_pressure: false,
            },
        ];
        let snapshot = make_snapshot(pods);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
    }
}
