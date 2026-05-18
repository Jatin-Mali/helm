//! Detector trait and registry. Per TRD §4.3.

pub mod backup_schedule;
pub mod backup_stale;
pub mod compose_health;
pub mod container_restart;
pub mod disk_usage;
pub mod domain_state;
pub mod exposed_port;
pub mod failed_services;
pub mod fs_readonly;
pub mod high_load;
pub mod inactive_service;
pub mod inode_usage;
pub mod journal_errors;
pub mod memory_pressure;
pub mod oom_event;
pub mod oom_kill;
pub mod package_updates;
pub mod pod_restart;
pub mod pvc_pressure;
pub mod restart_loop;
pub mod restore_test;
pub mod smart;
pub mod snapshot_age;
pub mod swap_exhaustion;
pub mod unhealthy_container;

use crate::{
    findings::{Finding, MonitorDomain},
    snapshot::SystemSnapshot,
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
        let mut reg = Self::new();
        reg.register(Box::new(disk_usage::DiskUsageDetector));
        reg.register(Box::new(inode_usage::InodeUsageDetector));
        reg.register(Box::new(smart::SmartHealthDetector));
        reg.register(Box::new(fs_readonly::FilesystemReadOnlyDetector));
        reg.register(Box::new(failed_services::FailedServicesDetector));
        reg.register(Box::new(restart_loop::ServiceRestartLoopDetector));
        reg.register(Box::new(inactive_service::EnabledInactiveServiceDetector));
        reg.register(Box::new(unhealthy_container::UnhealthyContainerDetector));
        reg.register(Box::new(container_restart::ContainerRestartLoopDetector));
        reg.register(Box::new(exposed_port::ExposedPortDetector));
        reg.register(Box::new(high_load::HighLoadDetector));
        reg.register(Box::new(memory_pressure::MemoryPressureDetector));
        reg.register(Box::new(swap_exhaustion::SwapExhaustionDetector));
        reg.register(Box::new(oom_event::OomKillerDetector));
        reg.register(Box::new(journal_errors::JournalErrorBurstDetector));
        reg.register(Box::new(backup_stale::StaleBackupDetector));
        reg.register(Box::new(backup_schedule::MissingBackupScheduleDetector));
        reg.register(Box::new(restore_test::RestoreTestMissingDetector));
        reg.register(Box::new(package_updates::SecurityUpdatesDetector));
        // Kubernetes detectors
        reg.register(Box::new(pod_restart::PodRestartDetector));
        reg.register(Box::new(oom_kill::OOMKillDetector));
        reg.register(Box::new(pvc_pressure::PVCPressureDetector));
        // Libvirt detectors
        reg.register(Box::new(domain_state::DomainStateDetector));
        reg.register(Box::new(snapshot_age::SnapshotAgeDetector));
        // Compose detectors
        reg.register(Box::new(compose_health::ComposeHealthDetector));
        reg
    }
}
