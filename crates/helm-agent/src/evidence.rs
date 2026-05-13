//! Structured evidence model for v1.6 diagnose and permission gates.
//!
//! Replaces the previous flat df/free/loadavg snapshot with a rich,
//! inspectable model that captures what the agent knows, what it assumes,
//! and the action it intends to take.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Structured evidence emitted before permission-sensitive tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredEvidence {
    /// Where the evidence came from (tool names, file paths, commands).
    pub inspected_sources: Vec<String>,

    /// Concrete observations and measurements (key → value text).
    pub findings: Vec<Finding>,

    /// What the agent is assuming but cannot prove.
    pub assumptions: Vec<String>,

    /// Confidence/disagreement among findings (low / medium / high).
    pub uncertainty: Uncertainty,

    /// What the agent intends to do next.
    pub proposed_actions: Vec<ProposedAction>,

    /// Affected paths, services, or hosts.
    pub blast_radius: BlastRadius,

    /// Whether the plan includes a revert path.
    pub rollback: RollbackStatus,

    /// Exact tool calls or commands the agent would run.
    /// Separate from proposed_actions to distinguish between what
    /// would be executed vs. what preparatory steps are planned.
    pub exact_tool_calls: Vec<ToolCallPreview>,
}

/// A single tool call preview: the tool name and its serialised input.
/// Mirrors what the agent will pass to the tool registry on approval.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallPreview {
    /// Tool name (e.g. "shell", "fs_write", "git").
    pub tool: String,
    /// JSON-serialised input that would be passed to the tool.
    pub tool_input: String,
    /// Human-readable summary of what this call does.
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    /// Human-readable label (e.g. "root partition usage").
    pub label: String,
    /// Observed value (e.g. "85%").
    pub value: String,
    /// Source tool or command that produced this finding.
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Uncertainty {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposedAction {
    /// Human-readable description of the action.
    pub description: String,
    /// Exact tool name to be called.
    pub tool: String,
    /// Tool input parameters (serialised as JSON).
    pub tool_input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlastRadius {
    /// Filesystem paths that may be affected.
    pub paths: Vec<String>,
    /// Services that may be restarted or disrupted.
    pub services: Vec<String>,
    /// External hosts that may receive traffic.
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RollbackStatus {
    /// Whether a known revert path exists.
    pub available: bool,
    /// Human-readable description of the revert plan (commands etc.).
    pub description: String,
}

impl Default for StructuredEvidence {
    fn default() -> Self {
        Self {
            inspected_sources: vec!["disk".into(), "process".into(), "shell".into()],
            findings: vec![Finding {
                label: "machine state".into(),
                value: "pending collection".into(),
                source: "agent initialization".into(),
            }],
            assumptions: Vec::new(),
            uncertainty: Uncertainty::Medium,
            proposed_actions: Vec::new(),
            blast_radius: BlastRadius {
                paths: Vec::new(),
                services: Vec::new(),
                hosts: Vec::new(),
            },
            rollback: RollbackStatus {
                available: false,
                description: String::new(),
            },
            exact_tool_calls: Vec::new(),
        }
    }
}

/// Collects a structured system-state snapshot for evidence display.
/// The `tool_name` and `input` are the pending tool call that triggered
/// evidence collection; they are used to derive concrete blast radius
/// and rollback recommendations.
pub async fn collect_system_evidence(tool_name: &str, input: &Value) -> StructuredEvidence {
    let mut sources = Vec::new();
    let mut findings = Vec::new();

    // Disk usage
    if let Ok(output) = tokio::process::Command::new("df").arg("-h").output().await {
        sources.push("df -h".into());
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        findings.push(Finding {
            label: "disk usage".into(),
            value: text.trim().lines().take(8).collect::<Vec<_>>().join("\n"),
            source: "df -h".into(),
        });
    }
    // Disk inode usage
    if let Ok(output) = tokio::process::Command::new("df").arg("-i").output().await {
        sources.push("df -i".into());
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        findings.push(Finding {
            label: "disk inodes".into(),
            value: text.trim().lines().take(8).collect::<Vec<_>>().join("\n"),
            source: "df -i".into(),
        });
    }
    // Memory
    if let Ok(output) = tokio::process::Command::new("free")
        .arg("-h")
        .output()
        .await
    {
        sources.push("free -h".into());
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        findings.push(Finding {
            label: "memory".into(),
            value: text.trim().lines().take(4).collect::<Vec<_>>().join("\n"),
            source: "free -h".into(),
        });
    }
    // Load average
    if let Ok(output) = tokio::process::Command::new("cat")
        .arg("/proc/loadavg")
        .output()
        .await
    {
        sources.push("/proc/loadavg".into());
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        findings.push(Finding {
            label: "system load".into(),
            value: text.trim().to_owned(),
            source: "/proc/loadavg".into(),
        });
    }
    // Uptime
    if let Ok(output) = tokio::process::Command::new("uptime").output().await {
        sources.push("uptime".into());
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        findings.push(Finding {
            label: "uptime".into(),
            value: text.trim().to_owned(),
            source: "uptime".into(),
        });
    }
    // Kernel version
    if let Ok(output) = tokio::process::Command::new("uname")
        .arg("-a")
        .output()
        .await
    {
        sources.push("uname -a".into());
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        findings.push(Finding {
            label: "kernel".into(),
            value: text.trim().to_owned(),
            source: "uname -a".into(),
        });
    }
    // Logged-in users
    if let Ok(output) = tokio::process::Command::new("who").output().await {
        sources.push("who".into());
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let trimmed = text.trim();
        findings.push(Finding {
            label: "logged in users".into(),
            value: if trimmed.is_empty() {
                "(no users logged in)".into()
            } else {
                trimmed.lines().take(8).collect::<Vec<_>>().join("\n")
            },
            source: "who".into(),
        });
    }
    // Top memory-consuming processes (bounded to 5)
    if let Ok(output) = tokio::process::Command::new("ps")
        .args(["-eo", "pid,comm,%mem,%cpu", "--sort=-%mem"])
        .output()
        .await
    {
        sources.push("ps -eo pid,comm,%mem,%cpu --sort=-%mem".into());
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        // Skip header line, take up to 5 process lines
        let lines: Vec<&str> = text.trim().lines().collect();
        let header = lines.first().copied().unwrap_or("");
        let top = lines
            .iter()
            .skip(1)
            .take(5)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        findings.push(Finding {
            label: "top processes".into(),
            value: format!("{header}\n{top}"),
            source: "ps --sort=-%mem".into(),
        });
    }
    // Network interfaces
    if let Ok(output) = tokio::process::Command::new("ip")
        .args(["-br", "addr"])
        .output()
        .await
    {
        sources.push("ip -br addr".into());
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        findings.push(Finding {
            label: "network interfaces".into(),
            value: text.trim().to_owned(),
            source: "ip -br addr".into(),
        });
    }

    let input_json = serde_json::to_string(input).unwrap_or_else(|_| "<invalid>".into());

    let (proposed_action, blast_radius, rollback) =
        derive_tool_impact(tool_name, input, &input_json);

    let exact_tool_call = ToolCallPreview {
        tool: tool_name.to_owned(),
        tool_input: input_json.clone(),
        summary: proposed_action.description.clone(),
    };

    StructuredEvidence {
        inspected_sources: sources,
        findings,
        assumptions: vec![
            "system is in a steady state at time of evidence collection".into(),
            "agent is running with the permissions of the current user".into(),
        ],
        uncertainty: Uncertainty::Low,
        proposed_actions: vec![proposed_action],
        blast_radius,
        rollback,
        exact_tool_calls: vec![exact_tool_call],
    }
}

/// Derive tool-specific proposed action, blast radius, and rollback
/// from the actual tool name and input value.
fn derive_tool_impact(
    tool_name: &str,
    input: &Value,
    input_json: &str,
) -> (ProposedAction, BlastRadius, RollbackStatus) {
    let action = ProposedAction {
        description: format!("execute {tool_name} with provided parameters"),
        tool: tool_name.to_owned(),
        tool_input: input_json.to_owned(),
    };

    match tool_name {
        "fs_read" => {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            (
                ProposedAction {
                    description: format!("read file at {path}"),
                    ..action
                },
                BlastRadius {
                    paths: vec![path.to_owned()],
                    services: vec![],
                    hosts: vec![],
                },
                RollbackStatus {
                    available: true,
                    description: "fs_read is non-mutating; no rollback needed".into(),
                },
            )
        }
        "fs_write" => {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let backup = format!("{path}.helm-bak");
            (
                ProposedAction {
                    description: format!("write file at {path}"),
                    ..action
                },
                BlastRadius {
                    paths: vec![path.to_owned()],
                    services: vec![],
                    hosts: vec![],
                },
                RollbackStatus {
                    available: true,
                    description: format!(
                        "copy {path} to {backup} before writing; restore with mv {backup} {path}"
                    ),
                },
            )
        }
        "disk" => {
            let disk_action = input.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("/");
            (
                ProposedAction {
                    description: format!("disk {disk_action} on {path}"),
                    ..action
                },
                BlastRadius {
                    paths: vec![path.to_owned()],
                    services: vec![],
                    hosts: vec![],
                },
                RollbackStatus {
                    available: true,
                    description: "disk inspection is non-mutating; no rollback needed".into(),
                },
            )
        }
        "process" => {
            let pid = input.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
            let proc_action = input.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            if proc_action == "kill" {
                (
                    ProposedAction {
                        description: format!("kill process {pid}"),
                        ..action
                    },
                    BlastRadius {
                        paths: vec![],
                        services: vec![format!("pid {pid}")],
                        hosts: vec![],
                    },
                    RollbackStatus {
                        available: true,
                        description: format!(
                            "restart with the original command used to start pid {pid}; if systemd service: systemctl restart <unit>"
                        ),
                    },
                )
            } else {
                (
                    ProposedAction {
                        description: format!("inspect process {pid}"),
                        ..action
                    },
                    BlastRadius {
                        paths: vec![],
                        services: vec![],
                        hosts: vec![],
                    },
                    RollbackStatus {
                        available: true,
                        description: "process inspection is non-mutating; no rollback needed"
                            .into(),
                    },
                )
            }
        }
        "shell" => {
            let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            let cwd = input.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
            let stdout_dest = input.get("redirect_stdout_to").and_then(|v| v.as_str());
            let stderr_dest = input.get("redirect_stderr_to").and_then(|v| v.as_str());
            let mut paths = vec![cwd.to_owned()];
            if let Some(p) = stdout_dest {
                paths.push(p.to_owned());
            }
            if let Some(p) = stderr_dest {
                if !paths.contains(&p.to_owned()) {
                    paths.push(p.to_owned());
                }
            }
            let rollback = if stdout_dest.is_some() {
                RollbackStatus {
                    available: true,
                    description: "output was redirected; back up target files before execution"
                        .to_owned(),
                }
            } else {
                RollbackStatus {
                    available: false,
                    description: "shell execution has no automatic rollback — review the command carefully before approving".into(),
                }
            };
            (
                ProposedAction {
                    description: format!("execute shell command: {cmd}"),
                    ..action
                },
                BlastRadius {
                    paths,
                    services: vec![],
                    hosts: vec![],
                },
                rollback,
            )
        }
        "git" => {
            let git_action = input.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let repo_path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            let rollback = match git_action {
                "add" => "git reset HEAD <files> to unstage".into(),
                "commit" => "git reset --soft HEAD~1 to undo the commit".into(),
                "push" => "git push --force origin <previous-sha> to revert".into(),
                "pull" => "git reset --hard ORIG_HEAD to undo the pull".into(),
                "checkout" => "git checkout - to return to previous branch".into(),
                "clone" => "rm -rf <clone-target> to remove the clone".into(),
                _ => "git action is read-only; no rollback needed".into(),
            };
            let is_mutating = matches!(
                git_action,
                "add" | "commit" | "push" | "pull" | "checkout" | "clone" | "stash"
            );
            (
                ProposedAction {
                    description: format!("git {git_action} in {repo_path}"),
                    ..action
                },
                BlastRadius {
                    paths: vec![repo_path.to_owned()],
                    services: vec![],
                    hosts: vec![],
                },
                RollbackStatus {
                    available: is_mutating,
                    description: rollback,
                },
            )
        }
        "http" => {
            let url = input.get("url").and_then(|v| v.as_str()).unwrap_or("?");
            let http_action = input.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let is_mutating = matches!(http_action, "post" | "put" | "delete" | "patch");
            (
                ProposedAction {
                    description: format!("HTTP {http_action} {url}"),
                    ..action
                },
                BlastRadius {
                    paths: vec![],
                    services: vec![],
                    hosts: vec![url.to_owned()],
                },
                RollbackStatus {
                    available: !is_mutating,
                    description: if is_mutating {
                        format!(
                            "HTTP {http_action} is mutating; no automatic rollback — verify the request is safe before approving"
                        )
                    } else {
                        "HTTP GET/HEAD is non-mutating; no rollback needed".into()
                    },
                },
            )
        }
        "service" => {
            let svc_name = input.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let svc_action = input.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let is_mutating = matches!(
                svc_action,
                "start" | "stop" | "restart" | "enable" | "disable"
            );
            (
                ProposedAction {
                    description: format!("service {svc_action} {svc_name}"),
                    ..action
                },
                BlastRadius {
                    paths: vec![],
                    services: vec![svc_name.to_owned()],
                    hosts: vec![],
                },
                RollbackStatus {
                    available: is_mutating,
                    description: if is_mutating {
                        format!(
                            "undo with systemctl {op} {svc_name}",
                            op = opposite_service_action(svc_action)
                        )
                    } else {
                        "service status/logs inspection is non-mutating; no rollback needed".into()
                    },
                },
            )
        }
        "package" => {
            let pkg = input.get("package").and_then(|v| v.as_str()).unwrap_or("?");
            let pkg_action = input.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let is_mutating = matches!(pkg_action, "install" | "remove" | "update" | "upgrade");
            (
                ProposedAction {
                    description: format!("package {pkg_action} {pkg}"),
                    ..action
                },
                BlastRadius {
                    paths: vec!["/usr/bin".into(), "/usr/lib".into()],
                    services: vec![],
                    hosts: vec![],
                },
                RollbackStatus {
                    available: is_mutating,
                    description: if is_mutating {
                        format!("uninstall with the detected package manager: remove {pkg}")
                    } else {
                        "package search/detect is non-mutating; no rollback needed".into()
                    },
                },
            )
        }
        _ => (
            action,
            BlastRadius {
                paths: vec![],
                services: vec![],
                hosts: vec![],
            },
            RollbackStatus {
                available: false,
                description: format!(
                    "tool {tool_name}: review the operation before approving — no automatic rollback"
                ),
            },
        ),
    }
}

fn opposite_service_action(action: &str) -> &str {
    match action {
        "start" => "stop",
        "stop" => "start",
        "restart" => "stop", // no true opposite; stop is closest
        "enable" => "disable",
        "disable" => "enable",
        _ => "is-active",
    }
}
