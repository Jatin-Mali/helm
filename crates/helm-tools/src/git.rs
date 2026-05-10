//! Git version-control tool for HELM.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use tokio::process::Command;

use crate::tool::{Tool, ToolContext, ToolError, ToolOutput};

/// Tool that exposes common git operations to the agent.
#[derive(Debug, Default)]
pub struct GitTool;

#[async_trait]
impl Tool for GitTool {
    fn name(&self) -> &'static str {
        "git"
    }

    fn description(&self) -> &'static str {
        "Git version-control operations: status, log, diff, add, commit, push, pull, branch, checkout, stash, clone."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "log", "diff", "add", "commit", "push", "pull", "branch", "checkout", "stash", "clone"],
                    "description": "Git operation to perform."
                },
                "path": {
                    "type": "string",
                    "description": "Working directory for the git operation (default: tool context working dir)."
                },
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "File paths for add, diff, or checkout actions."
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (required for action=commit)."
                },
                "branch": {
                    "type": "string",
                    "description": "Branch name for branch/checkout actions."
                },
                "remote": {
                    "type": "string",
                    "description": "Remote name for push/pull (default: origin)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of log entries (default: 20).",
                    "minimum": 1,
                    "maximum": 1000
                },
                "staged": {
                    "type": "boolean",
                    "description": "For diff: show staged (cached) changes only."
                },
                "url": {
                    "type": "string",
                    "description": "Repository URL (required for action=clone)."
                },
                "stash_action": {
                    "type": "string",
                    "enum": ["push", "pop", "list", "drop"],
                    "description": "Sub-action for stash (default: push)."
                },
                "create": {
                    "type": "boolean",
                    "description": "For branch: create a new branch with the given name."
                },
                "delete": {
                    "type": "boolean",
                    "description": "For branch: delete the named branch."
                },
                "all": {
                    "type": "boolean",
                    "description": "For log/branch: include all refs and remotes."
                },
                "short": {
                    "type": "boolean",
                    "description": "For status: use --short format."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = input["action"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidInput("action is required".into()))?;

        let cwd = if let Some(p) = input["path"].as_str() {
            PathBuf::from(p)
        } else {
            ctx.working_dir.clone()
        };

        match action {
            "status" => git_status(&cwd, &input).await,
            "log" => git_log(&cwd, &input).await,
            "diff" => git_diff(&cwd, &input).await,
            "add" => git_add(&cwd, &input).await,
            "commit" => git_commit(&cwd, &input).await,
            "push" => git_push(&cwd, &input).await,
            "pull" => git_pull(&cwd, &input).await,
            "branch" => git_branch(&cwd, &input).await,
            "checkout" => git_checkout(&cwd, &input).await,
            "stash" => git_stash(&cwd, &input).await,
            "clone" => git_clone(&cwd, &input).await,
            _ => Err(ToolError::InvalidInput(format!("unknown action: {action}"))),
        }
    }
}

/// Run a git command in the given directory and collect stdout/stderr/status.
async fn run_git(cwd: &Path, args: &[&str]) -> Result<(String, String, bool), ToolError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((stdout, stderr, output.status.success()))
}

fn make_output(stdout: &str, stderr: &str, success: bool) -> Result<ToolOutput, ToolError> {
    let content = if success {
        if stdout.trim().is_empty() {
            "(no output)".to_owned()
        } else {
            stdout.to_owned()
        }
    } else {
        let msg = stderr.trim();
        if msg.is_empty() {
            stdout.trim().to_owned()
        } else {
            format!("error: {msg}")
        }
    };
    Ok(ToolOutput {
        content,
        success,
        metadata: Map::new(),
    })
}

async fn git_status(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let mut cmd = Command::new("git");
    cmd.arg("status").current_dir(cwd);
    if input["short"].as_bool().unwrap_or(false) {
        cmd.arg("--short");
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    make_output(&stdout, &stderr, output.status.success())
}

async fn git_log(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let limit = input["limit"].as_u64().unwrap_or(20);
    let mut cmd = Command::new("git");
    cmd.args(["log", "--oneline", "--decorate"])
        .arg(format!("-{limit}"))
        .current_dir(cwd);
    if input["all"].as_bool().unwrap_or(false) {
        cmd.arg("--all");
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    make_output(&stdout, &stderr, output.status.success())
}

async fn git_diff(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let staged = input["staged"].as_bool().unwrap_or(false);
    let files: Vec<String> = input["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut cmd = Command::new("git");
    cmd.arg("diff").current_dir(cwd);
    if staged {
        cmd.arg("--cached");
    }
    if !files.is_empty() {
        cmd.arg("--");
        cmd.args(&files);
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    make_output(&stdout, &stderr, output.status.success())
}

async fn git_add(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let files: Vec<String> = input["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if files.is_empty() {
        return Err(ToolError::InvalidInput(
            "add requires at least one file in 'files'".into(),
        ));
    }

    let mut cmd = Command::new("git");
    cmd.arg("add").arg("--").args(&files).current_dir(cwd);
    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    make_output(&stdout, &stderr, output.status.success())
}

async fn git_commit(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let message = input["message"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidInput("commit requires 'message'".into()))?;

    let (stdout, stderr, success) = run_git(cwd, &["commit", "-m", message]).await?;
    make_output(&stdout, &stderr, success)
}

async fn git_push(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let remote = input["remote"].as_str().unwrap_or("origin");
    let mut cmd = Command::new("git");
    cmd.args(["push", remote]).current_dir(cwd);
    if let Some(branch) = input["branch"].as_str() {
        cmd.arg(branch);
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    make_output(&stdout, &stderr, output.status.success())
}

async fn git_pull(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let remote = input["remote"].as_str().unwrap_or("origin");
    let mut cmd = Command::new("git");
    cmd.args(["pull", remote]).current_dir(cwd);
    if let Some(branch) = input["branch"].as_str() {
        cmd.arg(branch);
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    make_output(&stdout, &stderr, output.status.success())
}

async fn git_branch(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    if input["create"].as_bool().unwrap_or(false) {
        let branch = input["branch"].as_str().ok_or_else(|| {
            ToolError::InvalidInput("branch create requires 'branch' name".into())
        })?;
        let (stdout, stderr, success) = run_git(cwd, &["checkout", "-b", branch]).await?;
        return make_output(&stdout, &stderr, success);
    }
    if input["delete"].as_bool().unwrap_or(false) {
        let branch = input["branch"].as_str().ok_or_else(|| {
            ToolError::InvalidInput("branch delete requires 'branch' name".into())
        })?;
        let (stdout, stderr, success) = run_git(cwd, &["branch", "-d", branch]).await?;
        return make_output(&stdout, &stderr, success);
    }

    // List branches.
    let mut cmd = Command::new("git");
    cmd.arg("branch").current_dir(cwd);
    if input["all"].as_bool().unwrap_or(false) {
        cmd.arg("-a");
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    make_output(&stdout, &stderr, output.status.success())
}

async fn git_checkout(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let branch = input["branch"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidInput("checkout requires 'branch'".into()))?;
    let files: Vec<String> = input["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut cmd = Command::new("git");
    cmd.arg("checkout").arg(branch).current_dir(cwd);
    if !files.is_empty() {
        cmd.arg("--").args(&files);
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    make_output(&stdout, &stderr, output.status.success())
}

async fn git_stash(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let sub = input["stash_action"].as_str().unwrap_or("push");
    let args: &[&str] = match sub {
        "push" => &["stash", "push"],
        "pop" => &["stash", "pop"],
        "list" => &["stash", "list"],
        "drop" => &["stash", "drop"],
        _ => {
            return Err(ToolError::InvalidInput(format!(
                "unknown stash_action: {sub}"
            )));
        }
    };
    let (stdout, stderr, success) = run_git(cwd, args).await?;
    make_output(&stdout, &stderr, success)
}

async fn git_clone(cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let url = input["url"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidInput("clone requires 'url'".into()))?;

    let mut cmd = Command::new("git");
    cmd.args(["clone", url]).current_dir(cwd);
    if let Some(branch) = input["branch"].as_str() {
        cmd.args(["-b", branch]);
    }
    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    make_output(&stdout, &stderr, output.status.success())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::GitTool;
    use crate::tool::{Tool, ToolContext};

    fn temp_repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // init repo with identity
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@helm"])
            .current_dir(&path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Helm Test"])
            .current_dir(&path)
            .output()
            .unwrap();

        (dir, path)
    }

    fn ctx(path: &Path) -> ToolContext {
        ToolContext::new(path.to_path_buf())
    }

    #[tokio::test]
    async fn git_status_on_clean_repo_happy_path() {
        let (_dir, path) = temp_repo();
        let tool = GitTool;
        let result = tool
            .execute(json!({"action": "status"}), &ctx(&path))
            .await
            .unwrap();
        assert!(result.success);
        assert!(
            result.content.contains("nothing to commit")
                || result.content.contains("No commits yet")
        );
    }

    #[tokio::test]
    async fn git_log_empty_repo_edge_case() {
        let (_dir, path) = temp_repo();
        let tool = GitTool;
        let result = tool
            .execute(json!({"action": "log"}), &ctx(&path))
            .await
            .unwrap();
        // Empty repo has no commits — git log returns non-zero exit
        assert!(!result.success || result.content.contains("(no output)"));
    }

    #[tokio::test]
    async fn git_add_and_commit_happy_path() {
        let (_dir, path) = temp_repo();
        // Create a file to commit.
        std::fs::write(path.join("hello.txt"), "hello\n").unwrap();

        let tool = GitTool;

        let add = tool
            .execute(
                json!({"action": "add", "files": ["hello.txt"]}),
                &ctx(&path),
            )
            .await
            .unwrap();
        assert!(add.success, "add failed: {}", add.content);

        let commit = tool
            .execute(
                json!({"action": "commit", "message": "initial commit"}),
                &ctx(&path),
            )
            .await
            .unwrap();
        assert!(commit.success, "commit failed: {}", commit.content);

        let log = tool
            .execute(json!({"action": "log"}), &ctx(&path))
            .await
            .unwrap();
        assert!(log.success);
        assert!(log.content.contains("initial commit"));
    }

    #[tokio::test]
    async fn git_diff_after_modify_happy_path() {
        let (_dir, path) = temp_repo();
        std::fs::write(path.join("f.txt"), "v1\n").unwrap();

        let tool = GitTool;
        tool.execute(json!({"action": "add", "files": ["f.txt"]}), &ctx(&path))
            .await
            .unwrap();
        tool.execute(json!({"action": "commit", "message": "v1"}), &ctx(&path))
            .await
            .unwrap();

        std::fs::write(path.join("f.txt"), "v2\n").unwrap();

        let diff = tool
            .execute(json!({"action": "diff"}), &ctx(&path))
            .await
            .unwrap();
        assert!(diff.success);
        assert!(diff.content.contains("v2") || diff.content.contains("-v1"));
    }

    #[tokio::test]
    async fn git_add_missing_files_error_path() {
        let (_dir, path) = temp_repo();
        let tool = GitTool;
        let err = tool
            .execute(json!({"action": "add"}), &ctx(&path))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("files"));
    }

    #[tokio::test]
    async fn git_commit_missing_message_error_path() {
        let (_dir, path) = temp_repo();
        let tool = GitTool;
        let err = tool
            .execute(json!({"action": "commit"}), &ctx(&path))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("message"));
    }

    #[tokio::test]
    async fn git_branch_list_happy_path() {
        let (_dir, path) = temp_repo();
        // Need at least one commit to have branches.
        std::fs::write(path.join("x.txt"), "x").unwrap();
        std::process::Command::new("git")
            .args(["add", "x.txt"])
            .current_dir(&path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "x"])
            .current_dir(&path)
            .output()
            .unwrap();

        let tool = GitTool;
        let result = tool
            .execute(json!({"action": "branch"}), &ctx(&path))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.content.contains("main"));
    }

    #[tokio::test]
    async fn git_status_short_flag_edge_case() {
        let (_dir, path) = temp_repo();
        std::fs::write(path.join("new.txt"), "new").unwrap();
        let tool = GitTool;
        let result = tool
            .execute(json!({"action": "status", "short": true}), &ctx(&path))
            .await
            .unwrap();
        assert!(result.success);
        // Short format uses ?? for untracked
        assert!(result.content.contains("??"));
    }
}
