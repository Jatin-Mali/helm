//! Deterministic plan supervision and evidence verification.
//!
//! This module is intentionally model-free.  LLMs may propose actions, but the
//! supervisor owns step state and the verifier decides whether concrete
//! evidence exists for claimed effects.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
};

use helm_core::{ContentBlock, Message};
use serde_json::Value;

/// Deterministic task plan represented as a dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Plan steps keyed by their `id`.
    pub steps: Vec<PlanStep>,
}

impl Plan {
    /// Creates a plan from ordered steps.
    pub fn new(steps: Vec<PlanStep>) -> Self {
        Self { steps }
    }

    /// Returns `true` when the plan contains no duplicate step identifiers and
    /// every dependency points at an existing step.
    pub fn is_valid(&self) -> bool {
        let ids = self
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<BTreeSet<_>>();
        ids.len() == self.steps.len()
            && self.steps.iter().all(|step| {
                step.dependencies
                    .iter()
                    .all(|dep| ids.contains(dep.as_str()))
            })
    }
}

/// One node in a deterministic plan graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    /// Stable step identifier.
    pub id: String,
    /// Human-readable work description.
    pub description: String,
    /// Step ids that must complete before this step may run.
    pub dependencies: Vec<String>,
    /// Evidence required before this step may be marked complete.
    pub expected_evidence: Vec<EvidenceRequest>,
    /// Maximum tool calls allowed while executing this step.
    pub tool_budget: u32,
    /// Maximum retry attempts before the step fails permanently.
    pub max_retries: u32,
}

impl PlanStep {
    /// Creates a plan step with conservative defaults.
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            dependencies: Vec::new(),
            expected_evidence: Vec::new(),
            tool_budget: 8,
            max_retries: 2,
        }
    }
}

/// Evidence the supervisor expects for a step or final task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceRequest {
    /// A file must exist.
    FileExists { path: String },
    /// A file must contain the given text.
    FileContains { path: String, needle: String },
    /// A tool result must contain a specific exit code.
    ExitCode { tool_use_id: String, code: i32 },
    /// A tool result's stdout/content must contain a pattern.
    StdoutMatch {
        tool_use_id: String,
        pattern: String,
    },
    /// A systemd service must have the requested status text.
    ServiceStatus { service: String, status: String },
    /// An HTTP probe must return the requested status code.
    HttpStatus { url: String, status: u16 },
}

/// Concrete evidence observed from filesystem state or tool outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// A file exists at the path.
    FileExists { path: String },
    /// A file contains a required substring.
    FileContains { path: String, needle: String },
    /// A tool result contained an exit code.
    ExitCode { tool_use_id: String, code: i32 },
    /// A tool result matched expected stdout/content text.
    StdoutMatch {
        tool_use_id: String,
        pattern: String,
    },
    /// A service status was observed.
    ServiceStatus { service: String, status: String },
    /// An HTTP status was observed.
    HttpStatus { url: String, status: u16 },
}

/// Result of deterministic evidence verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationResult {
    /// Whether every required evidence item was found and no hard warnings were
    /// produced.
    pub ok: bool,
    /// Concrete evidence found.
    pub evidence: Vec<Evidence>,
    /// Required evidence that was not found.
    pub missing: Vec<String>,
    /// Non-fatal or fatal warnings about suspicious state.
    pub warnings: Vec<String>,
}

impl VerificationResult {
    /// Creates a successful result with the given evidence.
    pub fn ok(evidence: Vec<Evidence>) -> Self {
        Self {
            ok: true,
            evidence,
            missing: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Creates a failed result with one missing-evidence reason.
    pub fn missing(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            evidence: Vec::new(),
            missing: vec![reason.into()],
            warnings: Vec::new(),
        }
    }

    /// Returns all missing evidence and warnings as human-readable lines.
    pub fn problems(&self) -> Vec<String> {
        self.missing
            .iter()
            .chain(self.warnings.iter())
            .cloned()
            .collect()
    }
}

/// Compact record of a tool call and the tool result that answered it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallRecord {
    /// Tool-use id generated by the provider or extractor.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Tool JSON input.
    pub input: Value,
    /// Tool result content, if available.
    pub output: Option<String>,
    /// Whether the tool result was an error.
    pub is_error: bool,
}

/// Deterministic supervisor state for a plan DAG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supervisor {
    plan: Plan,
    states: BTreeMap<String, StepRuntime>,
}

impl Supervisor {
    /// Creates a supervisor for `plan`.
    pub fn new(plan: Plan) -> Result<Self, SupervisorError> {
        if !plan.is_valid() {
            return Err(SupervisorError::InvalidPlan(
                "duplicate step id or unknown dependency".to_owned(),
            ));
        }
        let states = plan
            .steps
            .iter()
            .map(|step| {
                (
                    step.id.clone(),
                    StepRuntime {
                        status: StepStatus::Pending,
                        attempts: 0,
                    },
                )
            })
            .collect();
        Ok(Self { plan, states })
    }

    /// Returns steps that are pending and whose dependencies have completed.
    pub fn runnable_steps(&self) -> Vec<&PlanStep> {
        self.plan
            .steps
            .iter()
            .filter(|step| self.status(&step.id) == Some(StepStatus::Pending))
            .filter(|step| {
                step.dependencies
                    .iter()
                    .all(|dep| self.status(dep) == Some(StepStatus::Complete))
            })
            .collect()
    }

    /// Marks a step as running.
    pub fn start_step(&mut self, step_id: &str) -> Result<(), SupervisorError> {
        if !self.runnable_steps().iter().any(|step| step.id == step_id) {
            return Err(SupervisorError::StepNotRunnable(step_id.to_owned()));
        }
        let Some(runtime) = self.states.get_mut(step_id) else {
            return Err(SupervisorError::UnknownStep(step_id.to_owned()));
        };
        runtime.status = StepStatus::Running;
        runtime.attempts = runtime.attempts.saturating_add(1);
        Ok(())
    }

    /// Applies verification to a running step and advances it to complete,
    /// retrying, or failed.
    pub fn apply_verification(
        &mut self,
        step_id: &str,
        verification: &VerificationResult,
    ) -> Result<StepStatus, SupervisorError> {
        let Some(step) = self.plan.steps.iter().find(|step| step.id == step_id) else {
            return Err(SupervisorError::UnknownStep(step_id.to_owned()));
        };
        let Some(runtime) = self.states.get_mut(step_id) else {
            return Err(SupervisorError::UnknownStep(step_id.to_owned()));
        };
        if runtime.status != StepStatus::Running {
            return Err(SupervisorError::StepNotRunning(step_id.to_owned()));
        }
        runtime.status = if verification.ok {
            StepStatus::Complete
        } else if runtime.attempts <= step.max_retries {
            StepStatus::Retrying
        } else {
            StepStatus::Failed
        };
        Ok(runtime.status)
    }

    /// Moves a retrying step back to pending so it can run again.
    pub fn retry_step(&mut self, step_id: &str) -> Result<(), SupervisorError> {
        let Some(runtime) = self.states.get_mut(step_id) else {
            return Err(SupervisorError::UnknownStep(step_id.to_owned()));
        };
        if runtime.status != StepStatus::Retrying {
            return Err(SupervisorError::StepNotRetrying(step_id.to_owned()));
        }
        runtime.status = StepStatus::Pending;
        Ok(())
    }

    /// Returns the status for a step id.
    pub fn status(&self, step_id: &str) -> Option<StepStatus> {
        self.states.get(step_id).map(|runtime| runtime.status)
    }

    /// Returns true when every step completed.
    pub fn is_complete(&self) -> bool {
        self.states
            .values()
            .all(|runtime| runtime.status == StepStatus::Complete)
    }

    /// Returns true when any step has failed permanently.
    pub fn has_failed(&self) -> bool {
        self.states
            .values()
            .any(|runtime| runtime.status == StepStatus::Failed)
    }
}

/// Runtime status for one supervised step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    /// Not started and waiting for dependencies.
    Pending,
    /// Currently executing.
    Running,
    /// Verifying execution evidence.
    Verifying,
    /// Failed but eligible for another attempt.
    Retrying,
    /// Failed permanently.
    Failed,
    /// Completed with evidence.
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StepRuntime {
    status: StepStatus,
    attempts: u32,
}

/// Error returned by deterministic supervisor operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorError {
    /// The plan graph is invalid.
    InvalidPlan(String),
    /// The requested step id does not exist.
    UnknownStep(String),
    /// The requested step cannot run yet.
    StepNotRunnable(String),
    /// Verification was applied to a step that is not running.
    StepNotRunning(String),
    /// Retry was requested for a step that is not in retrying state.
    StepNotRetrying(String),
}

impl fmt::Display for SupervisorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlan(reason) => write!(formatter, "invalid plan: {reason}"),
            Self::UnknownStep(id) => write!(formatter, "unknown step: {id}"),
            Self::StepNotRunnable(id) => write!(formatter, "step is not runnable: {id}"),
            Self::StepNotRunning(id) => write!(formatter, "step is not running: {id}"),
            Self::StepNotRetrying(id) => write!(formatter, "step is not retrying: {id}"),
        }
    }
}

impl std::error::Error for SupervisorError {}

/// Goal-aware deterministic verifier for files, tool output, services, and HTTP
/// status evidence.
#[derive(Debug, Clone)]
pub struct EvidenceVerifier {
    working_dir: PathBuf,
}

impl EvidenceVerifier {
    /// Creates a verifier rooted at `working_dir` for resolving relative paths.
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            working_dir: working_dir.into(),
        }
    }

    /// Verifies concrete postconditions implied by a user goal and conversation.
    pub fn verify_goal(&self, goal: &str, messages: &[Message]) -> VerificationResult {
        self.verify_records(goal, &records_from_messages(messages))
    }

    /// Verifies concrete postconditions implied by a user goal and tool records.
    pub fn verify_records(&self, goal: &str, records: &[ToolCallRecord]) -> VerificationResult {
        let mut evidence = Vec::new();
        let mut missing = Vec::new();
        let mut warnings = Vec::new();

        for record in records {
            if let Some(output) = &record.output {
                if let Some(code) = parse_exit_code(output) {
                    evidence.push(Evidence::ExitCode {
                        tool_use_id: record.id.clone(),
                        code,
                    });
                }
                if record.name == "service" {
                    if let Some(service) = record.input.get("unit").and_then(Value::as_str) {
                        let status = if output.contains("Active: active")
                            || output.contains("active (running)")
                            || output.contains("\"active\"")
                        {
                            "active"
                        } else if output.contains("inactive") {
                            "inactive"
                        } else if output.contains("failed") {
                            "failed"
                        } else {
                            "unknown"
                        };
                        evidence.push(Evidence::ServiceStatus {
                            service: service.to_owned(),
                            status: status.to_owned(),
                        });
                    }
                }
                if record.name == "network" {
                    if let Some(url) = record.input.get("url").and_then(Value::as_str) {
                        if let Some(status) = parse_http_status(output) {
                            evidence.push(Evidence::HttpStatus {
                                url: url.to_owned(),
                                status,
                            });
                        }
                    }
                }
            }
        }

        let mut written_paths = written_paths_from_records(records);
        if written_paths.is_empty() && goal_requests_file_write(goal) {
            written_paths.extend(extract_goal_paths(goal));
        }
        written_paths.sort();
        written_paths.dedup();

        for path in written_paths {
            let abs_path = resolve_path(&self.working_dir, &path);
            match std::fs::read_to_string(&abs_path) {
                Ok(content) => {
                    evidence.push(Evidence::FileExists { path: path.clone() });
                    if content.trim().is_empty() {
                        missing.push(format!("file is empty: {path}"));
                    }
                    if goal_requests_command_output(goal)
                        && (content.contains("$(")
                            || content.contains("${")
                            || content.contains('`'))
                    {
                        warnings.push(format!("file contains unexpanded shell syntax: {path}"));
                    }
                    if goal_mentions_uname(goal) && !content.contains("Linux") {
                        missing.push(format!("file is missing uname/Linux output: {path}"));
                    }
                    if goal_mentions_date(goal) && !contains_four_digit_year(&content) {
                        missing.push(format!("file is missing expanded date output: {path}"));
                    }
                    if goal_has_multiple_commands(goal) && content.lines().count() < 2 {
                        missing.push(format!(
                            "goal mentions multiple commands but file has fewer than 2 lines: {path}"
                        ));
                    }
                }
                Err(error) => {
                    missing.push(format!(
                        "file does not exist or cannot be read: {path}: {error}"
                    ));
                }
            }
        }

        VerificationResult {
            ok: missing.is_empty() && warnings.is_empty(),
            evidence,
            missing,
            warnings,
        }
    }
}

fn records_from_messages(messages: &[Message]) -> Vec<ToolCallRecord> {
    let mut records = Vec::new();
    let mut by_id = BTreeMap::<String, usize>::new();
    for message in messages {
        for block in &message.content {
            match block {
                ContentBlock::ToolUse { id, name, input } => {
                    by_id.insert(id.clone(), records.len());
                    records.push(ToolCallRecord {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                        output: None,
                        is_error: false,
                    });
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    if let Some(index) = by_id.get(tool_use_id).copied() {
                        if let Some(record) = records.get_mut(index) {
                            record.output = Some(content.clone());
                            record.is_error = *is_error;
                        }
                    }
                }
                ContentBlock::Text(_) => {}
            }
        }
    }
    records
}

fn written_paths_from_records(records: &[ToolCallRecord]) -> Vec<String> {
    records
        .iter()
        .filter_map(|record| {
            if record.name == "fs_write" {
                return record
                    .input
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            if record.name == "shell" {
                return record
                    .input
                    .get("redirect_stdout_to")
                    .or_else(|| record.input.get("redirect_stderr_to"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            None
        })
        .collect()
}

fn resolve_path(working_dir: &Path, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        working_dir.join(candidate)
    }
}

fn parse_exit_code(output: &str) -> Option<i32> {
    let marker = "[exit code:";
    let start = output.find(marker)?;
    let rest = output.get(start + marker.len()..)?;
    let end = rest.find(']')?;
    rest.get(..end)?.trim().parse().ok()
}

fn parse_http_status(output: &str) -> Option<u16> {
    for token in output.split(|ch: char| !ch.is_ascii_digit()) {
        if token.len() == 3 {
            if let Ok(status) = token.parse::<u16>() {
                if (100..=599).contains(&status) {
                    return Some(status);
                }
            }
        }
    }
    None
}

fn goal_requests_file_write(goal: &str) -> bool {
    let lower = goal.to_ascii_lowercase();
    (lower.contains("create") || lower.contains("write") || lower.contains("save"))
        && (lower.contains(".txt") || lower.contains("/tmp/") || lower.contains("file"))
}

fn goal_requests_command_output(goal: &str) -> bool {
    let lower = goal.to_ascii_lowercase();
    lower.contains("output of")
        || lower.contains("current date")
        || lower.contains("uname")
        || lower.contains("date")
}

fn goal_has_multiple_commands(goal: &str) -> bool {
    let lower = goal.to_ascii_lowercase();
    lower.contains("&&") || (lower.contains(" and ") && goal_requests_command_output(goal))
}

fn goal_mentions_uname(goal: &str) -> bool {
    goal.to_ascii_lowercase().contains("uname")
}

fn goal_mentions_date(goal: &str) -> bool {
    let lower = goal.to_ascii_lowercase();
    lower.contains("date") || lower.contains("current") || lower.contains("today")
}

fn contains_four_digit_year(text: &str) -> bool {
    text.as_bytes()
        .windows(4)
        .any(|window| window.iter().all(u8::is_ascii_digit))
}

fn extract_goal_paths(goal: &str) -> Vec<String> {
    goal.split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| matches!(ch, ',' | '"' | '\'' | ':' | ';' | ')' | '('))
        })
        .filter(|token| token.starts_with('/') && token.contains('/'))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use helm_core::{ContentBlock, Message};
    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        Evidence, EvidenceVerifier, Plan, PlanStep, StepStatus, Supervisor, VerificationResult,
    };

    #[test]
    fn supervisor_runs_dependency_dag_happy_path() {
        let mut first = PlanStep::new("inspect", "inspect state");
        first.max_retries = 1;
        let mut second = PlanStep::new("fix", "apply fix");
        second.dependencies = vec!["inspect".to_owned()];
        let plan = Plan::new(vec![first, second]);
        let mut supervisor = Supervisor::new(plan).unwrap();

        assert_eq!(supervisor.runnable_steps()[0].id, "inspect");
        supervisor.start_step("inspect").unwrap();
        assert_eq!(
            supervisor.apply_verification("inspect", &VerificationResult::ok(Vec::new())),
            Ok(StepStatus::Complete)
        );
        assert_eq!(supervisor.runnable_steps()[0].id, "fix");
    }

    #[test]
    fn supervisor_retries_then_fails_error_path() {
        let mut step = PlanStep::new("write", "write file");
        step.max_retries = 1;
        let mut supervisor = Supervisor::new(Plan::new(vec![step])).unwrap();

        supervisor.start_step("write").unwrap();
        assert_eq!(
            supervisor.apply_verification("write", &VerificationResult::missing("missing file")),
            Ok(StepStatus::Retrying)
        );
        supervisor.retry_step("write").unwrap();
        supervisor.start_step("write").unwrap();
        assert_eq!(
            supervisor.apply_verification("write", &VerificationResult::missing("missing file")),
            Ok(StepStatus::Failed)
        );
        assert!(supervisor.has_failed());
    }

    #[test]
    fn supervisor_rejects_unknown_dependency_edge_case() {
        let mut step = PlanStep::new("fix", "fix");
        step.dependencies = vec!["missing".to_owned()];

        let error = Supervisor::new(Plan::new(vec![step])).unwrap_err();

        assert!(error.to_string().contains("invalid plan"));
    }

    #[test]
    fn verifier_requires_file_to_exist() {
        let dir = tempdir().unwrap();
        let verifier = EvidenceVerifier::new(dir.path());
        let messages = vec![Message::assistant(vec![ContentBlock::ToolUse {
            id: "call_1".to_owned(),
            name: "fs_write".to_owned(),
            input: json!({"path": dir.path().join("missing.txt")}),
        }])];

        let result = verifier.verify_goal("create file /tmp/x.txt", &messages);

        assert!(!result.ok);
        assert!(
            result
                .missing
                .iter()
                .any(|problem| problem.contains("does not exist"))
        );
    }

    #[test]
    fn verifier_accepts_date_and_uname_file_happy_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("out.txt");
        std::fs::write(&path, "Tuesday 05 May 2026\nLinux PHANTOM").unwrap();
        let verifier = EvidenceVerifier::new(dir.path());
        let messages = vec![Message::assistant(vec![ContentBlock::ToolUse {
            id: "call_1".to_owned(),
            name: "shell".to_owned(),
            input: json!({"command": "date && uname -a", "redirect_stdout_to": path}),
        }])];

        let result =
            verifier.verify_goal("create file with output of date and uname -a", &messages);

        assert!(result.ok, "{result:?}");
        assert!(
            result
                .evidence
                .iter()
                .any(|evidence| matches!(evidence, Evidence::FileExists { .. }))
        );
    }

    #[test]
    fn verifier_rejects_unexpanded_shell_literals_edge_case() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("out.txt");
        std::fs::write(&path, "$(date)\n$(uname -a)").unwrap();
        let verifier = EvidenceVerifier::new(dir.path());
        let messages = vec![Message::assistant(vec![ContentBlock::ToolUse {
            id: "call_1".to_owned(),
            name: "fs_write".to_owned(),
            input: json!({"path": path}),
        }])];

        let result =
            verifier.verify_goal("create file with output of date and uname -a", &messages);

        assert!(!result.ok);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("unexpanded shell"))
        );
    }

    #[test]
    fn verifier_extracts_service_and_exit_evidence() {
        let dir = tempdir().unwrap();
        let verifier = EvidenceVerifier::new(dir.path());
        let messages = vec![
            Message::assistant(vec![ContentBlock::ToolUse {
                id: "svc_1".to_owned(),
                name: "service".to_owned(),
                input: json!({"action": "status", "unit": "nginx"}),
            }]),
            Message::tool_results(vec![ContentBlock::ToolResult {
                tool_use_id: "svc_1".to_owned(),
                content: "Active: active (running)\n[exit code: 0]".to_owned(),
                is_error: false,
            }]),
        ];

        let result = verifier.verify_goal("show nginx status", &messages);

        assert!(result.ok);
        assert!(result.evidence.iter().any(|evidence| {
            matches!(
                evidence,
                Evidence::ServiceStatus { service, status }
                    if service == "nginx" && status == "active"
            )
        }));
        assert!(result.evidence.iter().any(|evidence| {
            matches!(
                evidence,
                Evidence::ExitCode { tool_use_id, code }
                    if tool_use_id == "svc_1" && *code == 0
            )
        }));
    }
}
