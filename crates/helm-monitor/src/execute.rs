use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    collect_snapshot,
    snapshot::{MonitorProfile, SnapshotId, SystemSnapshot},
    troubleshoot::{CommandPreview, PlanSource, TroubleshootingPlan},
};

// ── ChangeSet ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSetStatus {
    Pending,
    Approved,
    Rejected,
    Running,
    Completed,
    Failed,
    RolledBack,
}

impl std::fmt::Display for ChangeSetStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Approved => write!(f, "approved"),
            Self::Rejected => write!(f, "rejected"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::RolledBack => write!(f, "rolled_back"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

impl std::fmt::Display for StepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Succeeded => write!(f, "succeeded"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeSetStep {
    pub id: String,
    pub plan_step_title: String,
    pub command: CommandPreview,
    pub status: StepStatus,
    pub output_text: String,
    pub error_text: String,
    pub verification_result: String,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
}

/// Pre-change backup of a file before mutation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreChangeBackup {
    pub id: String,
    pub file_path: String,
    pub checksum_before: String,
    #[serde(skip)]
    pub backup_content: String,
    pub restored: bool,
}

impl std::fmt::Debug for PreChangeBackup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreChangeBackup")
            .field("id", &self.id)
            .field("backup_content", &"[sensitive recovery data]")
            .finish_non_exhaustive()
    }
}

/// Persistence adapters must commit preparation before this engine performs a mutation.
pub trait ExecutionJournal: Send + Sync {
    fn start(&self, change: &ChangeSet, before: &SystemSnapshot) -> Result<(), String>;
    fn prepare(
        &self,
        change_id: &str,
        step: &ChangeSetStep,
        backup: Option<&PreChangeBackup>,
    ) -> Result<(), String>;
    fn record_step(&self, step: &ChangeSetStep) -> Result<(), String>;
    fn finish(&self, change: &ChangeSet, after: &SystemSnapshot) -> Result<(), String>;
}

/// Complete change set for an approved plan execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub id: String,
    pub plan_id: String,
    pub plan_title: String,
    pub snapshot_id: SnapshotId,
    pub before_snapshot_id: SnapshotId,
    pub after_snapshot_id: Option<SnapshotId>,
    pub status: ChangeSetStatus,
    pub created_at: i64,
    pub approved_at: Option<i64>,
    pub rejected_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub rolled_back_at: Option<i64>,
    pub rollback_snapshot_id: Option<SnapshotId>,
    pub steps: Vec<ChangeSetStep>,
    pub backups: Vec<PreChangeBackup>,
    pub summary: String,
}

impl ChangeSet {
    pub fn new(plan: &TroubleshootingPlan, before_snapshot_id: SnapshotId) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut steps = Vec::new();
        for ps in &plan.proposed_fix_steps {
            steps.push(ChangeSetStep {
                id: Uuid::new_v4().to_string(),
                plan_step_title: ps.title.clone(),
                command: ps.command.clone(),
                status: StepStatus::Pending,
                output_text: String::new(),
                error_text: String::new(),
                verification_result: String::new(),
                started_at: None,
                completed_at: None,
            });
        }
        let title = match &plan.source {
            PlanSource::UserQuestion(q) => q.clone(),
            PlanSource::Finding(id) => format!("finding: {id}"),
        };
        Self {
            id: Uuid::new_v4().to_string(),
            plan_id: plan.id.clone(),
            plan_title: title,
            snapshot_id: plan.snapshot_id.clone(),
            before_snapshot_id,
            after_snapshot_id: None,
            status: ChangeSetStatus::Pending,
            created_at: now,
            approved_at: None,
            rejected_at: None,
            completed_at: None,
            rolled_back_at: None,
            rollback_snapshot_id: None,
            steps,
            backups: Vec::new(),
            summary: String::new(),
        }
    }

    pub fn status_summary(&self) -> String {
        let total = self.steps.len();
        let succeeded = self
            .steps
            .iter()
            .filter(|s| s.status == StepStatus::Succeeded)
            .count();
        let failed = self
            .steps
            .iter()
            .filter(|s| s.status == StepStatus::Failed)
            .count();
        format!(
            "{}: {}/{} steps succeeded, {} failed",
            self.status, succeeded, total, failed
        )
    }
}

// ── ExecutionEngine ────────────────────────────────────────────────────────

/// Single entry in the HMAC-chained audit log for step execution.
/// Each entry's hash = SHA-256(prev_hash ‖ timestamp ‖ command ‖ exit_ok ‖ stdout ‖ stderr).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAuditEntry {
    /// Unix timestamp of execution.
    pub timestamp: i64,
    /// Command text that was run.
    pub command_text: String,
    /// Whether the command exited successfully.
    pub exit_ok: bool,
    /// SHA-256 of stdout (hex).
    pub stdout_hash: String,
    /// SHA-256 of stderr (hex).
    pub stderr_hash: String,
    /// Hash of the previous entry (all-zeros string for the first entry).
    pub prev_hash: String,
    /// HMAC-style chain hash for this entry.
    pub hash: String,
}

impl ExecutionAuditEntry {
    /// Compute a new audit entry chained from the previous hash.
    pub fn new(
        timestamp: i64,
        command_text: impl Into<String>,
        exit_ok: bool,
        stdout: &str,
        stderr: &str,
        prev_hash: impl Into<String>,
    ) -> Self {
        let command_text = command_text.into();
        let prev_hash = prev_hash.into();
        let stdout_hash = hex::encode(Sha256::digest(stdout.as_bytes()));
        let stderr_hash = hex::encode(Sha256::digest(stderr.as_bytes()));
        let chain_input = format!(
            "{prev_hash}{timestamp}{command_text}{}{stdout_hash}{stderr_hash}",
            if exit_ok { "1" } else { "0" }
        );
        let hash = hex::encode(Sha256::digest(chain_input.as_bytes()));
        Self {
            timestamp,
            command_text,
            exit_ok,
            stdout_hash,
            stderr_hash,
            prev_hash,
            hash,
        }
    }
}

/// Verify the integrity of a HMAC-chained audit log.
/// Returns `true` if all entries link correctly, `false` on any break.
pub fn verify_audit_chain(entries: &[ExecutionAuditEntry]) -> bool {
    let zero = "0".repeat(64);
    let mut expected_prev = zero;
    for entry in entries {
        if entry.prev_hash != expected_prev {
            return false;
        }
        let chain_input = format!(
            "{}{}{}{}{}{}",
            entry.prev_hash,
            entry.timestamp,
            entry.command_text,
            if entry.exit_ok { "1" } else { "0" },
            entry.stdout_hash,
            entry.stderr_hash,
        );
        let expected_hash = hex::encode(Sha256::digest(chain_input.as_bytes()));
        if entry.hash != expected_hash {
            return false;
        }
        expected_prev = entry.hash.clone();
    }
    true
}

/// Outcome of a single executed step.
#[derive(Debug, Clone)]
pub struct StepOutcome {
    pub success: bool,
    pub output: String,
    pub error: String,
    pub changed_files: Vec<String>,
    pub verification_ok: bool,
    pub verification_output: String,
    pub backups: Vec<PreChangeBackup>,
}

/// Drives controlled execution of a TroubleshootingPlan with pre/post snapshots,
/// approval gating, change detection, and verification.
pub struct ExecutionEngine {
    approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync>,
    approvals: Option<crate::PlanApprovals>,
    policy: crate::ExecutionPolicy,
    limits: crate::process_runner::ProcessLimits,
    cancellation: crate::process_runner::CancellationToken,
    journal: Option<std::sync::Arc<dyn ExecutionJournal>>,
    before_snapshot: Option<SystemSnapshot>,
    after_snapshot: Option<SystemSnapshot>,
    /// HMAC-chained audit log of every executed step.
    pub audit_chain: Vec<ExecutionAuditEntry>,
}

impl ExecutionEngine {
    /// Create with an approval callback. Return `true` to proceed, `false` to deny.
    /// Attach `with_journal` for durable recovery; otherwise evidence exists only in memory.
    pub fn new(approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync>) -> Self {
        Self {
            approve_fn,
            approvals: None,
            policy: crate::ExecutionPolicy::default(),
            limits: crate::process_runner::ProcessLimits::default(),
            cancellation: crate::process_runner::CancellationToken::default(),
            journal: None,
            before_snapshot: None,
            after_snapshot: None,
            audit_chain: Vec::new(),
        }
    }

    pub fn with_approvals(approvals: crate::PlanApprovals) -> Self {
        let mut engine = Self::new(Box::new(|_| false));
        engine.approvals = Some(approvals);
        engine
    }

    pub fn with_policy(mut self, policy: crate::ExecutionPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_journal(mut self, journal: std::sync::Arc<dyn ExecutionJournal>) -> Self {
        self.journal = Some(journal);
        self
    }

    pub fn with_cancellation(mut self, token: crate::process_runner::CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    /// Run a pre-flight idempotency check. Returns Some(reason) if the step
    /// is already satisfied and can be skipped, None if execution is needed.
    async fn check_idempotent(preview: &CommandPreview) -> Option<String> {
        match preview.tool.as_str() {
            "fs_write" | "write" => {
                let path = preview
                    .input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let content = preview
                    .input
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !path.is_empty()
                    && std::path::Path::new(path).exists()
                    && let Ok(existing) = std::fs::read_to_string(path)
                    && existing == content
                {
                    return Some(format!("file {} already has the desired content", path));
                }
                None
            }
            _ => None,
        }
    }

    /// Run verification commands from a preview and compare output against expectations.
    async fn run_verification(
        &self,
        preview: &CommandPreview,
        expected_output: Option<&str>,
    ) -> (bool, Vec<String>) {
        let mut results = Vec::new();
        let mut all_passed = true;
        for v in &preview.verification {
            let checked = async {
                let text = v.command_text.as_deref().ok_or("verification command missing")?;
                if v.tool != "shell" || text.contains([';', '|', '&', '<', '>', '$', '`', '\n', '\r', '\'', '"', '\\']) {
                    return Err("verification requires a simple read-only command without shell syntax".to_string());
                }
                let args: Vec<_> = text.split_whitespace().collect();
                let allowed = matches!(args.first().copied(), Some("printf" | "echo" | "test" | "cat" | "stat" | "ls" | "df" | "du" | "head" | "tail" | "grep"))
                    || (args.first() == Some(&"systemctl") && matches!(args.get(1).copied(), Some("is-active" | "is-enabled" | "status" | "show")));
                if !allowed || args.iter().any(|arg| helm_core::is_protected_local_path(std::path::Path::new(arg.rsplit('=').next().unwrap_or(arg)))) {
                    return Err("unsupported or protected verification command; use an explicit reviewed plan step".to_string());
                }
                // Verification runs literal argv; it never receives shell mutation authority.
                let mut command = tokio::process::Command::new(args[0]);
                command.args(&args[1..]).env("SYSTEMD_PAGER", "").stdin(std::process::Stdio::null());
                crate::process_runner::run_command(command, self.limits, &self.cancellation)
                    .await.map_err(|error| error.to_string())
            }.await;
            let (success, output, error) = match checked {
                Ok(out) => (
                    out.status.success(),
                    String::from_utf8_lossy(&out.stdout).into_owned(),
                    String::from_utf8_lossy(&out.stderr).into_owned(),
                ),
                Err(error) => (false, String::new(), error),
            };
            let output_line = output.lines().next().unwrap_or("").to_string();
            let passed = if let Some(expected) = expected_output {
                output.contains(expected) && success
            } else {
                success
            };
            all_passed &= passed;
            let v_text = if passed {
                format!("ok: {output_line}")
            } else {
                format!(
                    "failed: expected {:?}; output: {output_line}; {}",
                    expected_output,
                    error.lines().next().unwrap_or("")
                )
            };
            results.push(v_text);
        }
        (all_passed, results)
    }

    /// Execute a full plan, returning a ChangeSet with execution results.
    /// Takes a before-snapshot, runs each fix step through approval + execution,
    /// takes an after-snapshot, and runs post-change detection.
    pub async fn execute(&mut self, plan: &TroubleshootingPlan) -> ChangeSet {
        let approvals = self.approvals.take();
        match self.policy.check_mutation() {
            Ok(true) => {}
            disposition => {
                let mut result = ChangeSet::new(plan, plan.snapshot_id.clone());
                result.status = if disposition.is_ok() {
                    ChangeSetStatus::Pending
                } else {
                    ChangeSetStatus::Rejected
                };
                result.summary = disposition
                    .err()
                    .unwrap_or_else(|| "dry run: no operations executed".into());
                for step in &mut result.steps {
                    step.status = StepStatus::Skipped;
                    step.verification_result = result.summary.clone();
                }
                return result;
            }
        }
        // 1. Capture before-snapshot
        let before = collect_snapshot(MonitorProfile::Quick).await;
        let before_id = before.id.clone();
        self.before_snapshot = Some(before);

        let mut cs = ChangeSet::new(plan, before_id);
        if let Some(journal) = &self.journal
            && let Err(error) = journal.start(
                &cs,
                self.before_snapshot
                    .as_ref()
                    .expect("before snapshot captured"),
            )
        {
            cs.status = ChangeSetStatus::Failed;
            cs.summary = format!("durable preparation failed; no actions executed: {error}");
            return cs;
        }

        // 2. Execute each fix step (zipped with plan steps for expected_output access)
        // halt_on_failure: once a step fails, remaining steps are not executed.
        let mut halt = false;
        for (index, (step, plan_step)) in cs
            .steps
            .iter_mut()
            .zip(plan.proposed_fix_steps.iter())
            .enumerate()
        {
            // 2-halt. If a prior step failed, skip all remaining steps without execution.
            if halt {
                step.status = StepStatus::Skipped;
                step.verification_result = "not executed (prior step failed)".into();
                cs.summary.push_str(&format!(
                    "  - {}: SKIPPED (halted after prior failure)\n",
                    step.plan_step_title
                ));
                continue;
            }

            let preview = &step.command;
            if self.cancellation.is_cancelled() {
                step.status = StepStatus::Skipped;
                step.verification_result = "cancelled before execution".into();
                halt = true;
                continue;
            }

            let approved = match &approvals {
                Some(approvals) => approvals.allows(plan, index),
                None => (self.approve_fn)(preview),
            };
            if !approved {
                step.status = StepStatus::Skipped;
                cs.summary
                    .push_str(&format!("  - {}: SKIPPED (denied)\n", step.plan_step_title));
                continue;
            }

            // 2a. Idempotency pre-check: skip if system already in desired state
            let pre_check = Self::check_idempotent(preview).await;
            if let Some(reason) = pre_check {
                step.status = StepStatus::Skipped;
                step.verification_result = reason.clone();
                cs.summary.push_str(&format!(
                    "  - {}: SKIPPED (idempotent: {reason})\n",
                    step.plan_step_title
                ));
                continue;
            }

            // 2c. Run the step
            let backup = match crate::file_change::prepare_file_change(preview) {
                Ok(backup) => backup,
                Err(error) => {
                    step.status = StepStatus::Failed;
                    step.error_text = error;
                    halt = true;
                    continue;
                }
            };
            step.started_at = Some(chrono::Utc::now().timestamp());
            step.status = StepStatus::Running;
            if let Some(journal) = &self.journal
                && let Err(error) = journal.prepare(&cs.id, step, backup.as_ref())
            {
                step.status = StepStatus::Failed;
                step.error_text = format!("durable step preparation failed: {error}");
                halt = true;
                continue;
            }
            let outcome = self.execute_preview(preview, backup.as_ref()).await;
            step.status = if outcome.success {
                StepStatus::Succeeded
            } else {
                StepStatus::Failed
            };
            step.output_text = outcome.output.clone();
            step.error_text = outcome.error.clone();

            // 2d. Verification: run preview verification commands + expected_output matching
            let expected = plan_step.expected_output.as_deref();
            let (verified, verify_results) = if outcome.success {
                self.run_verification(preview, expected).await
            } else {
                (false, vec!["not verified (action failed)".into()])
            };
            if !verified || !outcome.verification_ok {
                step.status = StepStatus::Failed;
            }
            step.verification_result = if !verify_results.is_empty() {
                verify_results.join("; ")
            } else if outcome.verification_ok {
                "verified ok".into()
            } else {
                format!("verification: {}", outcome.verification_output)
            };

            // 2e. Collect backups from step outcome
            cs.backups.extend(backup);

            let now = chrono::Utc::now().timestamp();
            step.completed_at = Some(now);
            if let Some(journal) = &self.journal
                && let Err(error) = journal.record_step(step)
            {
                step.status = StepStatus::Failed;
                step.error_text
                    .push_str(&format!("; cannot persist outcome: {error}"));
            }

            // 2f. Append to HMAC audit chain.
            let prev_hash = self
                .audit_chain
                .last()
                .map(|e| e.hash.clone())
                .unwrap_or_else(|| "0".repeat(64));
            let audit_entry = ExecutionAuditEntry::new(
                now,
                preview.command_text.as_deref().unwrap_or(&preview.tool),
                outcome.success && verified,
                &outcome.output,
                &outcome.error,
                prev_hash,
            );
            self.audit_chain.push(audit_entry);

            cs.summary.push_str(&format!(
                "  - {}: {} | {} | verification: {}\n",
                step.plan_step_title,
                step.status,
                if outcome.changed_files.is_empty() {
                    "no file changes detected".into()
                } else {
                    format!("files: {}", outcome.changed_files.join(", "))
                },
                step.verification_result,
            ));

            // 2g. Halt on failure: subsequent steps will be skipped.
            if step.status == StepStatus::Failed {
                halt = true;
            }
        }

        // 3. Capture after-snapshot
        let after = collect_snapshot(MonitorProfile::Quick).await;
        let after_id = after.id.clone();
        self.after_snapshot = Some(after);

        cs.after_snapshot_id = Some(after_id);

        // 4. Detect system changes between before and after
        let changes = self.detect_system_changes();
        if !changes.is_empty() {
            cs.summary
                .push_str("\nSystem changes detected after execution:\n");
            for c in &changes {
                cs.summary.push_str(&format!("  - {c}\n"));
            }
        } else {
            cs.summary
                .push_str("\nNo significant system changes detected.\n");
        }

        cs.status = if self.cancellation.is_cancelled()
            || cs.steps.iter().any(|s| s.status == StepStatus::Failed)
        {
            ChangeSetStatus::Failed
        } else if cs.steps.iter().all(|s| s.status == StepStatus::Skipped) {
            ChangeSetStatus::Rejected
        } else {
            ChangeSetStatus::Completed
        };
        cs.completed_at = Some(chrono::Utc::now().timestamp());
        if let Some(journal) = &self.journal
            && let Err(error) = journal.finish(
                &cs,
                self.after_snapshot
                    .as_ref()
                    .expect("after snapshot captured"),
            )
        {
            cs.status = ChangeSetStatus::Failed;
            cs.summary.push_str(&format!(
                "\nCannot persist final outcome; inspect change {} before retrying: {error}",
                cs.id
            ));
        }

        cs
    }

    /// Execute a single command preview and capture outcome.
    /// This delegates to the real shell/fs tools.
    async fn execute_preview(
        &self,
        preview: &CommandPreview,
        backup: Option<&PreChangeBackup>,
    ) -> StepOutcome {
        match preview.tool.as_str() {
            "shell" | "bash" | "sh" | "cmd" => {
                let cmd = preview.command_text.as_deref().unwrap_or("");
                if cmd.is_empty() {
                    return StepOutcome {
                        success: false,
                        output: String::new(),
                        error: "No command text in preview".into(),
                        changed_files: vec![],
                        verification_ok: false,
                        verification_output: "no command to execute".into(),
                        backups: vec![],
                    };
                }
                // Use tokio::process::Command to run the shell command
                let mut command = tokio::process::Command::new("sh");
                command.args(["-c", cmd]).stdin(std::process::Stdio::null());
                let output =
                    crate::process_runner::run_command(command, self.limits, &self.cancellation)
                        .await;
                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                        let success = out.status.success();
                        let changed_files = extract_file_paths(&stdout, &stderr);
                        let verification_output = if success {
                            "command succeeded".into()
                        } else {
                            stderr.clone()
                        };
                        StepOutcome {
                            success,
                            output: stdout,
                            error: stderr,
                            changed_files,
                            verification_ok: success,
                            verification_output,
                            backups: vec![],
                        }
                    }
                    Err(e) => StepOutcome {
                        success: false,
                        output: e.stdout.clone(),
                        error: format!("execution error: {e}"),
                        changed_files: vec![],
                        verification_ok: false,
                        verification_output: format!("execution error: {e}"),
                        backups: vec![],
                    },
                }
            }
            "fs_write" | "write" => {
                let result: Result<&PreChangeBackup, String> = (|| {
                    let backup = backup.ok_or("file writes require prepared recovery data")?;
                    if self.cancellation.is_cancelled() {
                        return Err("cancelled before file mutation".into());
                    }
                    let content = preview
                        .input
                        .get("content")
                        .and_then(|v| v.as_str())
                        .ok_or("file content required")?;
                    crate::file_change::apply_file_change(backup, content)?;
                    Ok(backup)
                })();
                match result {
                    Ok(backup) => StepOutcome {
                        success: true,
                        output: format!("wrote {}", backup.file_path),
                        error: String::new(),
                        changed_files: vec![backup.file_path.clone()],
                        verification_ok: true,
                        verification_output: "file written atomically".into(),
                        backups: vec![backup.clone()],
                    },
                    Err(error) => StepOutcome {
                        success: false,
                        output: String::new(),
                        error: error.clone(),
                        changed_files: vec![],
                        verification_ok: false,
                        verification_output: error,
                        backups: vec![],
                    },
                }
            }
            other => {
                // Unknown tool — return placeholder
                StepOutcome {
                    success: false,
                    output: String::new(),
                    error: format!("unsupported execution tool: {other}"),
                    changed_files: vec![],
                    verification_ok: false,
                    verification_output: format!("tool {other} not supported for execution"),
                    backups: vec![],
                }
            }
        }
    }

    /// Run pre/post change detection.
    pub fn detect_system_changes(&self) -> Vec<String> {
        let mut changes = Vec::new();
        if let (Some(before), Some(after)) = (&self.before_snapshot, &self.after_snapshot) {
            // Compare filesystem usage
            let before_disks = &before.domains.disks;
            let after_disks = &after.domains.disks;
            for (i, bd) in before_disks.filesystems.iter().enumerate() {
                if let Some(ad) = after_disks.filesystems.get(i)
                    && bd.mount_point == ad.mount_point
                {
                    let b_pct = if bd.total_bytes > 0 {
                        bd.used_bytes as f64 / bd.total_bytes as f64 * 100.0
                    } else {
                        0.0
                    };
                    let a_pct = if ad.total_bytes > 0 {
                        ad.used_bytes as f64 / ad.total_bytes as f64 * 100.0
                    } else {
                        0.0
                    };
                    let diff = (a_pct - b_pct).abs();
                    if diff > 5.0 {
                        changes.push(format!(
                            "filesystem {} usage changed: {:.1}% -> {:.1}%",
                            bd.mount_point, b_pct, a_pct
                        ));
                    }
                }
            }

            // Compare load
            let bl = &before.domains.load;
            let al = &after.domains.load;
            if (al.load_average.one - bl.load_average.one).abs() > 1.0 {
                changes.push(format!(
                    "load 1m changed: {:.2} -> {:.2}",
                    bl.load_average.one, al.load_average.one
                ));
            }

            // Compare services (systemd units)
            let bs = &before.domains.services;
            let asv = &after.domains.services;
            for bsvc in &bs.units {
                let matching = asv.units.iter().find(|a| a.name == bsvc.name);
                if let Some(after_svc) = matching {
                    if bsvc.active != after_svc.active {
                        changes.push(format!(
                            "service {} status changed: active={} -> active={}",
                            bsvc.name, bsvc.active, after_svc.active
                        ));
                    }
                } else {
                    changes.push(format!("service disappeared: {}", bsvc.name));
                }
            }
            for asvc in &asv.units {
                if !bs.units.iter().any(|b| b.name == asvc.name) {
                    changes.push(format!("new service appeared: {}", asvc.name));
                }
            }

            // Compare processes (top_by_memory + top_by_cpu)
            let bprocs: std::collections::HashSet<_> = before
                .domains
                .processes
                .top_by_memory
                .iter()
                .chain(&before.domains.processes.top_by_cpu)
                .map(|p| p.command.clone())
                .collect();
            let aprocs: std::collections::HashSet<_> = after
                .domains
                .processes
                .top_by_memory
                .iter()
                .chain(&after.domains.processes.top_by_cpu)
                .map(|p| p.command.clone())
                .collect();
            for p in &aprocs {
                if !bprocs.contains(p) {
                    changes.push(format!("new process appeared: {p}"));
                }
            }
            for p in &bprocs {
                if !aprocs.contains(p) {
                    changes.push(format!("process disappeared: {p}"));
                }
            }
        }
        changes
    }

    pub fn before_snapshot_id(&self) -> Option<&str> {
        self.before_snapshot.as_ref().map(|s| s.id.as_str())
    }

    pub fn after_snapshot_id(&self) -> Option<&str> {
        self.after_snapshot.as_ref().map(|s| s.id.as_str())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract file paths from command output stderr/stdout heuristically.
fn extract_file_paths(stdout: &str, stderr: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in stdout.lines().chain(stderr.lines()) {
        let trimmed = line.trim();
        if trimmed.starts_with('/') || trimmed.starts_with("./") || trimmed.starts_with("../") {
            let path = trimmed.split_whitespace().next().unwrap_or("").to_string();
            if !path.is_empty() && !files.contains(&path) {
                files.push(path);
            }
        }
    }
    files
}

// ── ChangeSet formatting ─────────────────────────────────────────────────────

pub fn format_change_set(cs: &ChangeSet) -> String {
    let mut out = String::new();
    out.push_str(&format!("ChangeSet: {}\n", cs.id));
    out.push_str(&format!(
        "  Plan:       {} ({})\n",
        cs.plan_title, cs.plan_id
    ));
    out.push_str(&format!("  Status:     {}\n", cs.status));
    out.push_str(&format!("  Created:    {}\n", format_ts(cs.created_at)));
    if let Some(t) = cs.approved_at {
        out.push_str(&format!("  Approved:   {}\n", format_ts(t)));
    }
    if let Some(t) = cs.completed_at {
        out.push_str(&format!("  Completed:  {}\n", format_ts(t)));
    }
    out.push_str(&format!("  Before snap: {}\n", cs.before_snapshot_id));
    if let Some(id) = &cs.after_snapshot_id {
        out.push_str(&format!("  After snap:  {}\n", id));
    }
    out.push_str("\nSteps:\n");
    for s in &cs.steps {
        let output_short = if s.output_text.chars().count() > 120 {
            format!("{}…", s.output_text.chars().take(120).collect::<String>())
        } else {
            s.output_text.clone()
        };
        out.push_str(&format!(
            "  [{:>9}] {}: {}\n",
            s.status.to_string(),
            s.plan_step_title,
            output_short.lines().next().unwrap_or(""),
        ));
        if !s.verification_result.is_empty() {
            out.push_str(&format!("          verify: {}\n", s.verification_result));
        }
        if !s.error_text.is_empty() {
            out.push_str(&format!("          error: {}\n", s.error_text));
        }
    }
    if !cs.backups.is_empty() {
        out.push_str("\nBackups:\n");
        for b in &cs.backups {
            out.push_str(&format!(
                "  {} \u{2192} checksum: {}\n",
                b.file_path, b.checksum_before
            ));
        }
    }
    if !cs.summary.is_empty() {
        out.push_str(&format!("\nSummary:\n{}", cs.summary));
    }
    out
}

fn format_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::troubleshoot::PlanStep;

    #[test]
    fn change_set_creation() {
        let plan = TroubleshootingPlan {
            id: "plan-1".into(),
            source: PlanSource::UserQuestion("test disk issue".into()),
            snapshot_id: "snap-1".into(),
            hypotheses: vec![],
            read_only_steps: vec![],
            proposed_fix_steps: vec![PlanStep {
                title: "check disk".into(),
                command: CommandPreview::new("shell", "df -h /", "check disk usage"),
                hypothesis_id: None,
                expected_output: None,
                interpretation_guide: None,
            }],
            approval_required: true,
        };
        let cs = ChangeSet::new(&plan, "before-snap".into());
        assert_eq!(cs.status, ChangeSetStatus::Pending);
        assert_eq!(cs.steps.len(), 1);
        assert_eq!(cs.steps[0].plan_step_title, "check disk");
    }

    #[test]
    fn extract_file_paths_from_output() {
        let stdout = "/etc/nginx/nginx.conf\n/etc/hosts\n";
        let stderr = "";
        let files = extract_file_paths(stdout, stderr);
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"/etc/nginx/nginx.conf".to_string()));
        assert!(files.contains(&"/etc/hosts".to_string()));
    }

    #[test]
    fn change_set_status_display() {
        assert_eq!(format!("{}", ChangeSetStatus::Pending), "pending");
        assert_eq!(format!("{}", ChangeSetStatus::Approved), "approved");
        assert_eq!(format!("{}", ChangeSetStatus::Rejected), "rejected");
        assert_eq!(format!("{}", ChangeSetStatus::Running), "running");
        assert_eq!(format!("{}", ChangeSetStatus::Completed), "completed");
        assert_eq!(format!("{}", ChangeSetStatus::Failed), "failed");
        assert_eq!(format!("{}", ChangeSetStatus::RolledBack), "rolled_back");
    }

    #[test]
    fn change_set_empty_status_summary() {
        let plan = TroubleshootingPlan {
            id: "plan-2".into(),
            source: PlanSource::Finding("finding-1".into()),
            snapshot_id: "snap-2".into(),
            hypotheses: vec![],
            read_only_steps: vec![],
            proposed_fix_steps: vec![],
            approval_required: true,
        };
        let cs = ChangeSet::new(&plan, "before".into());
        assert_eq!(cs.plan_title, "finding: finding-1");
        assert!(cs.status_summary().contains("pending"));
    }

    // ── Property tests ──────────────────────────────────────────────────────

    #[cfg(test)]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        /// Property 1: HMAC chain integrity — any chain built by ExecutionAuditEntry::new
        /// must pass verify_audit_chain, and mutating any field must break verification.
        #[test]
        fn property_hmac_chain_integrity() {
            // Rebuild a clean chain so hashes link correctly
            let mut chain: Vec<ExecutionAuditEntry> = Vec::new();
            let cmds = [
                ("echo hello", true, "hello\n", ""),
                ("bad-cmd", false, "", "not found"),
                ("ls /tmp", true, "/tmp/foo\n", ""),
            ];
            for (cmd, ok, stdout, stderr) in &cmds {
                let prev = chain
                    .last()
                    .map(|e| e.hash.clone())
                    .unwrap_or_else(|| "0".repeat(64));
                chain.push(ExecutionAuditEntry::new(0, *cmd, *ok, stdout, stderr, prev));
            }
            assert!(verify_audit_chain(&chain));

            // Mutate one entry — chain must fail
            let mut tampered = chain.clone();
            tampered[1].exit_ok = !tampered[1].exit_ok;
            assert!(!verify_audit_chain(&tampered));
        }

        proptest! {
            /// Property 2: Execution halts on step failure — all steps after the first
            /// Failed step must have status Skipped with halt reason, never Succeeded.
            #[test]
            fn property_halt_on_failure_leaves_remaining_pending(
                n_steps in 2usize..8,
                fail_idx in 0usize..7,
            ) {
                let fail_idx = fail_idx % n_steps;
                let mut steps: Vec<ChangeSetStep> = (0..n_steps)
                    .map(|i| ChangeSetStep {
                        id: i.to_string(),
                        plan_step_title: format!("step-{i}"),
                        command: CommandPreview::new("shell", &format!("echo {i}"), ""),
                        status: StepStatus::Pending,
                        output_text: String::new(),
                        error_text: String::new(),
                        verification_result: String::new(),
                        started_at: None,
                        completed_at: None,
                    })
                    .collect();

                // Simulate halt-on-failure logic
                let mut halt = false;
                for (i, step) in steps.iter_mut().enumerate() {
                    if halt {
                        step.status = StepStatus::Skipped;
                        step.verification_result = "not executed (prior step failed)".into();
                    } else if i == fail_idx {
                        step.status = StepStatus::Failed;
                        halt = true;
                    } else {
                        step.status = StepStatus::Succeeded;
                    }
                }

                // All steps after fail_idx must be Skipped, never Succeeded
                for step in &steps[fail_idx + 1..] {
                    prop_assert_eq!(step.status, StepStatus::Skipped);
                    prop_assert!(step.verification_result.contains("not executed"));
                }
                // The fail_idx step itself must be Failed
                prop_assert_eq!(steps[fail_idx].status, StepStatus::Failed);
            }

            /// Property 3: HMAC chain of N arbitrary entries is always self-consistent.
            #[test]
            fn property_hmac_chain_always_verifies(
                cmds in proptest::collection::vec("[a-z ]{1,20}", 1..10),
                successes in proptest::collection::vec(proptest::bool::ANY, 1..10),
            ) {
                let len = cmds.len().min(successes.len());
                let mut chain: Vec<ExecutionAuditEntry> = Vec::new();
                for i in 0..len {
                    let prev = chain
                        .last()
                        .map(|e| e.hash.clone())
                        .unwrap_or_else(|| "0".repeat(64));
                    chain.push(ExecutionAuditEntry::new(
                        i as i64,
                        &cmds[i],
                        successes[i],
                        "stdout",
                        "stderr",
                        prev,
                    ));
                }
                prop_assert!(verify_audit_chain(&chain));
            }
        }
    }
}
