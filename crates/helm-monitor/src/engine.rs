//! Snapshot engine: orchestrates all collectors with partial-failure tolerance.

use uuid::Uuid;

use crate::{
    alerting::{AlertConfig, AlertRouter},
    collectors::{
        backups::BackupsCollector, compose::ComposeCollector, containers::ContainersCollector,
        disks::DisksCollector, firewall::FirewallCollector, host::HostCollector,
        kubernetes::KubernetesCollector, libvirt::LibvirtCollector, load::LoadCollector,
        logs::LogsCollector, network::NetworkCollector, packages::PackagesCollector,
        ports::PortsCollector, processes::ProcessCollector, services::ServicesCollector,
        timers::TimersCollector,
    },
    detectors::DetectorRegistry,
    findings::Finding,
    snapshot::{CollectorError, MonitorProfile, SnapshotDomains, SystemSnapshot},
};

use crate::collectors::Collector as _;

/// Orchestrates collect → detect → alert in a single tick.
pub struct AlertingEngine {
    detectors: DetectorRegistry,
    router: AlertRouter,
    previous: Option<SystemSnapshot>,
}

impl AlertingEngine {
    pub fn new(router: AlertRouter) -> Self {
        Self {
            detectors: DetectorRegistry::default(),
            router,
            previous: None,
        }
    }

    /// Replace the detector registry (used in tests).
    pub fn with_detectors(mut self, detectors: DetectorRegistry) -> Self {
        self.detectors = detectors;
        self
    }

    /// Collect a snapshot, run detectors, route findings through alerting sinks.
    pub async fn run_once(
        &mut self,
        profile: MonitorProfile,
    ) -> Result<Vec<Finding>, Box<dyn std::error::Error>> {
        let snapshot = collect_snapshot(profile).await;
        let findings = self
            .detectors
            .detect(&snapshot, None, self.previous.as_ref());
        for finding in &findings {
            self.router.route(finding).await?;
        }
        self.previous = Some(snapshot);
        Ok(findings)
    }
}

impl Default for AlertingEngine {
    fn default() -> Self {
        Self::new(AlertRouter::new(AlertConfig::default()))
    }
}

pub async fn collect_snapshot(profile: MonitorProfile) -> SystemSnapshot {
    let id = Uuid::new_v4().to_string();
    let mut errors: Vec<CollectorError> = Vec::new();

    // Run all collectors concurrently via tokio::join!
    let (
        host_result,
        load_result,
        disks_result,
        services_result,
        containers_result,
        ports_result,
        logs_result,
        backups_result,
        packages_result,
        timers_result,
        network_result,
        processes_result,
        firewall_result,
        kubernetes_result,
        libvirt_result,
        compose_result,
    ) = tokio::join!(
        HostCollector.collect(profile),
        LoadCollector.collect(profile),
        DisksCollector.collect(profile),
        ServicesCollector.collect(profile),
        ContainersCollector.collect(profile),
        PortsCollector.collect(profile),
        LogsCollector.collect(profile),
        BackupsCollector.collect(profile),
        PackagesCollector.collect(profile),
        TimersCollector.collect(profile),
        NetworkCollector.collect(profile),
        ProcessCollector.collect(profile),
        FirewallCollector.collect(profile),
        KubernetesCollector.collect(profile),
        LibvirtCollector.collect(profile),
        ComposeCollector.collect(profile),
    );

    let host_identity = unwrap_or_default(host_result, "host", &mut errors);
    let load_out = unwrap_or_default(load_result, "load", &mut errors);
    let disks_out = unwrap_or_default(disks_result, "disks", &mut errors);
    let services_out = unwrap_or_default(services_result, "services", &mut errors);
    let containers_out = unwrap_or_default(containers_result, "containers", &mut errors);
    let ports_out = unwrap_or_default(ports_result, "ports", &mut errors);
    let logs_out = unwrap_or_default(logs_result, "logs", &mut errors);
    let backups_out = unwrap_or_default(backups_result, "backups", &mut errors);
    let packages_out = unwrap_or_default(packages_result, "packages", &mut errors);
    let timers_out = unwrap_or_default(timers_result, "timers", &mut errors);
    let network_out = unwrap_or_default(network_result, "network", &mut errors);
    let processes_out = unwrap_or_default(processes_result, "processes", &mut errors);
    let firewall_out = unwrap_or_default(firewall_result, "firewall", &mut errors);
    let kubernetes_out = unwrap_or_default(kubernetes_result, "kubernetes", &mut errors);
    let libvirt_out = unwrap_or_default(libvirt_result, "libvirt", &mut errors);
    let compose_out = unwrap_or_default(compose_result, "compose", &mut errors);

    let domains = SnapshotDomains {
        host: host_identity.clone(),
        load: load_out,
        disks: disks_out,
        services: services_out,
        containers: containers_out,
        ports: ports_out,
        logs: logs_out,
        backups: backups_out,
        packages: packages_out,
        timers: timers_out,
        network: network_out,
        processes: processes_out,
        firewall: firewall_out,
        kubernetes: kubernetes_out,
        libvirt: libvirt_out,
        compose: compose_out,
    };

    SystemSnapshot {
        id,
        host: host_identity,
        collected_at: chrono::Utc::now(),
        profile,
        domains,
        collector_errors: errors,
        redaction_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn unwrap_or_default<T: Default>(
    result: Result<T, CollectorError>,
    domain: &str,
    errors: &mut Vec<CollectorError>,
) -> T {
    match result {
        Ok(v) => v,
        Err(e) => {
            errors.push(CollectorError {
                domain: domain.to_string(),
                message: e.message,
                is_timeout: e.is_timeout,
            });
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        alerting::{AlertConfig, AlertPayload, AlertRouter, AlertSink, SendFuture},
        detectors::Detector,
        findings::{Confidence, Finding, MonitorDomain, Severity},
        snapshot::MonitorProfile,
    };
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn test_collect_snapshot_includes_new_domains() {
        let result = collect_snapshot(MonitorProfile::Standard).await;
        let _kubernetes = &result.domains.kubernetes;
        let _libvirt = &result.domains.libvirt;
        let _compose = &result.domains.compose;
        let domain_names = crate::snapshot::SnapshotDomains::domain_names();
        assert!(domain_names.contains(&"kubernetes"));
        assert!(domain_names.contains(&"libvirt"));
        assert!(domain_names.contains(&"compose"));
    }

    struct CaptureSink(Arc<Mutex<Vec<String>>>);

    impl AlertSink for CaptureSink {
        fn send<'a>(&'a self, alert: &'a AlertPayload) -> SendFuture<'a> {
            let captured = self.0.clone();
            let fp = alert.fingerprint.clone();
            Box::pin(async move {
                captured.lock().unwrap().push(fp);
                Ok(())
            })
        }
    }

    struct AlwaysCritDetector;

    impl Detector for AlwaysCritDetector {
        fn id(&self) -> &'static str {
            "test-crit"
        }
        fn domain(&self) -> MonitorDomain {
            MonitorDomain::Services
        }
        fn detect(
            &self,
            snapshot: &SystemSnapshot,
            _previous: Option<&SystemSnapshot>,
        ) -> Vec<Finding> {
            vec![Finding::new(
                &snapshot.id,
                "test-crit",
                "test-resource",
                "Test CRIT alert",
                Severity::Critical,
                Confidence::High,
                MonitorDomain::Services,
            )]
        }
    }

    #[tokio::test]
    async fn alerting_engine_routes_crit_finding_to_sink() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Box::new(CaptureSink(captured.clone()));
        let config = AlertConfig {
            min_severity: Severity::Critical,
            dedup_window_secs: 0,
            rate_limit_per_min: 60,
        };
        let router = AlertRouter::new(config).with_sink(sink);

        let mut reg = DetectorRegistry::new();
        reg.register(Box::new(AlwaysCritDetector));

        let mut engine = AlertingEngine::new(router).with_detectors(reg);
        let findings = engine.run_once(MonitorProfile::Standard).await.unwrap();

        assert!(!findings.is_empty(), "detector should produce at least one finding");
        assert!(
            !captured.lock().unwrap().is_empty(),
            "sink should receive the routed alert"
        );
    }
}
