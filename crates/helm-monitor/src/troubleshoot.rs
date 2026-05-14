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
                expected_output: None,
                interpretation_guide: None,
            });
            steps.push(PlanStep {
                title: "Check for deleted-open files".into(),
                command: CommandPreview::new(
                    "shell",
                    "lsof +L1 2>/dev/null | head -20",
                    "Find files deleted but still held open by processes",
                ),
                hypothesis_id: Some(hypothesis.id.clone()),
                expected_output: None,
                interpretation_guide: None,
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
                expected_output: None,
                interpretation_guide: None,
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
                expected_output: None,
                interpretation_guide: None,
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
                expected_output: None,
                interpretation_guide: None,
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
    fn command_preview_contains_required_fields() {
        let preview = CommandPreview::new("shell", "df -h /", "Check root filesystem usage")
            .with_risk(RiskLevel::None)
            .with_blast(BlastRadius::File("/".into()))
            .with_verification(CommandPreview::new(
                "shell",
                "echo ok",
                "verify nothing changed",
            ));

        assert_eq!(preview.tool, "shell");
        assert_eq!(preview.command_text.as_deref(), Some("df -h /"));
        assert_eq!(preview.risk, RiskLevel::None);
    }
}
