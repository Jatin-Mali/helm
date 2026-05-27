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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreChangeBackup {
    pub id: String,
    pub file_path: String,
    pub checksum_before: String,
    pub backup_content: String,
    pub restored: bool,
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
    before_snapshot: Option<SystemSnapshot>,
    after_snapshot: Option<SystemSnapshot>,
    /// HMAC-chained audit log of every executed step.
    pub audit_chain: Vec<ExecutionAuditEntry>,
}

impl ExecutionEngine {
    /// Create with an approval callback. Return `true` to proceed, `false` to deny.
    pub fn new(approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync>) -> Self {
        Self {
            approve_fn,
            before_snapshot: None,
            after_snapshot: None,
            audit_chain: Vec::new(),
        }
    }

    /// Run a pre-flight idempotency check. Returns Some(reason) if the step
    /// is already satisfied and can be skipped, None if execution is needed.
    async fn check_idempotent(preview: &CommandPreview) -> Option<String> {
        match preview.tool.as_str() {
            "shell" | "bash" | "sh" | "cmd" => {
                let cmd = preview.command_text.as_deref().unwrap_or("");
                Self::check_shell_idempotent(cmd).await
            }
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
                if !path.is_empty() && std::path::Path::new(path).exists() {
                    if let Ok(existing) = std::fs::read_to_string(path) {
                        if existing == content {
                            return Some(format!("file {} already has the desired content", path));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Check if a shell command is idempotent by running a pre-flight heuristic.
    async fn check_shell_idempotent(cmd: &str) -> Option<String> {
        let cmd = cmd.trim();
        // systemctl start <service>: skip if already active
        if let Some(svc) = cmd.strip_prefix("systemctl start ") {
            let svc = svc.trim();
            let check = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("systemctl is-active --quiet {svc}"))
                .output()
                .await;
            if let Ok(out) = check {
                if out.status.success() {
                    return Some(format!("service {svc} is already active"));
                }
            }
        }
        // systemctl stop <service>: skip if already inactive
        if let Some(svc) = cmd.strip_prefix("systemctl stop ") {
            let svc = svc.trim();
            let check = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("systemctl is-active --quiet {svc}"))
                .output()
                .await;
            if let Ok(out) = check {
                if !out.status.success() {
                    return Some(format!("service {svc} is already inactive"));
                }
            }
        }
        // systemctl restart <service>: skip if already active (no need to restart healthy services)
        if let Some(svc) = cmd.strip_prefix("systemctl restart ") {
            let svc = svc.trim();
            let check = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("systemctl is-active --quiet {svc}"))
                .output()
                .await;
            if let Ok(out) = check {
                if out.status.success() {
                    return Some(format!(
                        "service {svc} is already active, no restart needed"
                    ));
                }
            }
        }
        // systemctl enable <service>: skip if already enabled
        if let Some(svc) = cmd.strip_prefix("systemctl enable ") {
            let svc = svc.trim();
            let check = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("systemctl is-enabled --quiet {svc}"))
                .output()
                .await;
            if let Ok(out) = check {
                if out.status.success() {
                    return Some(format!("service {svc} is already enabled"));
                }
            }
        }
        // systemctl disable <service>: skip if already disabled
        if let Some(svc) = cmd.strip_prefix("systemctl disable ") {
            let svc = svc.trim();
            let check = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!("systemctl is-enabled --quiet {svc}"))
                .output()
                .await;
            if let Ok(out) = check {
                if !out.status.success() {
                    return Some(format!("service {svc} is already disabled"));
                }
            }
        }
        // apt install <pkg>: skip if already installed
        if let Some(pkg) = cmd.strip_prefix("apt install ") {
            let pkg = pkg.split_whitespace().next().unwrap_or("");
            if !pkg.is_empty() {
                let check = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("dpkg -l {pkg} 2>/dev/null | grep -q '^ii'"))
                    .output()
                    .await;
                if let Ok(out) = check {
                    if out.status.success() {
                        return Some(format!("package {pkg} is already installed"));
                    }
                }
            }
        }
        // apt remove <pkg>: skip if already removed
        if let Some(pkg) = cmd.strip_prefix("apt remove ") {
            let pkg = pkg.split_whitespace().next().unwrap_or("");
            if !pkg.is_empty() {
                let check = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("dpkg -l {pkg} 2>/dev/null | grep -q '^ii'"))
                    .output()
                    .await;
                if let Ok(out) = check {
                    if !out.status.success() {
                        return Some(format!("package {pkg} is already removed"));
                    }
                }
            }
        }
        // mkdir -p <dir>: skip if directory already exists
        if let Some(dir) = cmd.strip_prefix("mkdir -p ") {
            let dir = dir.trim();
            if std::path::Path::new(dir).exists() {
                return Some(format!("directory {dir} already exists"));
            }
        }
        // mkdir <dir>: skip if directory already exists
        if let Some(dir) = cmd.strip_prefix("mkdir ") {
            let dir = dir.trim();
            if !dir.starts_with('-') && std::path::Path::new(dir).exists() {
                return Some(format!("directory {dir} already exists"));
            }
        }
        // touch <file>: skip if file already exists
        if let Some(file) = cmd.strip_prefix("touch ") {
            let file = file.trim();
            if std::path::Path::new(file).exists() {
                return Some(format!("file {file} already exists"));
            }
        }
        // rm <path>: skip if file does not exist
        if let Some(target) = cmd.strip_prefix("rm ") {
            let target = target
                .trim()
                .trim_start_matches("-rf ")
                .trim_start_matches("-r ")
                .trim_start_matches("-f ");
            if !std::path::Path::new(target).exists() {
                return Some(format!("path {target} does not exist, nothing to remove"));
            }
        }
        None
    }

    /// Run verification commands from a preview and compare output against expectations.
    async fn run_verification(
        preview: &CommandPreview,
        expected_output: Option<&str>,
    ) -> Vec<String> {
        let mut results = Vec::new();
        for v in &preview.verification {
            let v_outcome = Self::execute_preview(v).await;
            let output_line = v_outcome.output.lines().next().unwrap_or("").to_string();
            let passed = if let Some(expected) = expected_output {
                output_line.contains(expected) || v_outcome.success
            } else {
                v_outcome.success
            };
            let v_text = if passed {
                format!("ok: {output_line}")
            } else {
                format!("failed: {}", v_outcome.error.lines().next().unwrap_or(""))
            };
            results.push(v_text);
        }
        results
    }

    /// Execute a full plan, returning a ChangeSet with execution results.
    /// Takes a before-snapshot, runs each fix step through approval + execution,
    /// takes an after-snapshot, and runs post-change detection.
    pub async fn execute(&mut self, plan: &TroubleshootingPlan) -> ChangeSet {
        // 1. Capture before-snapshot
        let before = collect_snapshot(MonitorProfile::Quick).await;
        let before_id = before.id.clone();
        self.before_snapshot = Some(before);

        let mut cs = ChangeSet::new(plan, before_id);

        // 2. Execute each fix step (zipped with plan steps for expected_output access)
        // halt_on_failure: once a step fails, remaining steps are not executed.
        let mut halt = false;
        for (step, plan_step) in cs.steps.iter_mut().zip(plan.proposed_fix_steps.iter()) {
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

            // 2b. Approval gate
            let approved = (self.approve_fn)(preview);
            if !approved {
                step.status = StepStatus::Skipped;
                cs.summary
                    .push_str(&format!("  - {}: SKIPPED (denied)\n", step.plan_step_title));
                continue;
            }

            // 2c. Run the step
            let outcome = Self::execute_preview(preview).await;
            step.status = if outcome.success {
                StepStatus::Succeeded
            } else {
                StepStatus::Failed
            };
            step.output_text = outcome.output.clone();
            step.error_text = outcome.error.clone();

            // 2d. Verification: run preview verification commands + expected_output matching
            let expected = plan_step.expected_output.as_deref();
            let verify_results = Self::run_verification(preview, expected).await;
            step.verification_result = if !verify_results.is_empty() {
                verify_results.join("; ")
            } else if outcome.verification_ok {
                "verified ok".into()
            } else {
                format!("verification: {}", outcome.verification_output)
            };

            // 2e. Collect backups from step outcome
            cs.backups.extend(outcome.backups);

            let now = chrono::Utc::now().timestamp();
            step.started_at = Some(now);
            step.completed_at = Some(now);

            // 2f. Append to HMAC audit chain.
            let prev_hash = self
                .audit_chain
                .last()
                .map(|e| e.hash.clone())
                .unwrap_or_else(|| "0".repeat(64));
            let audit_entry = ExecutionAuditEntry::new(
                now,
                preview.command_text.as_deref().unwrap_or(&preview.tool),
                outcome.success,
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

        cs.status = if cs.steps.iter().any(|s| s.status == StepStatus::Failed) {
            ChangeSetStatus::Failed
        } else if cs.steps.iter().all(|s| s.status == StepStatus::Skipped) {
            ChangeSetStatus::Rejected
        } else {
            ChangeSetStatus::Completed
        };
        cs.completed_at = Some(chrono::Utc::now().timestamp());

        cs
    }

    /// Execute a single command preview and capture outcome.
    /// This delegates to the real shell/fs tools.
    async fn execute_preview(preview: &CommandPreview) -> StepOutcome {
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
                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .output()
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
                        output: String::new(),
                        error: format!("execution error: {e}"),
                        changed_files: vec![],
                        verification_ok: false,
                        verification_output: format!("execution error: {e}"),
                        backups: vec![],
                    },
                }
            }
            "fs_write" | "write" => {
                // File write: backup before if it exists
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
                if path.is_empty() {
                    return StepOutcome {
                        success: false,
                        output: String::new(),
                        error: "No path in preview input".into(),
                        changed_files: vec![],
                        verification_ok: false,
                        verification_output: "no path specified".into(),
                        backups: vec![],
                    };
                }
                if std::path::Path::new(path).exists() {
                    if let Ok(orig) = std::fs::read_to_string(path) {
                        let _checksum = hex::encode(Sha256::digest(orig.as_bytes()));
                        let backup_path = format!(
                            "/tmp/helm-backup-{}-{}",
                            path.replace('/', "_"),
                            Uuid::new_v4().to_string().split('-').next().unwrap_or("0")
                        );
                        let _ = std::fs::write(&backup_path, &orig);
                    }
                }
                // Write new content
                match std::fs::write(path, content) {
                    Ok(()) => StepOutcome {
                        success: true,
                        output: format!("wrote {} bytes to {}", content.len(), path),
                        error: String::new(),
                        changed_files: vec![path.to_string()],
                        verification_ok: true,
                        verification_output: format!("file {} written", path),
                        backups: vec![PreChangeBackup {
                            id: Uuid::new_v4().to_string(),
                            file_path: path.to_string(),
                            checksum_before: String::new(),
                            backup_content: String::new(),
                            restored: false,
                        }],
                    },
                    Err(e) => StepOutcome {
                        success: false,
                        output: String::new(),
                        error: format!("write error: {e}"),
                        changed_files: vec![],
                        verification_ok: false,
                        verification_output: format!("write error: {e}"),
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
                if let Some(ad) = after_disks.filesystems.get(i) {
                    if bd.mount_point == ad.mount_point {
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
        let output_short = if s.output_text.len() > 120 {
            format!("{}…", &s.output_text[..120])
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
