//! Kubernetes PVC pressure detection — pod is experiencing storage pressure.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct PVCPressureDetector;

impl Detector for PVCPressureDetector {
    fn id(&self) -> &'static str {
        "pvc-pressure"
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
                if pod.pvc_pressure {
                    Some(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            &format!("{}:{}", pod.namespace, pod.name),
                            "Pod experiencing PVC pressure",
                            Severity::Warning,
                            Confidence::High,
                            MonitorDomain::Kubernetes,
                        )
                        .with_evidence(
                            &format!("kubernetes.pods[{}:{}].pvc_pressure", pod.namespace, pod.name),
                            "true",
                            "Pod detected storage pressure condition",
                        )
                        .with_impact(format!(
                            "Pod {}:{} is experiencing storage pressure — may impact performance or fail if storage is exhausted",
                            pod.namespace, pod.name
                        ))
                        .with_read_only_check(format!(
                            "kubectl get pod {} -n {} -o jsonpath='{{.status.conditions[?(@.type==\"StoragePressure\")]}}' | grep -q True",
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
        let detector = PVCPressureDetector;
        let snapshot = make_snapshot(vec![]);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn test_pod_no_pressure() {
        let detector = PVCPressureDetector;
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
    fn test_pod_with_pvc_pressure() {
        let detector = PVCPressureDetector;
        let pods = vec![PodInfo {
            namespace: "default".to_string(),
            name: "storage-pod".to_string(),
            status: "Running".to_string(),
            ready: "1/1".to_string(),
            restarts: 0,
            age: "1h".to_string(),
            events_count: 2,
            oom_kill_count: 0,
            pvc_pressure: true,
        }];
        let snapshot = make_snapshot(pods);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn test_multiple_pods_mixed() {
        let detector = PVCPressureDetector;
        let pods = vec![
            PodInfo {
                namespace: "default".to_string(),
                name: "pod1".to_string(),
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
                name: "pod2".to_string(),
                status: "Running".to_string(),
                ready: "1/1".to_string(),
                restarts: 1,
                age: "2h".to_string(),
                events_count: 1,
                oom_kill_count: 0,
                pvc_pressure: true,
            },
            PodInfo {
                namespace: "prod".to_string(),
                name: "pod3".to_string(),
                status: "Running".to_string(),
                ready: "1/1".to_string(),
                restarts: 0,
                age: "3h".to_string(),
                events_count: 0,
                oom_kill_count: 0,
                pvc_pressure: true,
            },
        ];
        let snapshot = make_snapshot(pods);
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 2);
    }
}
