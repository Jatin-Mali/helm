//! Monitoring engine for HELM — typed system snapshots, collectors, and detectors.

pub mod alerting;
pub mod collectors;
pub mod correlation;
pub mod custom_rules;
pub mod custom_service;
pub mod detectors;
pub mod engine;
pub mod execute;
pub mod findings;
pub mod log_analysis;
pub mod metric_history;
pub mod pipeline;
pub mod plan_cache;
pub mod plan_document;
pub mod reporter;
pub mod ring_buffer;
pub mod snapshot;
pub mod thresholds;
pub mod troubleshoot;

pub use correlation::{
    CausalLink, CausalRelationship, CorrelationEngine, CorrelationGroup, CorrelationResult,
};
pub use custom_rules::{
    CompareOp, CustomRule, CustomRulesEngine, LogicOp, RuleCondition, RuleValidationError,
    ThresholdValue, default_rules_path,
};
pub use custom_service::{
    DbKind, RegisteredService, ServiceRegistry, ServiceRegistryError, ServiceType,
    default_services_path,
};
pub use engine::{AlertingEngine, collect_snapshot};
pub use execute::{
    ChangeSet, ChangeSetStatus, ChangeSetStep, ExecutionEngine, PreChangeBackup, StepOutcome,
    StepStatus, format_change_set,
};
pub use findings::{
    Confidence, EvidenceRef, Finding, FindingId, FindingLifecycle, FindingValidationError,
    MonitorDomain, PlanId, Severity, self_resolved_findings, sort_findings,
};
pub use log_analysis::{
    LogPatternFilter, LogTimeRange, filter_log_patterns, log_pattern_ids_new_since_previous,
    normalize_log_message,
};
pub use metric_history::MetricHistory;
pub use pipeline::{
    PipelineCollector, PipelineProvider, PipelineSnapshot, PipelineStatus, PipelineWebhookError,
    default_pipeline_history_path, pipeline_failure_findings, record_pipeline_webhook_event,
};
pub use plan_cache::{DEFAULT_PLAN_CACHE_TTL_SECS, PlanCache};
pub use plan_document::{
    PlanApprovals, decode_plan_steps, decode_read_only_steps, encode_plan_steps, llm_fix_steps,
};
pub use reporter::{MonitorReport, MonitorReporter};
pub use ring_buffer::RingBuffer;
pub use snapshot::{
    BackupSnapshot, BackupTool, BlockDevice, CollectorError, ContainerInfo, ContainerRuntime,
    ContainerSnapshot, CronJob, DiskSnapshot, FailedUnit, FilesystemEntry, FirewallSnapshot,
    HostIdentity, InodeEntry, InterfaceEntry, JournalContextLine, JournalPattern, ListenerEntry,
    LoadAverage, LoadSnapshot, LogSeverity, LogSnapshot, MemoryInfo, MonitorProfile, MountEntry,
    NetworkSnapshot, PackageSnapshot, PortSnapshot, PressureStall, ProcessInfo, ProcessSnapshot,
    RouteEntry, ServiceSnapshot, SmartDevice, SnapshotDomains, SystemSnapshot, SystemdTimer,
    SystemdUnit, TimerSnapshot,
};
pub use thresholds::{
    DetectorThresholdOverride, ThresholdsConfig, ThresholdsLoadOutcome, default_thresholds_path,
    load_thresholds_config, load_thresholds_config_from_path,
};
pub use troubleshoot::{
    BlastRadius, CommandPreview, CommandValidator, FindingSummaryFields, Hypothesis, LlmFixPlan,
    LlmFixStep, LlmNarrative, LlmPlanStatus, MissingBinaryWarning, ParseError, PlanSource,
    PlanStep, PlanValidationError, RiskLevel, RollbackStatus, TroubleshootingPlan,
    build_narrative_prompt, explain_finding, parse_llm_response, plan_from_finding,
    plan_from_problem, plan_from_problem_with_snapshot, validate_fix_plan_steps,
};
