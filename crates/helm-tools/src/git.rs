//! Git version-control tool for HELM.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use tokio::time;

use crate::{
    command::build_command_in_dir,
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

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
            "status" => git_status(ctx, &cwd, &input).await,
            "log" => git_log(ctx, &cwd, &input).await,
            "diff" => git_diff(ctx, &cwd, &input).await,
            "add" => git_add(ctx, &cwd, &input).await,
            "commit" => git_commit(ctx, &cwd, &input).await,
            "push" => git_push(ctx, &cwd, &input).await,
            "pull" => git_pull(ctx, &cwd, &input).await,
            "branch" => git_branch(ctx, &cwd, &input).await,
            "checkout" => git_checkout(ctx, &cwd, &input).await,
            "stash" => git_stash(ctx, &cwd, &input).await,
            "clone" => git_clone(ctx, &cwd, &input).await,
            _ => Err(ToolError::InvalidInput(format!("unknown action: {action}"))),
        }
    }

    fn allowed_in_diagnose(&self) -> bool {
        true
    }
}

/// Run a git command in the given directory and collect stdout/stderr/status.
async fn run_git(ctx: &ToolContext, cwd: &Path, args: &[String]) -> Result<ToolOutput, ToolError> {
    let started = Instant::now();
    let mut command = build_command_in_dir("git", args, ctx, cwd)?;
    command.kill_on_drop(true);
    let output = match time::timeout(ctx.timeout, command.output()).await {
        Ok(output) => output.map_err(|e| ToolError::Other(format!("failed to spawn git: {e}")))?,
        Err(_) => return Err(ToolError::Timeout),
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let mut metadata = Map::new();
    metadata.insert("program".to_owned(), json!("git"));
    metadata.insert("args".to_owned(), json!(args));
    metadata.insert(
        "duration_ms".to_owned(),
        json!(started.elapsed().as_millis()),
    );
    make_output(&stdout, &stderr, output.status.success(), ctx, metadata)
}

fn make_output(
    stdout: &str,
    stderr: &str,
    success: bool,
    ctx: &ToolContext,
    mut metadata: Map<String, Value>,
) -> Result<ToolOutput, ToolError> {
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
    let (content, truncated) = truncate_string(&content, ctx.max_output_bytes);
    metadata.insert("truncated".to_owned(), json!(truncated));
    Ok(ToolOutput {
        content,
        success,
        metadata,
    })
}

fn git_args(base: &[&str]) -> Vec<String> {
    base.iter().map(|s| (*s).to_owned()).collect()
}

fn truncate_string(input: &str, max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (input.to_owned(), false);
    }
    let mut end = max_bytes;
    while !input.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    (format!("{}\n[output truncated]", &input[..end]), true)
}

async fn git_status(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let mut args = git_args(&["status"]);
    if input["short"].as_bool().unwrap_or(false) {
        args.push("--short".to_owned());
    }
    run_git(ctx, cwd, &args).await
}

async fn git_log(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let limit = input["limit"].as_u64().unwrap_or(20);
    let mut args = git_args(&["log", "--oneline", "--decorate"]);
    args.push(format!("-{limit}"));
    if input["all"].as_bool().unwrap_or(false) {
        args.push("--all".to_owned());
    }
    run_git(ctx, cwd, &args).await
}

async fn git_diff(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let staged = input["staged"].as_bool().unwrap_or(false);
    let files: Vec<String> = input["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut args = git_args(&["diff"]);
    if staged {
        args.push("--cached".to_owned());
    }
    if !files.is_empty() {
        args.push("--".to_owned());
        args.extend(files);
    }
    run_git(ctx, cwd, &args).await
}

async fn git_add(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
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

    let mut args = git_args(&["add", "--"]);
    args.extend(files);
    run_git(ctx, cwd, &args).await
}

async fn git_commit(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let message = input["message"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidInput("commit requires 'message'".into()))?;

    run_git(ctx, cwd, &git_args(&["commit", "-m", message])).await
}

async fn git_push(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let remote = input["remote"].as_str().unwrap_or("origin");
    let mut args = git_args(&["push", remote]);
    if let Some(branch) = input["branch"].as_str() {
        args.push(branch.to_owned());
    }
    run_git(ctx, cwd, &args).await
}

async fn git_pull(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let remote = input["remote"].as_str().unwrap_or("origin");
    let mut args = git_args(&["pull", remote]);
    if let Some(branch) = input["branch"].as_str() {
        args.push(branch.to_owned());
    }
    run_git(ctx, cwd, &args).await
}

async fn git_branch(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    if input["create"].as_bool().unwrap_or(false) {
        let branch = input["branch"].as_str().ok_or_else(|| {
            ToolError::InvalidInput("branch create requires 'branch' name".into())
        })?;
        return run_git(ctx, cwd, &git_args(&["checkout", "-b", branch])).await;
    }
    if input["delete"].as_bool().unwrap_or(false) {
        let branch = input["branch"].as_str().ok_or_else(|| {
            ToolError::InvalidInput("branch delete requires 'branch' name".into())
        })?;
        return run_git(ctx, cwd, &git_args(&["branch", "-d", branch])).await;
    }

    // List branches.
    let mut args = git_args(&["branch"]);
    if input["all"].as_bool().unwrap_or(false) {
        args.push("-a".to_owned());
    }
    run_git(ctx, cwd, &args).await
}

async fn git_checkout(
    ctx: &ToolContext,
    cwd: &Path,
    input: &Value,
) -> Result<ToolOutput, ToolError> {
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

    let mut args = git_args(&["checkout", branch]);
    if !files.is_empty() {
        args.push("--".to_owned());
        args.extend(files);
    }
    run_git(ctx, cwd, &args).await
}

async fn git_stash(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let sub = input["stash_action"].as_str().unwrap_or("push");
    let args = match sub {
        "push" => git_args(&["stash", "push"]),
        "pop" => git_args(&["stash", "pop"]),
        "list" => git_args(&["stash", "list"]),
        "drop" => git_args(&["stash", "drop"]),
        _ => {
            return Err(ToolError::InvalidInput(format!(
                "unknown stash_action: {sub}"
            )));
        }
    };
    run_git(ctx, cwd, &args).await
}

async fn git_clone(ctx: &ToolContext, cwd: &Path, input: &Value) -> Result<ToolOutput, ToolError> {
    let url = input["url"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidInput("clone requires 'url'".into()))?;

    let mut args = git_args(&["clone"]);
    if let Some(branch) = input["branch"].as_str() {
        args.extend(git_args(&["-b", branch]));
    }
    args.push(url.to_owned());
    run_git(ctx, cwd, &args).await
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

    #[tokio::test]
    async fn git_output_is_truncated_by_tool_context_edge_case() {
        let (_dir, path) = temp_repo();
        std::fs::write(path.join("f.txt"), "v1\n").unwrap();
        let tool = GitTool;
        tool.execute(json!({"action": "add", "files": ["f.txt"]}), &ctx(&path))
            .await
            .unwrap();
        tool.execute(json!({"action": "commit", "message": "v1"}), &ctx(&path))
            .await
            .unwrap();
        std::fs::write(path.join("f.txt"), "v2\nv3\nv4\nv5\n").unwrap();

        let mut context = ctx(&path);
        context.max_output_bytes = 16;
        let output = tool
            .execute(json!({"action": "diff"}), &context)
            .await
            .unwrap();

        assert!(output.success);
        assert!(output.content.contains("output truncated"));
        assert_eq!(output.metadata.get("truncated"), Some(&json!(true)));
    }
}
