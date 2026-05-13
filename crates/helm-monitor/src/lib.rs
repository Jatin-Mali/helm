//! Monitoring engine for HELM — typed system snapshots, collectors, and detectors.

pub mod collectors;
pub mod detectors;
pub mod engine;
pub mod findings;
pub mod reporter;
pub mod snapshot;
pub mod troubleshoot;

pub use engine::collect_snapshot;
pub use findings::{Confidence, EvidenceRef, Finding, FindingId, MonitorDomain, PlanId, Severity};
pub use reporter::{MonitorReport, MonitorReporter};
pub use snapshot::{
    BackupSnapshot, BackupTool, BlockDevice, CollectorError, ContainerInfo, ContainerRuntime,
    ContainerSnapshot, CronJob, DiskSnapshot, FailedUnit, FilesystemEntry, FirewallSnapshot,
    HostIdentity, InodeEntry, InterfaceEntry, ListenerEntry, LoadAverage, LoadSnapshot,
    LogSnapshot, MemoryInfo, MonitorProfile, MountEntry, NetworkSnapshot, PackageSnapshot,
    PortSnapshot, PressureStall, ProcessInfo, ProcessSnapshot, RouteEntry, ServiceSnapshot,
    SmartDevice, SnapshotDomains, SystemSnapshot, SystemdTimer, SystemdUnit, TimerSnapshot,
};
pub use troubleshoot::{
    BlastRadius, CommandPreview, Hypothesis, PlanSource, PlanStep, RiskLevel, RollbackStatus,
    TroubleshootingPlan, explain_finding, plan_from_finding, plan_from_problem,
    plan_from_problem_with_snapshot,
};
