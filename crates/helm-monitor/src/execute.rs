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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Outcome of a single executed step.
#[derive(Debug, Clone)]
pub struct StepOutcome {
    pub success: bool,
    pub output: String,
    pub error: String,
    pub changed_files: Vec<String>,
    pub verification_ok: bool,
    pub verification_output: String,
}

/// Drives controlled execution of a TroubleshootingPlan with pre/post snapshots,
/// approval gating, change detection, and verification.
pub struct ExecutionEngine {
    approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync>,
    before_snapshot: Option<SystemSnapshot>,
    after_snapshot: Option<SystemSnapshot>,
}

impl ExecutionEngine {
    /// Create with an approval callback. Return `true` to proceed, `false` to deny.
    pub fn new(approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync>) -> Self {
        Self {
            approve_fn,
            before_snapshot: None,
            after_snapshot: None,
        }
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

        // 2. Execute each fix step
        for step in &mut cs.steps {
            let preview = &step.command;

            // 2a. Approval gate
            let approved = (self.approve_fn)(preview);
            if !approved {
                step.status = StepStatus::Skipped;
                cs.summary
                    .push_str(&format!("  - {}: SKIPPED (denied)\n", step.plan_step_title));
                continue;
            }

            // 2b. Run the step
            let outcome = Self::execute_preview(preview).await;
            step.status = if outcome.success {
                StepStatus::Succeeded
            } else {
                StepStatus::Failed
            };
            step.output_text = outcome.output.clone();
            step.error_text = outcome.error.clone();
            step.verification_result = if outcome.verification_ok {
                "verified ok".into()
            } else {
                format!("verification: {}", outcome.verification_output)
            };
            let now = chrono::Utc::now().timestamp();
            step.started_at = Some(now);
            step.completed_at = Some(now);

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
                        }
                    }
                    Err(e) => StepOutcome {
                        success: false,
                        output: String::new(),
                        error: format!("execution error: {e}"),
                        changed_files: vec![],
                        verification_ok: false,
                        verification_output: format!("execution error: {e}"),
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
                    },
                    Err(e) => StepOutcome {
                        success: false,
                        output: String::new(),
                        error: format!("write error: {e}"),
                        changed_files: vec![],
                        verification_ok: false,
                        verification_output: format!("write error: {e}"),
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
}
