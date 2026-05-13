//! Process execution tool with direct exec and shell composition modes.

use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::timeout,
};

use crate::{
    command::build_command_in_dir,
    fs_read::validate_write_path,
    tool::{Tool, ToolContext, ToolError, ToolOutput},
    validator::AllowlistConfig,
};

/// Tool that runs a command directly or through a shell when composition is needed.
#[derive(Debug, Default)]
pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn description(&self) -> &'static str {
        "Execute a non-interactive command in exec or shell mode with optional stdin and atomic output redirection."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["exec", "shell"],
                    "description": "exec runs command with args literally; shell runs command through bash -c or sh -c. Default exec."
                },
                "command": {
                    "type": "string",
                    "description": "For exec, the binary to run. For shell, the full command line."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments for exec mode. Ignored when mode is shell."
                },
                "cwd": { "type": "string" },
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                },
                "stdin": { "type": "string" },
                "redirect_stdout_to": { "type": "string" },
                "redirect_stderr_to": { "type": "string" },
                "stdout_append": { "type": "boolean" },
                "stderr_append": { "type": "boolean" }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let parsed = ShellInput::parse(input, ctx)?;
        let allowlist = AllowlistConfig::load()?;
        // Diagnose-mode runtime gate: reject shell expansion, pipes, and output redirection.
        if ctx.diagnose_mode {
            if matches!(parsed.mode, ShellMode::Shell) {
                return Err(ToolError::InvalidInput(
                    "shell mode (pipes/redirects) not allowed in diagnose mode".into(),
                ));
            }
            if parsed.redirect_stdout_to.is_some() || parsed.redirect_stderr_to.is_some() {
                return Err(ToolError::InvalidInput(
                    "output redirection not allowed in diagnose mode".into(),
                ));
            }
        }
        let command_line = match parsed.mode {
            ShellMode::Exec => std::iter::once(parsed.command.as_str())
                .chain(parsed.args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
            ShellMode::Shell => parsed.command.clone(),
        };
        if !allowlist.is_shell_allowed(&command_line) {
            return Err(ToolError::InvalidInput(format!(
                "shell command not permitted by ~/.helm/allowlist.toml: {command_line}"
            )));
        }
        let stdout_redirect = parsed
            .redirect_stdout_to
            .as_ref()
            .map(|path| validate_write_path(path, ctx, false))
            .transpose()?;
        let stderr_redirect = parsed
            .redirect_stderr_to
            .as_ref()
            .map(|path| validate_write_path(path, ctx, false))
            .transpose()?;
        let started = Instant::now();

        let mut command = match parsed.mode {
            ShellMode::Exec => {
                build_command_in_dir(&parsed.command, &parsed.args, ctx, &parsed.cwd)?
            }
            ShellMode::Shell => {
                let shell = select_shell(&parsed.env)?;
                build_command_in_dir(
                    &shell.to_string_lossy(),
                    &[String::from("-c"), parsed.command.clone()],
                    ctx,
                    &parsed.cwd,
                )?
            }
        };

        command
            .stdin(if parsed.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &parsed.env {
            command.env(key, value);
        }

        let mut child = command.spawn()?;
        let stdin_task = if let Some(stdin) = parsed.stdin.clone() {
            let mut child_stdin = child
                .stdin
                .take()
                .ok_or_else(|| ToolError::Other("failed to open stdin".to_owned()))?;
            Some(tokio::spawn(async move {
                child_stdin.write_all(stdin.as_bytes()).await?;
                child_stdin.shutdown().await?;
                Ok::<(), ToolError>(())
            }))
        } else {
            None
        };
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::Other("failed to capture stdout".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::Other("failed to capture stderr".to_owned()))?;
        let stdout_task = tokio::spawn(read_pipe(stdout));
        let stderr_task = tokio::spawn(read_pipe(stderr));

        let status = match timeout(ctx.timeout, child.wait()).await {
            Ok(result) => result?,
            Err(_) => {
                child.kill().await?;
                let _ = child.wait().await;
                let stdout_bytes = stdout_task
                    .await
                    .map_err(|error| ToolError::Other(format!("stdout task failed: {error}")))??;
                let stderr_bytes = stderr_task
                    .await
                    .map_err(|error| ToolError::Other(format!("stderr task failed: {error}")))??;
                if let Some(task) = stdin_task {
                    task.await.map_err(|error| {
                        ToolError::Other(format!("stdin task failed: {error}"))
                    })??;
                }
                let stdout_text = String::from_utf8_lossy(&stdout_bytes);
                let stderr_text = String::from_utf8_lossy(&stderr_bytes);
                let timeout_secs = ctx.timeout.as_secs();
                let full = format!(
                    "STDOUT:\n{stdout_text}\nSTDERR:\n{stderr_text}\n[timed out after {timeout_secs}s; process killed]"
                );
                let (content, truncated, omitted) = truncate_string(&full, ctx.max_output_bytes);
                let mut metadata = Map::new();
                metadata.insert("mode".to_owned(), json!(parsed.mode.as_str()));
                metadata.insert("exit_code".to_owned(), Value::Null);
                metadata.insert("stdout_bytes".to_owned(), json!(stdout_bytes.len()));
                metadata.insert("stderr_bytes".to_owned(), json!(stderr_bytes.len()));
                metadata.insert("stdin_bytes".to_owned(), json!(parsed.stdin_bytes()));
                metadata.insert(
                    "duration_ms".to_owned(),
                    json!(duration_ms(started.elapsed())),
                );
                metadata.insert("timed_out".to_owned(), Value::Bool(true));
                metadata.insert("truncated".to_owned(), Value::Bool(truncated));
                if truncated {
                    metadata.insert("omitted_bytes".to_owned(), json!(omitted));
                }
                return Ok(ToolOutput {
                    content,
                    success: false,
                    metadata,
                });
            }
        };

        let stdout_bytes = stdout_task
            .await
            .map_err(|error| ToolError::Other(format!("stdout task failed: {error}")))??;
        let stderr_bytes = stderr_task
            .await
            .map_err(|error| ToolError::Other(format!("stderr task failed: {error}")))??;
        if let Some(task) = stdin_task {
            task.await
                .map_err(|error| ToolError::Other(format!("stdin task failed: {error}")))??;
        }
        let (bytes_redirected_stdout, bytes_redirected_stderr) =
            match (stdout_redirect.as_ref(), stderr_redirect.as_ref()) {
                (Some(stdout_path), Some(stderr_path)) if stdout_path == stderr_path => {
                    let mut combined = stdout_bytes.clone();
                    combined.extend_from_slice(&stderr_bytes);
                    atomic_redirect(stdout_path, &combined, parsed.stdout_append)?;
                    (stdout_bytes.len(), stderr_bytes.len())
                }
                (stdout_path, stderr_path) => {
                    let stdout_count = if let Some(path) = stdout_path {
                        atomic_redirect(path, &stdout_bytes, parsed.stdout_append)?;
                        stdout_bytes.len()
                    } else {
                        0
                    };
                    let stderr_count = if let Some(path) = stderr_path {
                        atomic_redirect(path, &stderr_bytes, parsed.stderr_append)?;
                        stderr_bytes.len()
                    } else {
                        0
                    };
                    (stdout_count, stderr_count)
                }
            };
        let stdout_text = String::from_utf8_lossy(&stdout_bytes);
        let stderr_text = String::from_utf8_lossy(&stderr_bytes);
        let exit_code = status.code();
        let exit_label = exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_owned());
        let stdout_section = match stdout_redirect.as_ref() {
            Some(path) => format!("[stdout redirected to {}]", path.display()),
            None => format!("STDOUT:\n{stdout_text}"),
        };
        let stderr_section = match stderr_redirect.as_ref() {
            Some(path) => format!("[stderr redirected to {}]", path.display()),
            None => format!("STDERR:\n{stderr_text}"),
        };
        let full = format!("{stdout_section}\n{stderr_section}\n[exit code: {exit_label}]");
        let (content, truncated, omitted) = truncate_string(&full, ctx.max_output_bytes);

        let mut metadata = Map::new();
        metadata.insert("mode".to_owned(), json!(parsed.mode.as_str()));
        metadata.insert(
            "exit_code".to_owned(),
            exit_code.map_or(Value::Null, Value::from),
        );
        metadata.insert("stdout_bytes".to_owned(), json!(stdout_bytes.len()));
        metadata.insert("stderr_bytes".to_owned(), json!(stderr_bytes.len()));
        metadata.insert(
            "stdout_redirected_to".to_owned(),
            stdout_redirect
                .as_ref()
                .map(|path| json!(path.to_string_lossy().to_string()))
                .unwrap_or(Value::Null),
        );
        metadata.insert(
            "stderr_redirected_to".to_owned(),
            stderr_redirect
                .as_ref()
                .map(|path| json!(path.to_string_lossy().to_string()))
                .unwrap_or(Value::Null),
        );
        metadata.insert("stdin_bytes".to_owned(), json!(parsed.stdin_bytes()));
        metadata.insert(
            "bytes_redirected_stdout".to_owned(),
            json!(bytes_redirected_stdout),
        );
        metadata.insert(
            "bytes_redirected_stderr".to_owned(),
            json!(bytes_redirected_stderr),
        );
        metadata.insert(
            "duration_ms".to_owned(),
            json!(duration_ms(started.elapsed())),
        );
        metadata.insert("truncated".to_owned(), Value::Bool(truncated));
        if truncated {
            metadata.insert("omitted_bytes".to_owned(), json!(omitted));
        }

        Ok(ToolOutput {
            content,
            success: status.success(),
            metadata,
        })
    }

    fn allowed_in_diagnose(&self) -> bool {
        // Shell is visible in diagnose mode so the model can inspect system
        // state, but exec-mode-only is enforced at runtime in execute().
        true
    }

    fn all_write_ops_gated_in_diagnose(&self) -> bool {
        true // shell mode and output redirection are runtime-gated via ctx.diagnose_mode
    }
}

#[derive(Debug)]
struct ShellInput {
    mode: ShellMode,
    command: String,
    args: Vec<String>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    stdin: Option<String>,
    redirect_stdout_to: Option<PathBuf>,
    redirect_stderr_to: Option<PathBuf>,
    stdout_append: bool,
    stderr_append: bool,
}

impl ShellInput {
    fn parse(input: Value, ctx: &ToolContext) -> Result<Self, ToolError> {
        let object = input
            .as_object()
            .ok_or_else(|| ToolError::InvalidInput("shell input must be an object".to_owned()))?;
        let command = object
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("command is required".to_owned()))?
            .to_owned();
        if command.is_empty() {
            return Err(ToolError::InvalidInput(
                "command cannot be empty".to_owned(),
            ));
        }
        let mode = match object.get("mode").and_then(Value::as_str) {
            Some("exec") | None => ShellMode::Exec,
            Some("shell") => ShellMode::Shell,
            Some(other) => {
                return Err(ToolError::InvalidInput(format!(
                    "mode must be \"exec\" or \"shell\", got {other:?}"
                )));
            }
        };

        let args = match object.get("args") {
            Some(Value::Array(values)) => values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        ToolError::InvalidInput("args must contain strings".to_owned())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(ToolError::InvalidInput("args must be an array".to_owned())),
            None => Vec::new(),
        };
        let cwd = match object.get("cwd") {
            Some(Value::String(path)) => {
                let path = PathBuf::from(path);
                if path.is_absolute() {
                    path
                } else {
                    ctx.working_dir.join(path)
                }
            }
            Some(_) => return Err(ToolError::InvalidInput("cwd must be a string".to_owned())),
            None => ctx.working_dir.clone(),
        };
        let env = match object.get("env") {
            Some(Value::Object(values)) => values
                .iter()
                .map(|(key, value)| {
                    value
                        .as_str()
                        .map(|text| (key.clone(), text.to_owned()))
                        .ok_or_else(|| {
                            ToolError::InvalidInput("env values must be strings".to_owned())
                        })
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(ToolError::InvalidInput("env must be an object".to_owned())),
            None => Vec::new(),
        };
        let stdin = match object.get("stdin") {
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => return Err(ToolError::InvalidInput("stdin must be a string".to_owned())),
            None => None,
        };
        let redirect_stdout_to =
            optional_path(object.get("redirect_stdout_to"), "redirect_stdout_to")?;
        let redirect_stderr_to =
            optional_path(object.get("redirect_stderr_to"), "redirect_stderr_to")?;
        let stdout_append = optional_bool(object.get("stdout_append"), "stdout_append")?;
        let stderr_append = optional_bool(object.get("stderr_append"), "stderr_append")?;

        Ok(Self {
            mode,
            command,
            args,
            cwd,
            env,
            stdin,
            redirect_stdout_to,
            redirect_stderr_to,
            stdout_append,
            stderr_append,
        })
    }

    fn stdin_bytes(&self) -> usize {
        self.stdin.as_ref().map_or(0, String::len)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellMode {
    Exec,
    Shell,
}

impl ShellMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Exec => "exec",
            Self::Shell => "shell",
        }
    }
}

fn optional_path(value: Option<&Value>, name: &str) -> Result<Option<PathBuf>, ToolError> {
    match value {
        Some(Value::String(path)) => Ok(Some(PathBuf::from(path))),
        Some(_) => Err(ToolError::InvalidInput(format!("{name} must be a string"))),
        None => Ok(None),
    }
}

fn optional_bool(value: Option<&Value>, name: &str) -> Result<bool, ToolError> {
    match value {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(ToolError::InvalidInput(format!("{name} must be a boolean"))),
        None => Ok(false),
    }
}

fn select_shell(env_overrides: &[(String, String)]) -> Result<PathBuf, ToolError> {
    find_in_path("bash", env_overrides)
        .or_else(|| find_in_path("sh", env_overrides))
        .ok_or_else(|| ToolError::Other("no shell available (tried bash, sh)".to_owned()))
}

fn find_in_path(binary: &str, env_overrides: &[(String, String)]) -> Option<PathBuf> {
    let path = env_overrides
        .iter()
        .rev()
        .find_map(|(key, value)| (key == "PATH").then(|| value.clone()))
        .map(Into::into)
        .or_else(|| env::var_os("PATH"))?;
    env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
}

fn atomic_redirect(path: &Path, bytes: &[u8], append: bool) -> Result<(), ToolError> {
    let parent = path
        .parent()
        .ok_or_else(|| ToolError::InvalidInput("path must have a parent".to_owned()))?;
    let mut temp = NamedTempFile::new_in(parent)?;
    if append && path.exists() {
        let existing = fs::read(path)?;
        temp.write_all(&existing)?;
    }
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| ToolError::IoError(error.error))?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

async fn read_pipe<R>(mut reader: R) -> Result<Vec<u8>, ToolError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

fn duration_ms(duration: Duration) -> u128 {
    duration.as_millis()
}

fn truncate_string(input: &str, max_bytes: usize) -> (String, bool, usize) {
    let bytes = input.as_bytes();
    if bytes.len() <= max_bytes {
        return (input.to_owned(), false, 0);
    }
    let omitted = bytes.len().saturating_sub(max_bytes);
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let mut output = input[..end].to_owned();
    output.push_str(&format!("\n[output truncated; {omitted} bytes omitted]"));
    (output, true, omitted)
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::tool::{Tool, ToolContext, ToolError};

    use super::{ShellInput, ShellTool, truncate_string};

    fn ctx() -> (tempfile::TempDir, ToolContext) {
        let dir = tempdir().unwrap();
        let mut ctx = ToolContext::new(dir.path().to_path_buf());
        ctx.timeout = Duration::from_secs(2);
        (dir, ctx)
    }

    #[tokio::test]
    async fn exit_zero_happy_path() {
        let (_dir, ctx) = ctx();
        let output = ShellTool
            .execute(json!({"command": "printf", "args": ["hi"]}), &ctx)
            .await
            .unwrap();

        assert!(output.success);
        assert!(output.content.contains("hi"));
    }

    #[tokio::test]
    async fn exit_nonzero_returns_output_error_path() {
        let (_dir, ctx) = ctx();
        let output = ShellTool
            .execute(json!({"command": "false"}), &ctx)
            .await
            .unwrap();

        assert!(!output.success);
        assert!(output.content.contains("[exit code: 1]"));
    }

    #[tokio::test]
    async fn timeout_kills_process_edge_case() {
        let (_dir, mut ctx) = ctx();
        ctx.timeout = Duration::from_millis(50);
        let output = ShellTool
            .execute(json!({"command": "sleep", "args": ["2"]}), &ctx)
            .await
            .unwrap();

        assert!(!output.success);
        assert!(output.content.contains("timed out after"));
        assert_eq!(output.metadata.get("timed_out"), Some(&json!(true)));
    }

    #[tokio::test]
    async fn output_is_truncated() {
        let (_dir, mut ctx) = ctx();
        ctx.max_output_bytes = 12;
        let output = ShellTool
            .execute(
                json!({"command": "printf", "args": ["abcdefghijklmnopqrstuvwxyz"]}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(output.metadata.get("truncated"), Some(&json!(true)));
        assert!(output.content.contains("output truncated"));
    }

    #[tokio::test]
    async fn command_not_found_errors() {
        let (_dir, ctx) = ctx();
        let error = ShellTool
            .execute(json!({"command": "helm-command-that-does-not-exist"}), &ctx)
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("No such file") || error.to_string().contains("not found")
        );
    }

    #[tokio::test]
    async fn sets_cwd_correctly() {
        let (dir, ctx) = ctx();
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).unwrap();
        let output = ShellTool
            .execute(json!({"command": "pwd", "cwd": "nested"}), &ctx)
            .await
            .unwrap();

        assert!(output.content.contains(nested.to_string_lossy().as_ref()));
    }

    #[tokio::test]
    async fn env_override_works() {
        let (_dir, ctx) = ctx();
        let output = ShellTool
            .execute(json!({"command": "printenv", "args": ["HELM_TEST_ENV"], "env": {"HELM_TEST_ENV": "ok"}}), &ctx)
            .await
            .unwrap();

        assert!(output.content.contains("ok"));
    }

    #[tokio::test]
    async fn shell_mode_expands_command_substitution_happy_path() {
        let (_dir, ctx) = ctx();
        let output = ShellTool
            .execute(
                json!({"mode": "shell", "command": "echo \"$(date)\""}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(output.success);
        assert!(!output.content.contains("$(date)"));
        assert_eq!(output.metadata.get("mode"), Some(&json!("shell")));
    }

    #[tokio::test]
    async fn shell_mode_supports_pipe_happy_path() {
        let (_dir, ctx) = ctx();
        let output = ShellTool
            .execute(
                json!({"mode": "shell", "command": "echo hello | tr a-z A-Z"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(output.content.contains("HELLO"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_mode_falls_back_to_sh_when_bash_not_on_path_edge_case() {
        let (dir, ctx) = ctx();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        symlink("/bin/sh", bin.join("sh")).unwrap();

        let output = ShellTool
            .execute(
                json!({
                    "mode": "shell",
                    "command": "echo fallback-ok",
                    "env": {"PATH": bin.to_string_lossy()}
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(output.content.contains("fallback-ok"));
    }

    #[tokio::test]
    async fn exec_mode_keeps_shell_syntax_literal_happy_path() {
        let (_dir, ctx) = ctx();
        let output = ShellTool
            .execute(
                json!({"mode": "exec", "command": "echo", "args": ["$(date)"]}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(output.content.contains("$(date)"));
    }

    #[tokio::test]
    async fn redirect_stdout_writes_named_file_happy_path() {
        let (dir, ctx) = ctx();
        let path = dir.path().join("out.txt");
        let output = ShellTool
            .execute(
                json!({"command": "printf", "args": ["hello"], "redirect_stdout_to": path}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        assert!(output.content.contains("[stdout redirected to"));
        assert_eq!(
            output.metadata.get("bytes_redirected_stdout"),
            Some(&json!(5))
        );
    }

    #[tokio::test]
    async fn denied_redirect_errors_before_command_runs_error_path() {
        let (dir, ctx) = ctx();
        let sentinel = dir.path().join("sentinel");
        let error = ShellTool
            .execute(
                json!({
                    "command": "touch",
                    "args": [sentinel.to_string_lossy()],
                    "redirect_stdout_to": "/etc/shadow"
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(matches!(error, ToolError::PathDenied(_)));
        assert!(!sentinel.exists());
    }

    #[tokio::test]
    async fn redirect_stdout_append_preserves_existing_content_edge_case() {
        let (dir, ctx) = ctx();
        let path = dir.path().join("append.txt");
        fs::write(&path, "old\n").unwrap();
        ShellTool
            .execute(
                json!({
                    "command": "printf",
                    "args": ["new"],
                    "redirect_stdout_to": path,
                    "stdout_append": true
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "old\nnew");
    }

    #[tokio::test]
    async fn stdin_is_piped_happy_path() {
        let (_dir, ctx) = ctx();
        let output = ShellTool
            .execute(json!({"command": "cat", "stdin": "hello"}), &ctx)
            .await
            .unwrap();

        assert!(output.content.contains("hello"));
        assert_eq!(output.metadata.get("stdin_bytes"), Some(&json!(5)));
    }

    #[tokio::test]
    async fn redirects_both_streams_reports_paths_and_exit_code_happy_path() {
        let (dir, ctx) = ctx();
        let stdout_path = dir.path().join("stdout.txt");
        let stderr_path = dir.path().join("stderr.txt");
        let output = ShellTool
            .execute(
                json!({
                    "mode": "shell",
                    "command": "printf out; printf err >&2",
                    "redirect_stdout_to": stdout_path,
                    "redirect_stderr_to": stderr_path
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&stdout_path).unwrap(), "out");
        assert_eq!(fs::read_to_string(&stderr_path).unwrap(), "err");
        assert!(output.content.contains("[stdout redirected to"));
        assert!(output.content.contains("[stderr redirected to"));
        assert!(output.content.contains("[exit code: 0]"));
        assert_eq!(
            output.metadata.get("bytes_redirected_stderr"),
            Some(&json!(3))
        );
    }

    #[tokio::test]
    async fn redirects_both_streams_to_same_path_without_overwriting_edge_case() {
        let (dir, ctx) = ctx();
        let path = dir.path().join("combined.txt");
        ShellTool
            .execute(
                json!({
                    "mode": "shell",
                    "command": "printf out; printf err >&2",
                    "redirect_stdout_to": path,
                    "redirect_stderr_to": path
                }),
                &ctx,
            )
            .await
            .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("out"));
        assert!(content.contains("err"));
    }

    #[test]
    fn parse_rejects_empty_command_error_path() {
        let (_dir, ctx) = ctx();
        let error = ShellInput::parse(json!({"command": ""}), &ctx).unwrap_err();

        assert!(error.to_string().contains("empty"));
    }

    #[test]
    fn parse_rejects_missing_command_error_path() {
        let (_dir, ctx) = ctx();
        let error = ShellInput::parse(json!({}), &ctx).unwrap_err();

        assert!(error.to_string().contains("command is required"));
    }

    #[test]
    fn parse_rejects_non_array_args_error_path() {
        let (_dir, ctx) = ctx();
        let error = ShellInput::parse(json!({"command": "echo", "args": "no"}), &ctx).unwrap_err();

        assert!(error.to_string().contains("args must be an array"));
    }

    #[test]
    fn parse_rejects_unknown_mode_error_path() {
        let (_dir, ctx) = ctx();
        let error =
            ShellInput::parse(json!({"command": "echo", "mode": "magic"}), &ctx).unwrap_err();

        assert!(error.to_string().contains("exec"));
        assert!(error.to_string().contains("shell"));
    }

    #[test]
    fn truncate_preserves_utf8_boundary_edge_case() {
        let (text, truncated, _) = truncate_string("ééé", 1);

        assert!(truncated);
        assert!(text.contains("output truncated"));
    }
}
