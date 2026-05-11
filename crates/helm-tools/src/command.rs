//! Small command runner shared by typed Linux tools.

use std::{
    path::{Path, PathBuf},
    time::Duration,
    time::Instant,
};

use serde_json::{Map, Value, json};
use tokio::{process::Command, time};

use crate::tool::{SandboxPolicy, ToolContext, ToolError, ToolOutput};

const OPTIONAL_RO_BINDS: &[&str] = &[
    "/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/opt", "/etc", "/run", "/nix",
];

pub async fn run_command(
    program: &str,
    args: &[String],
    ctx: &ToolContext,
) -> Result<ToolOutput, ToolError> {
    run_command_in_dir_with_timeout(program, args, ctx, &ctx.working_dir, ctx.timeout).await
}

pub async fn run_command_with_timeout(
    program: &str,
    args: &[String],
    ctx: &ToolContext,
    timeout_duration: Duration,
) -> Result<ToolOutput, ToolError> {
    run_command_in_dir_with_timeout(program, args, ctx, &ctx.working_dir, timeout_duration).await
}

pub async fn run_command_in_dir_with_timeout(
    program: &str,
    args: &[String],
    ctx: &ToolContext,
    cwd: &Path,
    timeout_duration: Duration,
) -> Result<ToolOutput, ToolError> {
    let started = Instant::now();
    let mut command = build_command_in_dir(program, args, ctx, cwd)?;
    command.kill_on_drop(true);
    let output = match time::timeout(timeout_duration, command.output()).await {
        Ok(output) => output?,
        Err(_) => return Err(ToolError::Timeout),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);
    let content = format!("STDOUT:\n{stdout}\nSTDERR:\n{stderr}\n[exit code: {exit_code}]");
    let mut metadata = Map::new();
    metadata.insert("program".to_owned(), json!(program));
    metadata.insert("args".to_owned(), json!(args));
    metadata.insert("exit_code".to_owned(), json!(exit_code));
    metadata.insert(
        "duration_ms".to_owned(),
        json!(started.elapsed().as_millis()),
    );
    Ok(ToolOutput {
        content: truncate(content, ctx.max_output_bytes),
        success: output.status.success(),
        metadata,
    })
}

#[allow(dead_code)]
pub fn build_command(
    program: &str,
    args: &[String],
    ctx: &ToolContext,
) -> Result<Command, ToolError> {
    build_command_in_dir(program, args, ctx, &ctx.working_dir)
}

pub fn build_command_in_dir(
    program: &str,
    args: &[String],
    ctx: &ToolContext,
    cwd: &Path,
) -> Result<Command, ToolError> {
    if let Some(policy) = &ctx.sandbox {
        build_bwrap_command(policy, program, args, cwd)
    } else {
        let mut command = Command::new(program);
        command.args(args).current_dir(cwd);
        Ok(command)
    }
}

pub fn require_confirm(input: &Value, action: &str) -> Result<(), ToolError> {
    if input
        .get("confirm")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(ToolError::InvalidInput(format!(
            "{action} is destructive and requires confirm=true"
        )))
    }
}

pub fn str_field(input: &Value, field: &str) -> Result<String, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ToolError::InvalidInput(format!("{field} is required")))
}

pub fn optional_str(input: &Value, field: &str) -> Option<String> {
    input.get(field).and_then(Value::as_str).map(str::to_owned)
}

pub fn u64_field(input: &Value, field: &str, default: u64) -> u64 {
    input.get(field).and_then(Value::as_u64).unwrap_or(default)
}

pub fn path_or_default(input: &Value, field: &str, default: &Path) -> String {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| default.display().to_string())
}

fn truncate(content: String, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content;
    }
    let mut end = max_bytes;
    while !content.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}\n[output truncated]", &content[..end])
}

fn build_bwrap_command(
    policy: &SandboxPolicy,
    program: &str,
    args: &[String],
    working_dir: &Path,
) -> Result<Command, ToolError> {
    let root_dir = policy
        .root_dir
        .canonicalize()
        .map_err(|error| ToolError::Other(format!("sandbox root not accessible: {error}")))?;
    let working_dir = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());
    let sandbox_cwd = sandbox_working_dir(&root_dir, &working_dir);
    let mut command = Command::new(&policy.bwrap_program);
    command
        .arg("--die-with-parent")
        .arg("--new-session")
        .arg("--unshare-all")
        .arg("--share-net")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg("/tmp")
        .arg("--bind")
        .arg(&root_dir)
        .arg(&root_dir)
        .arg("--setenv")
        .arg("HOME")
        .arg(&root_dir)
        .arg("--setenv")
        .arg("TMPDIR")
        .arg("/tmp")
        .arg("--chdir")
        .arg(&sandbox_cwd);

    for path in OPTIONAL_RO_BINDS.iter().map(Path::new) {
        if path.exists() {
            command.arg("--ro-bind").arg(path).arg(path);
        }
    }

    command.arg(program);
    command.args(args);
    Ok(command)
}

fn sandbox_working_dir(root_dir: &Path, working_dir: &Path) -> PathBuf {
    if working_dir.starts_with(root_dir) {
        working_dir.to_path_buf()
    } else {
        root_dir.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use crate::tool::{SandboxPolicy, ToolContext};

    use super::{build_command, build_command_in_dir, sandbox_working_dir};

    #[test]
    fn sandbox_working_dir_falls_back_to_root_when_outside_root() {
        let root = PathBuf::from("/tmp/helm-sandbox");
        let outside = PathBuf::from("/work/repo");

        assert_eq!(sandbox_working_dir(&root, &outside), root);
    }

    #[test]
    fn build_command_wraps_program_with_bwrap() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let ctx = ToolContext::new(root.clone()).with_sandbox(SandboxPolicy {
            root_dir: root.clone(),
            bwrap_program: PathBuf::from("/usr/bin/bwrap"),
        });

        let command = build_command("printf", &[String::from("ok")], &ctx).unwrap();
        let argv = command.as_std().get_args().collect::<Vec<_>>();

        assert_eq!(
            command.as_std().get_program().to_string_lossy(),
            "/usr/bin/bwrap"
        );
        assert!(argv.iter().any(|arg| arg.to_string_lossy() == "--bind"));
        assert!(argv.iter().any(|arg| arg == &root.as_os_str()));
        assert!(argv.iter().any(|arg| arg.to_string_lossy() == "printf"));
    }

    #[test]
    fn build_command_in_dir_uses_requested_cwd_inside_sandbox() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("root");
        let nested = root.join("repo");
        std::fs::create_dir_all(&nested).unwrap();
        let ctx = ToolContext::new(root.clone()).with_sandbox(SandboxPolicy {
            root_dir: root.clone(),
            bwrap_program: PathBuf::from("/usr/bin/bwrap"),
        });

        let command = build_command_in_dir("pwd", &[], &ctx, &nested).unwrap();
        let argv = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        let idx = argv.iter().position(|arg| arg == "--chdir").unwrap();
        assert_eq!(argv[idx + 1], nested.to_string_lossy());
    }
}
