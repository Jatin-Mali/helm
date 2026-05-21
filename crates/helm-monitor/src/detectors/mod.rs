//! Detector trait and registry. Per TRD §4.3.

pub mod backup_schedule;
pub mod backup_stale;
pub mod cert_expiry;
pub mod compose_health;
pub mod config_drift;
pub mod container_restart;
pub mod db_health;
pub mod desktop_notify;
pub mod disk_usage;
pub mod dns_health;
pub mod domain_state;
pub mod exposed_port;
pub mod failed_services;
pub mod fs_readonly;
pub mod high_load;
pub mod inactive_service;
pub mod inode_usage;
pub mod journal_errors;
pub mod memory_pressure;
pub mod monitoring_health;
pub mod oom_event;
pub mod oom_kill;
pub mod package_updates;
pub mod pod_restart;
pub mod pvc_pressure;
pub mod restart_loop;
pub mod restore_test;
pub mod smart;
pub mod snapshot_age;
pub mod ssh_exposure;
pub mod swap_exhaustion;
pub mod unhealthy_container;
pub mod web_server;

use crate::{
    findings::{Finding, MonitorDomain},
    snapshot::SystemSnapshot,
    thresholds::ThresholdsConfig,
};

/// A detector inspects a typed snapshot and produces zero or more findings.
pub trait Detector: Send + Sync {
    fn id(&self) -> &'static str;
    fn domain(&self) -> MonitorDomain;
    /// Detect issues in the current snapshot, optionally using a previous snapshot as baseline.
    fn detect(&self, snapshot: &SystemSnapshot, previous: Option<&SystemSnapshot>) -> Vec<Finding>;
}

/// Registry of all detectors, supporting domain filtering.
pub struct DetectorRegistry {
    detectors: Vec<Box<dyn Detector>>,
}

impl Default for DetectorRegistry {
    fn default() -> Self {
        Self::default_registry()
    }
}
impl DetectorRegistry {
    pub fn new() -> Self {
        Self {
            detectors: Vec::new(),
        }
    }

    pub fn register(&mut self, detector: Box<dyn Detector>) {
        self.detectors.push(detector);
    }

    /// Run all detectors (optionally filtered by domain) against a snapshot.
    pub fn detect(
        &self,
        snapshot: &SystemSnapshot,
        domains: Option<&[MonitorDomain]>,
        previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        self.detectors
            .iter()
            .filter(|d| domains.is_none_or(|doms| doms.contains(&d.domain())))
            .flat_map(|d| d.detect(snapshot, previous))
            .collect()
    }

    /// Return all registered detector IDs for auditing.
    pub fn detector_ids(&self) -> Vec<&'static str> {
        self.detectors.iter().map(|d| d.id()).collect()
    }

    /// Create the default set of all detectors.
    pub fn default_registry() -> Self {
        Self::default_registry_with_config(&crate::load_thresholds_config())
    }

    pub fn default_registry_with_config(config: &ThresholdsConfig) -> Self {
        let mut reg = Self::new();
        register_if_enabled(
            &mut reg,
            config,
            "disk-usage",
            Box::new(disk_usage::DiskUsageDetector::from_thresholds(
                config.detector_override("disk-usage"),
            )),
        );
        register_if_enabled(
            &mut reg,
            config,
            "inode-usage",
            Box::new(inode_usage::InodeUsageDetector::from_thresholds(
                config.detector_override("inode-usage"),
            )),
        );
        register_if_enabled(
            &mut reg,
            config,
            "smart-health",
            Box::new(smart::SmartHealthDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "filesystem-read-only",
            Box::new(fs_readonly::FilesystemReadOnlyDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "failed-services",
            Box::new(failed_services::FailedServicesDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "service-restart-loop",
            Box::new(restart_loop::ServiceRestartLoopDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "enabled-inactive-service",
            Box::new(inactive_service::EnabledInactiveServiceDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "unhealthy-container",
            Box::new(unhealthy_container::UnhealthyContainerDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "container-restart-loop",
            Box::new(container_restart::ContainerRestartLoopDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "exposed-port",
            Box::new(exposed_port::ExposedPortDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "high-load",
            Box::new(high_load::HighLoadDetector::from_thresholds(
                config.detector_override("high-load"),
            )),
        );
        register_if_enabled(
            &mut reg,
            config,
            "memory-pressure",
            Box::new(memory_pressure::MemoryPressureDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "swap-exhaustion",
            Box::new(swap_exhaustion::SwapExhaustionDetector::from_thresholds(
                config.detector_override("swap-exhaustion"),
            )),
        );
        register_if_enabled(
            &mut reg,
            config,
            "oom-killer",
            Box::new(oom_event::OomKillerDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "journal-error-burst",
            Box::new(journal_errors::JournalErrorBurstDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "stale-backup",
            Box::new(backup_stale::StaleBackupDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "missing-backup-schedule",
            Box::new(backup_schedule::MissingBackupScheduleDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "restore-test-missing",
            Box::new(restore_test::RestoreTestMissingDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "security-updates",
            Box::new(package_updates::SecurityUpdatesDetector),
        );
        // Kubernetes detectors
        register_if_enabled(
            &mut reg,
            config,
            "pod-restart",
            Box::new(pod_restart::PodRestartDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "oom-kill",
            Box::new(oom_kill::OOMKillDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "pvc-pressure",
            Box::new(pvc_pressure::PVCPressureDetector),
        );
        // Libvirt detectors
        register_if_enabled(
            &mut reg,
            config,
            "domain-state",
            Box::new(domain_state::DomainStateDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "snapshot-age",
            Box::new(snapshot_age::SnapshotAgeDetector),
        );
        // Compose detectors
        register_if_enabled(
            &mut reg,
            config,
            "compose-health",
            Box::new(compose_health::ComposeHealthDetector),
        );
        // Cert detectors
        register_if_enabled(
            &mut reg,
            config,
            "cert-expiry",
            Box::new(cert_expiry::CertExpiryDetector),
        );
        // SSH exposure
        register_if_enabled(
            &mut reg,
            config,
            "ssh-exposure",
            Box::new(ssh_exposure::SshExposureDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "db-health",
            Box::new(db_health::DbHealthDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "web-server",
            Box::new(web_server::WebServerDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "config-drift",
            Box::new(config_drift::ConfigDriftDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "desktop-notify",
            Box::new(desktop_notify::DesktopNotifyDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "monitoring-health",
            Box::new(monitoring_health::MonitoringHealthDetector),
        );
        register_if_enabled(
            &mut reg,
            config,
            "dns-health",
            Box::new(dns_health::DnsHealthDetector),
        );
        reg
    }
}

fn register_if_enabled(
    registry: &mut DetectorRegistry,
    config: &ThresholdsConfig,
    detector_id: &'static str,
    detector: Box<dyn Detector>,
) {
    if config.detector_enabled(detector_id) {
        registry.register(detector);
    }
}
