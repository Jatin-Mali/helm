//! Monitoring engine for HELM — typed system snapshots, collectors, and detectors.

pub mod alerting;
pub mod collectors;
pub mod detectors;
pub mod engine;
pub mod execute;
pub mod findings;
pub mod metric_history;
pub mod reporter;
pub mod ring_buffer;
pub mod snapshot;
pub mod thresholds;
pub mod troubleshoot;

pub use engine::{AlertingEngine, collect_snapshot};
pub use execute::{
    ChangeSet, ChangeSetStatus, ChangeSetStep, ExecutionEngine, PreChangeBackup, StepOutcome,
    StepStatus, format_change_set,
};
pub use findings::{
    Confidence, EvidenceRef, Finding, FindingId, FindingLifecycle, MonitorDomain, PlanId, Severity,
};
pub use metric_history::MetricHistory;
pub use reporter::{MonitorReport, MonitorReporter};
pub use ring_buffer::RingBuffer;
pub use snapshot::{
    BackupSnapshot, BackupTool, BlockDevice, CollectorError, ContainerInfo, ContainerRuntime,
    ContainerSnapshot, CronJob, DiskSnapshot, FailedUnit, FilesystemEntry, FirewallSnapshot,
    HostIdentity, InodeEntry, InterfaceEntry, ListenerEntry, LoadAverage, LoadSnapshot,
    LogSnapshot, MemoryInfo, MonitorProfile, MountEntry, NetworkSnapshot, PackageSnapshot,
    PortSnapshot, PressureStall, ProcessInfo, ProcessSnapshot, RouteEntry, ServiceSnapshot,
    SmartDevice, SnapshotDomains, SystemSnapshot, SystemdTimer, SystemdUnit, TimerSnapshot,
};
pub use thresholds::{
    DetectorThresholdOverride, ThresholdsConfig, ThresholdsLoadOutcome, default_thresholds_path,
    load_thresholds_config, load_thresholds_config_from_path,
};
pub use troubleshoot::{
    BlastRadius, CommandPreview, CommandValidator, FindingSummaryFields, Hypothesis, LlmFixPlan,
    LlmFixStep, LlmNarrative, LlmPlanStatus, MissingBinaryWarning, ParseError, PlanSource,
    PlanStep, RiskLevel, RollbackStatus, TroubleshootingPlan, build_narrative_prompt,
    explain_finding, parse_llm_response, plan_from_finding, plan_from_problem,
    plan_from_problem_with_snapshot,
};
