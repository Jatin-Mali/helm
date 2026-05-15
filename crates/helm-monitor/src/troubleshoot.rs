//! Guided troubleshooting types and planner. Per TRD §4.4, §4.5 and ROADMAP v1.9.

use serde::{Deserialize, Serialize};

use crate::{
    collect_snapshot,
    findings::{Finding, FindingId, MonitorDomain},
    snapshot::{MonitorProfile, SnapshotId, SystemSnapshot},
};

// ── TRD §4.5 CommandPreview ────────────────────────────────────────────────

/// Risk level for a command preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Read-only inspection, no side effects.
    None,
    /// Low risk — inspection with minor system impact (e.g. reading logs).
    Low,
    /// Medium risk — safe mutation with clear rollback (e.g. removing temp files).
    Medium,
    /// High risk — irreversible or service-affecting change.
    High,
}

impl RiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Affected scope for a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlastRadius {
    /// Affects a single file.
    File(String),
    /// Affects a single process.
    Process(u32),
    /// Affects a service.
    Service(String),
    /// Affects a directory tree.
    Directory(String),
    /// Affects the entire system.
    System,
    /// Unknown scope.
    Unknown,
}

impl std::fmt::Display for BlastRadius {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(p) => write!(f, "file: {p}"),
            Self::Process(pid) => write!(f, "process: {pid}"),
            Self::Service(s) => write!(f, "service: {s}"),
            Self::Directory(d) => write!(f, "directory: {d}"),
            Self::System => write!(f, "entire system"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Whether rollback is possible and what it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RollbackStatus {
    /// Rollback is available and described.
    Available(String),
    /// Rollback is not supported for this action.
    Unsupported,
    /// There is nothing to roll back (read-only action).
    NotNeeded,
}

impl std::fmt::Display for RollbackStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Available(d) => write!(f, "rollback: {d}"),
            Self::Unsupported => write!(f, "no rollback available"),
            Self::NotNeeded => write!(f, "read-only, no rollback needed"),
        }
    }
}

/// Preview of a single command before approval. Per TRD §4.5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandPreview {
    pub tool: String,
    pub input: serde_json::Value,
    pub command_text: Option<String>,
    pub expected_effect: String,
    pub risk: RiskLevel,
    pub blast_radius: BlastRadius,
    pub rollback: RollbackStatus,
    pub verification: Vec<CommandPreview>,
}

impl CommandPreview {
    pub fn new(tool: &str, command_text: &str, expected_effect: &str) -> Self {
        Self {
            tool: tool.into(),
            input: serde_json::json!({"command": command_text}),
            command_text: Some(command_text.into()),
            expected_effect: expected_effect.into(),
            risk: RiskLevel::Low,
            blast_radius: BlastRadius::Unknown,
            rollback: RollbackStatus::NotNeeded,
            verification: Vec::new(),
        }
    }

    pub fn with_risk(mut self, risk: RiskLevel) -> Self {
        self.risk = risk;
        self
    }

    pub fn with_blast(mut self, radius: BlastRadius) -> Self {
        self.blast_radius = radius;
        self
    }

    pub fn with_rollback(mut self, rollback: RollbackStatus) -> Self {
        self.rollback = rollback;
        self
    }

    pub fn with_verification(mut self, cmd: CommandPreview) -> Self {
        self.verification.push(cmd);
        self
    }
}

// ── CommandValidator: display-time blocklist + PATH check ────────────────────

use std::path::PathBuf;
use std::sync::OnceLock;

/// Warning for a binary referenced in a command that is not found in PATH.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingBinaryWarning {
    pub binary: String,
    pub warning: String,
}

/// Describes a single dangerous pattern rule (internal, not exported).
struct DangerPattern {
    regex: regex::Regex,
    description: &'static str,
}

/// Compiled once at first use.
fn danger_patterns() -> &'static [DangerPattern] {
    static PATTERNS: OnceLock<Vec<DangerPattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            DangerPattern {
                regex: regex::Regex::new(r"rm\s+-rf\s+/").unwrap(),
                description: "catastrophic recursive removal from root (rm -rf /)",
            },
            DangerPattern {
                regex: regex::Regex::new(r"mkfs\.[a-z]+").unwrap(),
                description: "filesystem creation — destroys data (mkfs.*)",
            },
            DangerPattern {
                regex: regex::Regex::new(r"dd\s+if=.*of=/dev/[a-z]+").unwrap(),
                description: "raw device write (dd of=/dev/*)",
            },
            DangerPattern {
                regex: regex::Regex::new(r"chmod\s+777\s+/").unwrap(),
                description: "world-writable root (chmod 777 /)",
            },
            DangerPattern {
                regex: regex::Regex::new(r":.*\(\)\s*\{\s*:\|:&\s*\}\s*;\s*:").unwrap(),
                description: "fork bomb (:(){ :|:& };:)",
            },
            DangerPattern {
                regex: regex::Regex::new(r">\s*/dev/[a-z]+").unwrap(),
                description: "redirect to raw device (> /dev/*)",
            },
        ]
    })
}

/// Display-time command validator that catches dangerous LLM-generated commands
/// before showing them to the operator.
pub struct CommandValidator;

impl CommandValidator {
    /// Validate a command string against the built-in dangerous pattern blocklist.
    ///
    /// Returns `Ok(())` if no dangerous patterns matched, or `Err(list of matched
    /// pattern descriptions)` if any pattern matched.
    pub fn validate_command(cmd: &str) -> Result<(), Vec<String>> {
        let matched: Vec<String> = danger_patterns()
            .iter()
            .filter(|p| p.regex.is_match(cmd))
            .map(|p| p.description.to_string())
            .collect();
        if matched.is_empty() {
            Ok(())
        } else {
            Err(matched)
        }
    }

    /// Check whether a binary exists and is executable in `PATH`.
    ///
    /// Returns `Some(PathBuf)` to the binary if found and executable, or `None`.
    pub fn check_binary_in_path(binary: &str) -> Option<PathBuf> {
        let path_var = std::env::var_os("PATH")?;
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(binary);
            if let Ok(meta) = std::fs::metadata(&candidate) {
                if meta.is_file() {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = meta.permissions().mode();
                    if mode & 0o111 != 0 {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    /// Scan a command string for all binaries and return warnings for any that
    /// are not found in `PATH`. Handles multi-command strings separated by `&&`,
    /// `;`, `|`, or newlines. Shell builtins (cd, echo, export, etc.) are
    /// skipped.
    pub fn check_all_binaries(cmd: &str) -> Vec<MissingBinaryWarning> {
        let mut warnings = Vec::new();

        // Split on common command separators — keep it simple and deterministic.
        for part in split_command_parts(cmd) {
            if part.is_empty() {
                continue;
            }
            let binary = part.split_whitespace().next().unwrap_or(&part);
            if is_shell_builtin(binary) {
                continue;
            }
            if Self::check_binary_in_path(binary).is_none() {
                warnings.push(MissingBinaryWarning {
                    binary: binary.to_string(),
                    warning: format!("binary '{binary}' not found in PATH"),
                });
            }
        }
        warnings
    }
}

/// Split a command string into individual command parts, splitting on `&&`, `;`,
/// `|`, and newlines.
fn split_command_parts(cmd: &str) -> Vec<String> {
    let separators = ["&&", ";", "|", "\n"];
    let mut parts = vec![cmd.to_string()];
    for sep in &separators {
        parts = parts
            .into_iter()
            .flat_map(|p| {
                p.split(sep)
                    .map(|s| s.trim().to_string())
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    parts
}

/// Known shell builtins and keywords that don't have PATH entries.
fn is_shell_builtin(word: &str) -> bool {
    matches!(
        word,
        "cd" | "echo"
            | "export"
            | "source"
            | "alias"
            | "unalias"
            | "bg"
            | "fg"
            | "jobs"
            | "kill"
            | "wait"
            | "disown"
            | "read"
            | "set"
            | "unset"
            | "shift"
            | "exec"
            | "exit"
            | "return"
            | "break"
            | "continue"
            | "eval"
            | "let"
            | "local"
            | "declare"
            | "typeset"
            | "readonly"
            | "getopts"
            | "history"
            | "logout"
            | "suspend"
            | "trap"
            | "type"
            | "ulimit"
            | "umask"
            | "true"
            | "false"
            | "times"
            | "test"
            | "["
            | "if"
            | "then"
            | "else"
            | "elif"
            | "fi"
            | "case"
            | "esac"
            | "for"
            | "while"
            | "until"
            | "do"
            | "done"
            | "in"
            | "select"
            | "function"
            | "time"
    )
}

// ── LLM Narrative + Fix Plan types ──────────────────────────────────────────

/// Summary of a Finding's key fields, suitable for feeding into an LLM prompt
/// without exposing the full internal structure.
pub struct FindingSummaryFields {
    pub title: String,
    pub severity: String,
    pub affected_resource: String,
    pub evidence_summaries: Vec<String>,
    pub detector_id: String,
    pub impact: String,
}

impl From<&crate::findings::Finding> for FindingSummaryFields {
    fn from(f: &crate::findings::Finding) -> Self {
        Self {
            title: f.title.clone(),
            severity: format!("{:?}", f.severity),
            affected_resource: f.affected_resource.clone(),
            evidence_summaries: f
                .evidence
                .iter()
                .map(|e| format!("{}: {} ({})", e.source, e.value, e.note))
                .collect(),
            detector_id: f.detector_id.clone(),
            impact: f.impact.clone(),
        }
    }
}

/// An LLM-generated explanation of WHY a finding happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmNarrative {
    pub text: String,
}

/// A single validated fix command from the LLM response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmFixStep {
    pub command: String,
    pub purpose: String,
    pub risk: RiskLevel,
    pub rollback: String,
    #[serde(default)]
    pub binary_warnings: Vec<String>,
}

/// Complete LLM-generated fix plan: narrative explanation + validated commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmFixPlan {
    pub narrative: LlmNarrative,
    pub steps: Vec<LlmFixStep>,
    pub generated_at: i64,
}

/// The status of an in-flight or completed LLM plan generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LlmPlanStatus {
    /// Still waiting for the LLM response.
    Loading,
    /// Successfully generated and parsed.
    Ready(LlmFixPlan),
    /// LLM call timed out — operator may retry.
    Timeout,
    /// Rate limited — seconds until the next attempt.
    RateLimited(u64),
    /// Authentication/configuration failure.
    AuthFailed,
    /// Response was received but couldn't be parsed.
    MalformedResponse,
    /// Response included a command blocked by the validator.
    DangerousCommand(String),
}

impl std::fmt::Display for LlmPlanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Loading => write!(f, "Generating fix plan…"),
            Self::Ready(plan) => {
                write!(f, "Fix plan ready ({} step(s))", plan.steps.len())
            }
            Self::Timeout => write!(f, "LLM call timed out — retry?"),
            Self::RateLimited(secs) => {
                write!(f, "Rate limited — retry in {}s", secs)
            }
            Self::AuthFailed => {
                write!(f, "Authentication failed — check provider config")
            }
            Self::MalformedResponse => {
                write!(f, "LLM response was malformed — check debug log")
            }
            Self::DangerousCommand(pattern) => {
                write!(f, "Blocked dangerous command: {pattern}")
            }
        }
    }
}

// ── Prompt builder ──────────────────────────────────────────────────────────

/// Build the system + user prompt string for the LLM.
///
/// The prompt instructs the LLM to respond in a structured format that
/// `parse_llm_response` can extract.  No upstream URLs are included — fork
/// identity references use `https://github.com/Jatin-Mali/helm`.
pub fn build_narrative_prompt(finding: &FindingSummaryFields) -> String {
    let mut prompt = String::new();

    // System context
    prompt.push_str("You are a Linux system troubleshooter for the HELM monitoring tool (https://github.com/Jatin-Mali/helm).\n\n");

    // Finding details
    prompt.push_str("=== FINDING ===\n");
    prompt.push_str(&format!("Title: {}\n", finding.title));
    prompt.push_str(&format!("Severity: {}\n", finding.severity));
    prompt.push_str(&format!("Resource: {}\n", finding.affected_resource));
    prompt.push_str(&format!("Detector: {}\n", finding.detector_id));
    prompt.push_str(&format!("Impact: {}\n", finding.impact));

    if !finding.evidence_summaries.is_empty() {
        prompt.push_str("Evidence:\n");
        for ev in &finding.evidence_summaries {
            prompt.push_str(&format!("  - {ev}\n"));
        }
    }
    prompt.push('\n');

    // Output format instructions
    prompt.push_str(
        "Analyze this finding and respond in the following structured format:\n\
\n\
First, provide a 3–5 sentence narrative explaining WHY this happened on this Linux host.\n\
Then, provide a fix plan with exact commands.\n\
\n\
Respond EXACTLY in this format:\n\
\n\
---NARRATIVE---\n\
(3-5 sentence explanation of why this happened on this Linux host)\n\
---FIX PLAN---\n\
COMMAND: <exact command to run>\n\
PURPOSE: <what this command does>\n\
RISK: <none|low|medium|high>\n\
ROLLBACK: <command to undo, or \"none\">\n\
---\n\
COMMAND: <second command if needed>\n\
PURPOSE: <what it does>\n\
RISK: <none|low|medium|high>\n\
ROLLBACK: <command to undo, or \"none\">\n\
---\n\
\n\
Rules:\n\
- COMMAND, PURPOSE, RISK, and ROLLBACK are required for each step.\n\
- RISK must be one of: none, low, medium, high.\n\
- If no rollback is possible, write ROLLBACK: none.\n\
- Each step must be separated by --- on its own line.\n\
- Use only Linux commands safe for production hosts.\n\
- Do not use rm -rf /, mkfs, dd to raw devices, chmod 777 /, fork bombs, or redirects to /dev/* devices.\n",
    );

    prompt
}

// ── Response parser ─────────────────────────────────────────────────────────

/// Errors that can occur while parsing an LLM response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The ---NARRATIVE--- section was not found.
    MissingNarrative,
    /// The ---FIX PLAN--- section was not found.
    MissingFixPlan,
    /// The fix plan section was present but contained no COMMAND blocks.
    NoCommandsFound,
    /// A RISK field contained an unrecognised value.
    InvalidRiskLevel(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingNarrative => write!(f, "LLM response missing ---NARRATIVE--- section"),
            Self::MissingFixPlan => write!(f, "LLM response missing ---FIX PLAN--- section"),
            Self::NoCommandsFound => write!(f, "Fix plan contains no COMMAND blocks"),
            Self::InvalidRiskLevel(v) => write!(
                f,
                "Unknown risk level '{v}' — expected none|low|medium|high"
            ),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse an LLM response string into an [`LlmFixPlan`].
///
/// The expected format is:
/// ```text
/// ---NARRATIVE---
/// (explanation text)
/// ---FIX PLAN---
/// COMMAND: <cmd>
/// PURPOSE: <why>
/// RISK: <none|low|medium|high>
/// ROLLBACK: <undo or "none">
/// ---
/// ```
pub fn parse_llm_response(text: &str) -> Result<LlmFixPlan, ParseError> {
    // Extract the narrative section.
    let narrative_text = extract_section(text, "---NARRATIVE---", "---FIX PLAN---")
        .ok_or(ParseError::MissingNarrative)?
        .trim()
        .to_string();

    // Extract the fix plan section (everything after ---FIX PLAN---).
    let fix_plan_start = text
        .find("---FIX PLAN---")
        .ok_or(ParseError::MissingFixPlan)?;
    let fix_plan_text = &text[fix_plan_start + "---FIX PLAN---".len()..];

    // Split into individual steps on "---" separators.
    let step_blocks: Vec<&str> = fix_plan_text
        .split("\n---\n")
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .collect();

    if step_blocks.is_empty() {
        return Err(ParseError::NoCommandsFound);
    }

    let mut steps = Vec::new();
    for block in &step_blocks {
        if let Some(step) = parse_fix_step(block)? {
            steps.push(step);
        }
    }

    if steps.is_empty() {
        return Err(ParseError::NoCommandsFound);
    }

    Ok(LlmFixPlan {
        narrative: LlmNarrative {
            text: narrative_text,
        },
        steps,
        generated_at: chrono::Utc::now().timestamp(),
    })
}

/// Extract text between two markers.  Returns `None` if the start marker
/// is missing; returns everything after start if end marker is absent.
fn extract_section<'a>(text: &'a str, start_marker: &str, end_marker: &str) -> Option<&'a str> {
    let start_pos = text.find(start_marker)?;
    let after_start = &text[start_pos + start_marker.len()..];
    match after_start.find(end_marker) {
        Some(end_pos) => Some(&after_start[..end_pos]),
        None => Some(after_start),
    }
}

/// Parse a single command block into an `LlmFixStep`.
///
/// Returns `Ok(None)` for empty blocks (no COMMAND line).
fn parse_fix_step(block: &str) -> Result<Option<LlmFixStep>, ParseError> {
    let mut command = String::new();
    let mut purpose = String::new();
    let mut risk_str = String::new();
    let mut rollback = String::new();

    for line in block.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("COMMAND:") {
            command = val.trim().to_string();
        } else if let Some(val) = trimmed.strip_prefix("PURPOSE:") {
            purpose = val.trim().to_string();
        } else if let Some(val) = trimmed.strip_prefix("RISK:") {
            risk_str = val.trim().to_lowercase();
        } else if let Some(val) = trimmed.strip_prefix("ROLLBACK:") {
            rollback = val.trim().to_string();
        }
    }

    if command.is_empty() {
        return Ok(None);
    }

    let risk = match risk_str.as_str() {
        "none" => RiskLevel::None,
        "low" => RiskLevel::Low,
        "medium" => RiskLevel::Medium,
        "high" => RiskLevel::High,
        "" => RiskLevel::Low, // default if missing
        other => return Err(ParseError::InvalidRiskLevel(other.to_string())),
    };

    Ok(Some(LlmFixStep {
        command,
        purpose,
        risk,
        rollback,
        binary_warnings: Vec::new(),
    }))
}

// ── TRD §4.4 ────────────────────────────────────────────────────────────────

/// A step in a troubleshooting plan (read-only check or proposed fix).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub title: String,
    pub command: CommandPreview,
    pub hypothesis_id: Option<String>,
    pub expected_output: Option<String>,
    pub interpretation_guide: Option<String>,
}

/// Source of a troubleshooting plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanSource {
    /// Started from a user question.
    UserQuestion(String),
    /// Started from a stored finding.
    Finding(FindingId),
}

impl std::fmt::Display for PlanSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserQuestion(q) => write!(f, "user question: {q}"),
            Self::Finding(id) => write!(f, "finding: {id}"),
        }
    }
}

/// One hypothesis in the tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hypothesis {
    pub id: String,
    pub hypothesis: String,
    pub evidence_for: Vec<String>,
    pub evidence_against: Vec<String>,
    pub missing_evidence: Vec<String>,
    pub confidence: f64,
    pub domain: MonitorDomain,
}

/// A complete troubleshooting plan. Per TRD §4.4.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TroubleshootingPlan {
    pub id: String,
    pub source: PlanSource,
    pub snapshot_id: SnapshotId,
    pub hypotheses: Vec<Hypothesis>,
    pub read_only_steps: Vec<PlanStep>,
    pub proposed_fix_steps: Vec<PlanStep>,
    pub approval_required: bool,
}

impl TroubleshootingPlan {
    pub fn new(source: PlanSource, snapshot_id: SnapshotId) -> Self {
        Self {
            id: format!("plan-{}", uuid::Uuid::new_v4()),
            source,
            snapshot_id,
            hypotheses: Vec::new(),
            read_only_steps: Vec::new(),
            proposed_fix_steps: Vec::new(),
            approval_required: true,
        }
    }

    /// Render a human-readable plan summary.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Troubleshooting Plan: {}\nSource: {}\n\n",
            self.id, self.source
        ));

        if !self.hypotheses.is_empty() {
            out.push_str("Hypotheses\n----------\n");
            for h in &self.hypotheses {
                out.push_str(&format!(
                    "  [{:.1}%] {} ({:?})\n",
                    h.confidence * 100.0,
                    h.hypothesis,
                    h.domain
                ));
                if !h.evidence_for.is_empty() {
                    out.push_str("    Evidence for:\n");
                    for e in &h.evidence_for {
                        out.push_str(&format!("      + {e}\n"));
                    }
                }
                if !h.evidence_against.is_empty() {
                    out.push_str("    Evidence against:\n");
                    for e in &h.evidence_against {
                        out.push_str(&format!("      - {e}\n"));
                    }
                }
                if !h.missing_evidence.is_empty() {
                    out.push_str("    Missing:\n");
                    for e in &h.missing_evidence {
                        out.push_str(&format!("      ? {e}\n"));
                    }
                }
            }
            out.push('\n');
        }

        if !self.read_only_steps.is_empty() {
            out.push_str("Read-only checks\n----------------\n");
            for s in &self.read_only_steps {
                let cmd = s.command.command_text.as_deref().unwrap_or(&s.command.tool);
                out.push_str(&format!("  $ {cmd}\n"));
                out.push_str(&format!("    {}\n", s.command.expected_effect));
                if let Some(eo) = &s.expected_output {
                    out.push_str(&format!("    Expected output: {eo}\n"));
                }
                if let Some(ig) = &s.interpretation_guide {
                    out.push_str(&format!("    Interpretation: {ig}\n"));
                }
                let r = &s.command.risk;
                let b = &s.command.blast_radius;
                out.push_str(&format!("    Risk: {} | Blast: {b}\n", r.as_str()));
            }
            out.push('\n');
        }

        if !self.proposed_fix_steps.is_empty() {
            out.push_str("Proposed fixes\n--------------\n");
            for s in &self.proposed_fix_steps {
                let cmd = s.command.command_text.as_deref().unwrap_or(&s.command.tool);
                out.push_str(&format!("  $ {cmd}\n"));
                out.push_str(&format!("    Effect: {}\n", s.command.expected_effect));
                let r = &s.command.risk;
                let b = &s.command.blast_radius;
                let rb = &s.command.rollback;
                out.push_str(&format!("    Risk: {} | Blast: {b} | {rb}\n", r.as_str()));
                if let Some(v) = s.command.verification.first() {
                    let v_cmd = v.command_text.as_deref().unwrap_or(&v.tool);
                    out.push_str(&format!("    Verify: $ {v_cmd}\n"));
                }
            }
            if self.approval_required {
                out.push_str("\n  ** Approval required before execution **\n");
            }
        }
        out
    }
}

// ── Troubleshoot planner ────────────────────────────────────────────────────

/// Generate a troubleshooting plan from a user problem using an explicit snapshot.
pub async fn plan_from_problem_with_snapshot(
    problem: &str,
    snapshot: SystemSnapshot,
) -> TroubleshootingPlan {
    let source = PlanSource::UserQuestion(problem.into());
    let sid = snapshot.id.clone();
    let mut plan = TroubleshootingPlan::new(source, sid);
    plan.snapshot_id = snapshot.id.clone();

    let problem_lower = problem.to_lowercase();
    let domains = detect_relevant_domains(&problem_lower, &snapshot);
    for (domain, hypothesis, confidence) in domains {
        let h = build_hypothesis(&snapshot, domain, hypothesis, confidence);
        plan.hypotheses.push(h);
    }

    for h in &plan.hypotheses {
        let steps = build_check_steps(h);
        plan.read_only_steps.extend(steps);
    }

    if has_actionable_evidence(&plan.hypotheses) {
        for h in &plan.hypotheses {
            if h.confidence >= 0.6 {
                let steps = build_fix_steps(h);
                plan.proposed_fix_steps.extend(steps);
            }
        }
    }

    plan
}

/// Generate a troubleshooting plan from a user problem (auto-collects snapshot).
pub async fn plan_from_problem(problem: &str) -> TroubleshootingPlan {
    let snapshot = collect_snapshot(MonitorProfile::Standard).await;
    let sid = snapshot.id.clone();
    let mut plan = TroubleshootingPlan::new(PlanSource::UserQuestion(problem.into()), sid);
    plan.snapshot_id = snapshot.id.clone();

    // Generate hypotheses based on user problem keywords
    let problem_lower = problem.to_lowercase();
    let domains = detect_relevant_domains(&problem_lower, &snapshot);
    for (domain, hypothesis, confidence) in domains {
        let h = build_hypothesis(&snapshot, domain, hypothesis, confidence);
        plan.hypotheses.push(h);
    }

    // Generate read-only check steps from all hypotheses
    for h in &plan.hypotheses {
        let steps = build_check_steps(h);
        plan.read_only_steps.extend(steps);
    }

    // Only propose fix steps if there's evidence backing at least one hypothesis
    if has_actionable_evidence(&plan.hypotheses) {
        for h in &plan.hypotheses {
            if h.confidence >= 0.6 {
                let steps = build_fix_steps(h);
                plan.proposed_fix_steps.extend(steps);
            }
        }
    }

    plan
}

/// Generate a troubleshooting plan from a stored finding.
pub async fn plan_from_finding(finding: &Finding) -> TroubleshootingPlan {
    let source = PlanSource::Finding(finding.id.clone());
    // We need a snapshot for plan construction. Since findings reference snapshot IDs,
    // create a minimal plan that can be populated.
    let mut plan = TroubleshootingPlan::new(source, finding.snapshot_id.clone());
    plan.snapshot_id = finding.snapshot_id.clone();

    // Build hypotheses based on the finding's domain and evidence
    let hyp = Hypothesis {
        id: format!("hyp-{}", uuid::Uuid::new_v4()),
        hypothesis: finding.title.clone(),
        evidence_for: finding
            .evidence
            .iter()
            .map(|e| format!("{}: {} ({})", e.source, e.value, e.note))
            .collect(),
        evidence_against: Vec::new(),
        missing_evidence: finding.missing_data.clone(),
        confidence: match finding.confidence {
            crate::findings::Confidence::High => 0.8,
            crate::findings::Confidence::Medium => 0.6,
            crate::findings::Confidence::Low => 0.3,
        },
        domain: finding.category,
    };
    plan.hypotheses.push(hyp);

    // Generate check steps from the finding
    for check in &finding.read_only_checks {
        let step = PlanStep {
            title: format!("Suggested check: {check}"),
            command: CommandPreview::new(
                "shell",
                check,
                "Inspection command suggested by the finding",
            ),
            hypothesis_id: Some(plan.hypotheses[0].id.clone()),
            expected_output: None,
            interpretation_guide: None,
        };
        plan.read_only_steps.push(step);
    }

    plan
}

/// Render a human explanation of a single finding.
pub fn explain_finding(finding: &Finding) -> String {
    let mut out = String::new();
    out.push_str(&format!("Finding: {}\n", finding.title));
    out.push_str(&format!("  Severity:     {:?}\n", finding.severity));
    out.push_str(&format!("  Confidence:   {:?}\n", finding.confidence));
    out.push_str(&format!("  Resource:     {}\n", finding.affected_resource));
    out.push_str(&format!("  Impact:       {}\n", finding.impact));
    if !finding.evidence.is_empty() {
        out.push_str("  Evidence:\n");
        for e in &finding.evidence {
            out.push_str(&format!("    {}: {} ({})\n", e.source, e.value, e.note));
        }
    }
    if !finding.assumptions.is_empty() {
        out.push_str(&format!(
            "  Assumptions: {}\n",
            finding.assumptions.join(", ")
        ));
    }
    if !finding.missing_data.is_empty() {
        out.push_str(&format!(
            "  Missing data: {}\n",
            finding.missing_data.join(", ")
        ));
    }
    if !finding.read_only_checks.is_empty() {
        out.push_str("  Suggested checks:\n");
        for c in &finding.read_only_checks {
            out.push_str(&format!("    {} {c}\n", "$"));
        }
    }
    out
}

// ── Private helpers ─────────────────────────────────────────────────────────

fn detect_relevant_domains(
    problem: &str,
    _snapshot: &SystemSnapshot,
) -> Vec<(MonitorDomain, String, f64)> {
    let mut domains: Vec<(MonitorDomain, String, f64)> = Vec::new();

    if problem.contains("disk") || problem.contains("space") || problem.contains("storage") {
        domains.push((MonitorDomain::Disks, "Filesystem may be full".into(), 0.7));
    }
    if problem.contains("slow") || problem.contains("load") || problem.contains("cpu") {
        domains.push((MonitorDomain::Load, "System may be overloaded".into(), 0.6));
    }
    if problem.contains("memory") || problem.contains("oom") || problem.contains("swap") {
        domains.push((MonitorDomain::Load, "Memory may be exhausted".into(), 0.7));
    }
    if problem.contains("service") || problem.contains("fail") || problem.contains("crash") {
        domains.push((
            MonitorDomain::Services,
            "Service may be failing".into(),
            0.8,
        ));
    }
    if problem.contains("container") || problem.contains("docker") || problem.contains("pod") {
        domains.push((
            MonitorDomain::Containers,
            "Container may be unhealthy".into(),
            0.7,
        ));
    }
    if problem.contains("port") || problem.contains("connect") || problem.contains("network") {
        domains.push((MonitorDomain::Ports, "Port or network issue".into(), 0.6));
    }
    if problem.contains("log") || problem.contains("error") {
        domains.push((
            MonitorDomain::Logs,
            "Log errors may indicate root cause".into(),
            0.7,
        ));
    }
    if problem.contains("backup") || problem.contains("restore") {
        domains.push((
            MonitorDomain::Backups,
            "Backup or restore issue".into(),
            0.8,
        ));
    }

    domains
}

fn build_hypothesis(
    snapshot: &SystemSnapshot,
    domain: MonitorDomain,
    hypothesis: String,
    confidence: f64,
) -> Hypothesis {
    let mut evidence_for = Vec::new();
    let mut evidence_against = Vec::new();
    let mut missing_evidence = Vec::new();

    match domain {
        MonitorDomain::Disks => {
            for fs in &snapshot.domains.disks.filesystems {
                let pct = if fs.total_bytes > 0 {
                    (fs.used_bytes as f64 / fs.total_bytes as f64) * 100.0
                } else {
                    0.0
                };
                if pct >= 90.0 {
                    evidence_for.push(format!(
                        "{} is {:.0}% full ({}/{})",
                        fs.mount_point,
                        pct,
                        human_bytes(fs.used_bytes),
                        human_bytes(fs.total_bytes)
                    ));
                }
            }
            if evidence_for.is_empty() {
                evidence_against.push("No filesystem is critically full".into());
            }
            missing_evidence.push("Deleted-open file check needed".into());
        }
        MonitorDomain::Load => {
            let cores = snapshot.domains.load.cpu_logical_count.max(1) as f64;
            if snapshot.domains.load.load_average.fifteen > cores * 2.0 {
                evidence_for.push(format!(
                    "15m load avg ({:.2}) exceeds 2x CPU count ({})",
                    snapshot.domains.load.load_average.fifteen, cores
                ));
            } else {
                evidence_against.push("Load average is within normal range".into());
            }
            if snapshot.domains.load.memory.total > 0 {
                let mem_pct = (snapshot.domains.load.memory.used as f64
                    / snapshot.domains.load.memory.total as f64)
                    * 100.0;
                if mem_pct >= 85.0 {
                    evidence_for.push(format!(
                        "Memory usage is {:.0}% ({}/{})",
                        mem_pct,
                        human_bytes(snapshot.domains.load.memory.used),
                        human_bytes(snapshot.domains.load.memory.total)
                    ));
                }
            }
        }
        MonitorDomain::Services => {
            for u in &snapshot.domains.services.failed_units {
                evidence_for.push(format!("{}: {}", u.name, u.description));
            }
            if snapshot.domains.services.failed_units.is_empty() {
                evidence_against.push("No failed systemd units".into());
            }
        }
        MonitorDomain::Containers => {
            for c in &snapshot.domains.containers.containers {
                if c.status.contains("Exited") || c.status.contains("unhealthy") {
                    evidence_for.push(format!("{} is {}", c.name, c.status));
                }
            }
            if evidence_for.is_empty() {
                evidence_against.push("All containers appear healthy".into());
            }
        }
        MonitorDomain::Ports => {
            for l in &snapshot.domains.ports.listeners {
                if l.local_address != "127.0.0.1" && l.local_address != "::1" {
                    evidence_for.push(format!(
                        "Port {} is exposed on {}",
                        l.local_port, l.local_address
                    ));
                }
            }
        }
        MonitorDomain::Logs => {
            if snapshot.domains.logs.journal_errors_last_hour > 0 {
                evidence_for.push(format!(
                    "{} journal errors in the last hour",
                    snapshot.domains.logs.journal_errors_last_hour
                ));
            }
        }
        MonitorDomain::Backups => {
            if snapshot.domains.backups.tools_detected.is_empty() {
                evidence_for.push("No backup tools detected".into());
            }
        }
        _ => {}
    }

    let has_evidence_for = !evidence_for.is_empty();
    let has_evidence_against = !evidence_against.is_empty();
    let adj_confidence = if !has_evidence_for && !has_evidence_against {
        confidence * 0.5
    } else if !has_evidence_for {
        confidence * 0.3
    } else {
        confidence.min(1.0)
    };

    Hypothesis {
        id: format!("hyp-{}", uuid::Uuid::new_v4()),
        hypothesis,
        evidence_for,
        evidence_against,
        missing_evidence,
        confidence: adj_confidence,
        domain,
    }
}

fn build_check_steps(hypothesis: &Hypothesis) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    match hypothesis.domain {
        MonitorDomain::Disks => {
            steps.push(PlanStep {
                title: "Check disk usage by directory".into(),
                command: CommandPreview::new(
                    "shell",
                    "du -sh /* 2>/dev/null | sort -rh | head -10",
                    "Identify largest directories consuming disk space",
                ),
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: Some("Sorted list of directory sizes (e.g., '4.5G /var/log')".into()),
                interpretation_guide: Some("The top entry is the largest space consumer. Cross-check with df to account for all usage.".into()),
            });
            steps.push(PlanStep {
                title: "Check for deleted-open files".into(),
                command: CommandPreview::new(
                    "shell",
                    "lsof +L1 2>/dev/null | head -20",
                    "Find files deleted but still held open by processes",
                ),
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: Some("Lines with '(deleted)' in the filename column".into()),
                interpretation_guide: Some("Files appearing here occupy space not shown by du. Restart the owning process to release space.".into()),
            });
        }
        MonitorDomain::Load => {
            steps.push(PlanStep {
                title: "Top CPU processes".into(),
                command: CommandPreview::new(
                    "shell",
                    "ps aux --sort=-%cpu | head -15",
                    "Identify highest CPU-consuming processes",
                ),
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: Some("Table: PID, %CPU, %MEM, COMMAND sorted by CPU descending".into()),
                interpretation_guide: Some("Processes above 50% CPU may be problematic. A single process near 100% indicates it is CPU-bound.".into()),
            });
        }
        MonitorDomain::Services => {
            steps.push(PlanStep {
                title: "Service status and recent logs".into(),
                command: CommandPreview::new(
                    "shell",
                    "systemctl list-units --failed --no-pager",
                    "List all failed systemd units with status details",
                ),
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: Some("Table with columns: UNIT, LOAD, ACTIVE, SUB, DESCRIPTION".into()),
                interpretation_guide: Some("Any unit with sub=failed needs investigation. Run 'journalctl -u <unit>' for the failure reason.".into()),
            });
        }
        MonitorDomain::Logs => {
            steps.push(PlanStep {
                title: "Recent error logs".into(),
                command: CommandPreview::new(
                    "shell",
                    "journalctl -p err -n 30 --no-pager",
                    "Show the 30 most recent error-level journal entries",
                ),
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: Some("Timestamped error messages (priority 'err' or higher)".into()),
                interpretation_guide: Some("Recurring identical errors indicate an active fault. OOM killer entries signal memory exhaustion. Service crash patterns help identify root cause.".into()),
            });
        }
        _ => {}
    }
    steps
}

fn build_fix_steps(hypothesis: &Hypothesis) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    match hypothesis.domain {
        MonitorDomain::Disks => {
            steps.push(PlanStep {
                title: "Clean package cache".into(),
                command: CommandPreview {
                    tool: "shell".into(),
                    input: serde_json::json!({"command": "apt-get clean"}),
                    command_text: Some("apt-get clean".into()),
                    expected_effect: "Free disk space by removing cached package files".into(),
                    risk: RiskLevel::Low,
                    blast_radius: BlastRadius::Directory("/var/cache/apt".into()),
                    rollback: RollbackStatus::NotNeeded,
                    verification: vec![CommandPreview::new(
                        "shell",
                        "df -h /",
                        "Verify freed space",
                    )],
                },
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: None,
                interpretation_guide: None,
            });
            steps.push(PlanStep {
                title: "Remove old journal logs".into(),
                command: CommandPreview {
                    tool: "shell".into(),
                    input: serde_json::json!({"command": "journalctl --vacuum-time=7d"}),
                    command_text: Some("journalctl --vacuum-time=7d".into()),
                    expected_effect: "Remove journal entries older than 7 days to free space"
                        .into(),
                    risk: RiskLevel::Low,
                    blast_radius: BlastRadius::Directory("/var/log/journal".into()),
                    rollback: RollbackStatus::Available(
                        "No rollback — logs are permanently removed".into(),
                    ),
                    verification: vec![CommandPreview::new(
                        "shell",
                        "df -h /",
                        "Verify freed space",
                    )],
                },
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: None,
                interpretation_guide: None,
            });
        }
        MonitorDomain::Load => {
            steps.push(PlanStep {
                title: "Restart high-memory service".into(),
                command: CommandPreview {
                    tool: "shell".into(),
                    input: serde_json::json!({"command": "systemctl restart <service>"}),
                    command_text: Some("systemctl restart <service>".into()),
                    expected_effect: "Restart the identified problematic service".into(),
                    risk: RiskLevel::Medium,
                    blast_radius: BlastRadius::Service("<service>".into()),
                    rollback: RollbackStatus::Available(
                        "systemctl restart <service> — same command restarts again".into(),
                    ),
                    verification: vec![CommandPreview::new(
                        "shell",
                        "systemctl status <service>",
                        "Verify service is running",
                    )],
                },
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: None,
                interpretation_guide: None,
            });
        }
        MonitorDomain::Services => {
            steps.push(PlanStep {
                title: "Restart failed service".into(),
                command: CommandPreview {
                    tool: "shell".into(),
                    input: serde_json::json!({"command": "systemctl restart nginx"}),
                    command_text: Some("systemctl restart nginx".into()),
                    expected_effect: "Attempt to restart the failed service".into(),
                    risk: RiskLevel::Medium,
                    blast_radius: BlastRadius::Service("nginx".into()),
                    rollback: RollbackStatus::Available(
                        "systemctl restart nginx — same command restarts again".into(),
                    ),
                    verification: vec![CommandPreview::new(
                        "shell",
                        "systemctl status nginx",
                        "Verify service is running",
                    )],
                },
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: None,
                interpretation_guide: None,
            });
        }
        _ => {}
    }
    steps
}

fn has_actionable_evidence(hypotheses: &[Hypothesis]) -> bool {
    hypotheses
        .iter()
        .any(|h| h.confidence >= 0.3 && !h.evidence_for.is_empty())
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}G", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}M", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{HostIdentity, MonitorProfile, SnapshotDomains};

    #[test]
    fn plan_from_problem_generates_hypotheses() {
        let plan = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(plan_from_problem("disk is full"));
        assert!(!plan.hypotheses.is_empty(), "should generate hypotheses");
        assert!(
            plan.hypotheses
                .iter()
                .any(|h| h.domain == MonitorDomain::Disks),
            "disk problem should produce disk hypothesis"
        );
    }

    #[test]
    fn plan_with_empty_snapshot_has_no_evidence() {
        let snapshot = SystemSnapshot {
            id: "test".into(),
            host: HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: MonitorProfile::Standard,
            domains: SnapshotDomains::default(),
            collector_errors: vec![],
            redaction_version: "0.1.0".into(),
        };
        let mut plan = TroubleshootingPlan::new(
            PlanSource::UserQuestion("disk is full".into()),
            snapshot.id.clone(),
        );
        let hypothesis = build_hypothesis(
            &snapshot,
            MonitorDomain::Disks,
            "Filesystem may be full".into(),
            0.7,
        );
        plan.hypotheses.push(hypothesis);
        assert!(
            !has_actionable_evidence(&plan.hypotheses),
            "empty snapshot should not trigger actionable evidence"
        );
        assert!(
            plan.proposed_fix_steps.is_empty(),
            "no evidence should mean no fix steps"
        );
    }

    #[test]
    fn command_validator_passes_safe_commands() {
        // Safe inspection commands should all pass validation.
        let safe = ["df -h", "systemctl status nginx", "free -h"];
        for cmd in safe {
            assert!(
                CommandValidator::validate_command(cmd).is_ok(),
                "safe command should pass: '{cmd}'"
            );
        }
    }

    #[test]
    fn command_validator_rejects_dangerous_patterns() {
        let dangerous = [
            ("rm -rf / --no-preserve-root", "rm -rf /"),
            ("sudo mkfs.ext4 /dev/sda1", "mkfs"),
            ("dd if=/dev/zero of=/dev/sda", "dd"),
            ("chmod 777 /", "chmod 777"),
            (":(){ :|:& };:", "fork bomb"),
            ("somecmd > /dev/sda", "redirect to raw device"),
        ];
        for (cmd, expected_keyword) in dangerous {
            let result = CommandValidator::validate_command(cmd);
            assert!(
                result.is_err(),
                "dangerous command should be rejected: '{cmd}'"
            );
            let errs = result.unwrap_err();
            assert!(
                !errs.is_empty(),
                "should have at least one error for '{cmd}'"
            );
            let joined = errs.join(" ");
            assert!(
                joined
                    .to_lowercase()
                    .contains(&expected_keyword.to_lowercase()),
                "error for '{cmd}' should mention '{expected_keyword}', got: {joined}"
            );
        }
    }

    #[test]
    fn command_validator_path_check_finds_binaries() {
        for bin in ["ls", "df", "cat"] {
            let found = CommandValidator::check_binary_in_path(bin);
            assert!(
                found.is_some(),
                "standard binary should be found in PATH: '{bin}'"
            );
        }
    }

    #[test]
    fn command_validator_path_check_warns_missing() {
        let fake = "nonexistent_binary_xyzzy_42";
        let found = CommandValidator::check_binary_in_path(fake);
        assert!(found.is_none(), "fake binary should not be found in PATH");
    }

    #[test]
    fn check_all_binaries_flags_missing_in_multi_command() {
        let cmd = "ls -la && nonexistent_binary_xyzzy_42 --flag";
        let warnings = CommandValidator::check_all_binaries(cmd);
        assert_eq!(warnings.len(), 1, "should have exactly 1 warning");
        assert_eq!(warnings[0].binary, "nonexistent_binary_xyzzy_42");
        assert!(warnings[0].warning.contains("not found in PATH"));
    }

    #[test]
    fn check_all_binaries_skips_shell_builtins() {
        let cmd = "cd /tmp && echo hello && export FOO=bar";
        let warnings = CommandValidator::check_all_binaries(cmd);
        assert!(
            warnings.is_empty(),
            "shell builtins should not generate warnings, got: {warnings:?}"
        );
    }

    // ── T02: LLM prompt + parser tests ─────────────────────────────────

    #[test]
    fn prompt_includes_finding_fields() {
        let finding = FindingSummaryFields {
            title: "Disk usage critical on /dev/sda1".into(),
            severity: "Critical".into(),
            affected_resource: "/dev/sda1".into(),
            evidence_summaries: vec![
                "/ is 95% full (45G/50G)".into(),
                "journald consuming 12G in /var/log/journal".into(),
            ],
            detector_id: "disk_usage".into(),
            impact: "System may become unresponsive if disk fills completely".into(),
        };

        let prompt = build_narrative_prompt(&finding);

        // Each field must appear in the prompt text.
        assert!(
            prompt.contains("Disk usage critical on /dev/sda1"),
            "prompt should contain title"
        );
        assert!(
            prompt.contains("Critical"),
            "prompt should contain severity"
        );
        assert!(
            prompt.contains("/dev/sda1"),
            "prompt should contain resource"
        );
        assert!(
            prompt.contains("disk_usage"),
            "prompt should contain detector_id"
        );
        assert!(
            prompt.contains("95% full"),
            "prompt should contain evidence"
        );
        assert!(
            prompt.contains("unresponsive"),
            "prompt should contain impact"
        );
        // Fork identity: must reference Jatin-Mali/helm, not upstream.
        assert!(
            prompt.contains("Jatin-Mali/helm"),
            "prompt should reference fork identity"
        );
        // Should include format markers.
        assert!(
            prompt.contains("---NARRATIVE---"),
            "prompt should instruct about NARRATIVE marker"
        );
        assert!(
            prompt.contains("---FIX PLAN---"),
            "prompt should instruct about FIX PLAN marker"
        );
    }

    #[test]
    fn parser_extracts_narrative_and_commands() {
        let response = "\
---NARRATIVE---
The disk filled up because journald logs were not being rotated properly.
The log retention policy was too aggressive, keeping 90 days of logs.
This consumed 12 GB in /var/log/journal, leaving only 5% free space.
---FIX PLAN---
COMMAND: journalctl --vacuum-time=7d
PURPOSE: Remove journal entries older than 7 days to free space
RISK: low
ROLLBACK: none
---
COMMAND: sed -i 's/MaxRetentionSec=90day/MaxRetentionSec=7day/' /etc/systemd/journald.conf
PURPOSE: Reduce log retention from 90 days to 7 days to prevent recurrence
RISK: medium
ROLLBACK: sed -i 's/MaxRetentionSec=7day/MaxRetentionSec=90day/' /etc/systemd/journald.conf
---
";

        let plan = parse_llm_response(response).expect("should parse valid response");

        assert!(
            plan.narrative.text.contains("journald logs"),
            "narrative should mention journald"
        );
        assert_eq!(plan.steps.len(), 2, "should have 2 fix steps");

        let step0 = &plan.steps[0];
        assert_eq!(step0.command, "journalctl --vacuum-time=7d");
        assert_eq!(step0.risk, RiskLevel::Low);
        assert_eq!(step0.rollback, "none");

        let step1 = &plan.steps[1];
        assert_eq!(
            step1.command,
            "sed -i 's/MaxRetentionSec=90day/MaxRetentionSec=7day/' /etc/systemd/journald.conf"
        );
        assert_eq!(step1.risk, RiskLevel::Medium);
        assert!(
            step1.rollback.contains("MaxRetentionSec=7day"),
            "rollback should reverse the sed"
        );

        assert!(plan.generated_at > 0, "generated_at should be set");
    }

    #[test]
    fn parser_rejects_missing_narrative() {
        let response = "\
COMMAND: df -h
PURPOSE: Check disk space
RISK: none
ROLLBACK: none
";
        let result = parse_llm_response(response);
        assert!(result.is_err(), "should error on missing narrative");
        match result.unwrap_err() {
            ParseError::MissingNarrative => {} // expected
            other => panic!("expected MissingNarrative, got {other:?}"),
        }
    }

    #[test]
    fn parser_rejects_malformed_risk() {
        let response = "\
---NARRATIVE---
Something is broken.
---FIX PLAN---
COMMAND: restart-foo
PURPOSE: Restart the foo service
RISK: apocalyptic
ROLLBACK: none
---
";
        let result = parse_llm_response(response);
        assert!(result.is_err(), "should error on malformed risk");
        match result.unwrap_err() {
            ParseError::InvalidRiskLevel(v) => {
                assert_eq!(v, "apocalyptic");
            }
            other => panic!("expected InvalidRiskLevel, got {other:?}"),
        }
    }

    #[test]
    fn parser_handles_empty_fix_plan() {
        let response = "\
---NARRATIVE---
Nothing is wrong with this system. All checks are passing.
---FIX PLAN---
";
        let result = parse_llm_response(response);
        assert!(result.is_err(), "empty fix plan should error");
        match result.unwrap_err() {
            ParseError::NoCommandsFound => {} // expected
            other => panic!("expected NoCommandsFound, got {other:?}"),
        }
    }

    #[test]
    fn parser_defaults_missing_risk_to_low() {
        let response = "\
---NARRATIVE---
Disk is getting full due to accumulated package caches.
---FIX PLAN---
COMMAND: apt-get clean
PURPOSE: Clear package cache to free space
ROLLBACK: none
---
";
        let plan = parse_llm_response(response).expect("should parse even without RISK");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(
            plan.steps[0].risk,
            RiskLevel::Low,
            "missing RISK should default to Low"
        );
    }

    #[test]
    fn finding_summary_from_finding() {
        use crate::findings::{Confidence, EvidenceRef, Finding, Severity};
        let mut f = Finding::new(
            "snap-1",
            "disk_usage",
            "/dev/sda1",
            "Disk near capacity",
            Severity::Critical,
            Confidence::High,
            MonitorDomain::Disks,
        );
        f.impact = "System may become unresponsive".into();
        f.evidence = vec![EvidenceRef {
            source: "disks.filesystems[0]".into(),
            value: "95%".into(),
            note: "Root filesystem is critically full".into(),
        }];

        let summary = FindingSummaryFields::from(&f);
        assert_eq!(summary.title, "Disk near capacity");
        assert!(summary.severity.contains("Critical"));
        assert_eq!(summary.affected_resource, "/dev/sda1");
        assert_eq!(summary.detector_id, "disk_usage");
        assert_eq!(summary.impact, "System may become unresponsive");
        assert_eq!(summary.evidence_summaries.len(), 1);
        assert!(
            summary.evidence_summaries[0].contains("95%"),
            "evidence summary should contain the observed value"
        );
    }
}
