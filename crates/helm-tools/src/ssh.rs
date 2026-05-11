//! SSH/SCP/RSYNC tools used by the v1.5 remote-execution path.
//!
//! All three tools resolve a `remote` name against `~/.helm/remotes.toml` to
//! build the host/port/user argv that the underlying ssh client invokes.
//! Inline `host`/`user`/`port` keys override the registry. Output is captured
//! and returned to the agent; large outputs are truncated per ToolContext.

use std::{
    fs,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

use crate::tool::{Tool, ToolContext, ToolError, ToolOutput};

#[derive(Debug, Default, Deserialize)]
struct RemoteFile {
    #[serde(default)]
    remotes: Vec<RemoteRegistryEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteRegistryEntry {
    name: String,
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    ssh_opts: Option<String>,
}

fn default_port() -> u16 {
    22
}

fn load_remote(name: &str) -> Result<Option<RemoteRegistryEntry>, ToolError> {
    let path: PathBuf = match dirs::home_dir() {
        Some(home) => home.join(".helm").join("remotes.toml"),
        None => return Ok(None),
    };
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ToolError::Other(error.to_string())),
    };
    let parsed: RemoteFile =
        toml::from_str(&text).map_err(|e| ToolError::Other(format!("remotes.toml: {e}")))?;
    Ok(parsed.remotes.into_iter().find(|r| r.name == name))
}

#[derive(Debug, Clone)]
struct RemoteEndpoint {
    host: String,
    port: u16,
    user: Option<String>,
    ssh_opts: Option<String>,
}

impl RemoteEndpoint {
    fn resolve(object: &Map<String, Value>) -> Result<Self, ToolError> {
        if let Some(name) = object.get("remote").and_then(Value::as_str) {
            let entry = load_remote(name)?.ok_or_else(|| {
                ToolError::InvalidInput(format!(
                    "remote '{name}' not found in ~/.helm/remotes.toml"
                ))
            })?;
            return Ok(Self {
                host: entry.host,
                port: entry.port,
                user: entry.user,
                ssh_opts: entry.ssh_opts,
            });
        }
        let host = object
            .get("host")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ToolError::InvalidInput("either `remote` or `host` must be provided".to_owned())
            })?
            .to_owned();
        let port = object
            .get("port")
            .and_then(Value::as_u64)
            .map(|p| p as u16)
            .unwrap_or(22);
        let user = object
            .get("user")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let ssh_opts = object
            .get("ssh_opts")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(Self {
            host,
            port,
            user,
            ssh_opts,
        })
    }

    fn target(&self) -> String {
        match self.user.as_deref() {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }

    fn ssh_argv(&self) -> Vec<String> {
        let mut argv = vec!["ssh".to_owned()];
        if let Some(opts) = self.ssh_opts.as_deref() {
            for tok in opts.split_whitespace() {
                argv.push(tok.to_owned());
            }
        }
        if self.port != 22 {
            argv.push("-p".to_owned());
            argv.push(self.port.to_string());
        }
        argv.push("-o".to_owned());
        argv.push("BatchMode=yes".to_owned());
        argv.push("-o".to_owned());
        argv.push("ConnectTimeout=10".to_owned());
        argv.push(self.target());
        argv
    }
}

// ── SshTool ──────────────────────────────────────────────────────────────────

/// Runs a non-interactive command on a remote host via `ssh`. Output is
/// external-tainted by the agent layer because it crossed a trust boundary.
#[derive(Debug, Default)]
pub struct SshTool;

#[async_trait]
impl Tool for SshTool {
    fn name(&self) -> &'static str {
        "ssh"
    }

    fn description(&self) -> &'static str {
        "Run a non-interactive shell command on a remote host registered with `helm remote add`."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "remote": {"type": "string", "description": "Name from ~/.helm/remotes.toml"},
                "host": {"type": "string"},
                "user": {"type": "string"},
                "port": {"type": "integer", "minimum": 1, "maximum": 65535},
                "ssh_opts": {"type": "string"},
                "command": {"type": "string", "description": "Shell command line to run remotely (passed to /bin/sh -c)"}
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let object = input
            .as_object()
            .ok_or_else(|| ToolError::InvalidInput("ssh input must be an object".to_owned()))?;
        let endpoint = RemoteEndpoint::resolve(object)?;
        let command = object
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`command` is required".to_owned()))?;
        if command.trim().is_empty() {
            return Err(ToolError::InvalidInput(
                "`command` cannot be empty".to_owned(),
            ));
        }

        let mut argv = endpoint.ssh_argv();
        argv.push(command.to_owned());
        run_capture(&argv, ctx.timeout, ctx.max_output_bytes, "ssh").await
    }
}

// ── ScpTool ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ScpTool;

#[async_trait]
impl Tool for ScpTool {
    fn name(&self) -> &'static str {
        "scp"
    }

    fn description(&self) -> &'static str {
        "Copy a file to or from a remote host using scp."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "remote": {"type": "string"},
                "host": {"type": "string"},
                "user": {"type": "string"},
                "port": {"type": "integer", "minimum": 1, "maximum": 65535},
                "ssh_opts": {"type": "string"},
                "src": {"type": "string", "description": "Source path"},
                "dst": {"type": "string", "description": "Destination path"},
                "direction": {"type": "string", "enum": ["up", "down"], "description": "`up` copies local→remote, `down` copies remote→local"}
            },
            "required": ["src", "dst", "direction"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let object = input
            .as_object()
            .ok_or_else(|| ToolError::InvalidInput("scp input must be an object".to_owned()))?;
        let endpoint = RemoteEndpoint::resolve(object)?;
        let src = object
            .get("src")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`src` is required".to_owned()))?;
        let dst = object
            .get("dst")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`dst` is required".to_owned()))?;
        let direction = object
            .get("direction")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`direction` is required".to_owned()))?;

        let mut argv = vec!["scp".to_owned()];
        if let Some(opts) = endpoint.ssh_opts.as_deref() {
            for tok in opts.split_whitespace() {
                argv.push(tok.to_owned());
            }
        }
        if endpoint.port != 22 {
            argv.push("-P".to_owned());
            argv.push(endpoint.port.to_string());
        }
        argv.push("-o".to_owned());
        argv.push("BatchMode=yes".to_owned());
        let target = endpoint.target();
        match direction {
            "up" => {
                argv.push(src.to_owned());
                argv.push(format!("{target}:{dst}"));
            }
            "down" => {
                argv.push(format!("{target}:{src}"));
                argv.push(dst.to_owned());
            }
            other => {
                return Err(ToolError::InvalidInput(format!(
                    "direction must be 'up' or 'down', got {other}"
                )));
            }
        }
        run_capture(&argv, ctx.timeout, ctx.max_output_bytes, "scp").await
    }
}

// ── RsyncTool ────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct RsyncTool;

#[async_trait]
impl Tool for RsyncTool {
    fn name(&self) -> &'static str {
        "rsync"
    }

    fn description(&self) -> &'static str {
        "Synchronize a directory tree to or from a remote host using rsync over ssh."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "remote": {"type": "string"},
                "host": {"type": "string"},
                "user": {"type": "string"},
                "port": {"type": "integer", "minimum": 1, "maximum": 65535},
                "ssh_opts": {"type": "string"},
                "src": {"type": "string"},
                "dst": {"type": "string"},
                "direction": {"type": "string", "enum": ["up", "down"]},
                "delete": {"type": "boolean", "description": "Pass --delete to rsync (mirror mode)"}
            },
            "required": ["src", "dst", "direction"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let object = input
            .as_object()
            .ok_or_else(|| ToolError::InvalidInput("rsync input must be an object".to_owned()))?;
        let endpoint = RemoteEndpoint::resolve(object)?;
        let src = object
            .get("src")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`src` is required".to_owned()))?;
        let dst = object
            .get("dst")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`dst` is required".to_owned()))?;
        let direction = object
            .get("direction")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`direction` is required".to_owned()))?;
        let delete = object
            .get("delete")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let mut argv = vec!["rsync".to_owned(), "-az".to_owned()];
        if delete {
            argv.push("--delete".to_owned());
        }

        let mut ssh_opts = String::from("ssh -o BatchMode=yes");
        if endpoint.port != 22 {
            ssh_opts.push_str(&format!(" -p {}", endpoint.port));
        }
        if let Some(opts) = endpoint.ssh_opts.as_deref() {
            ssh_opts.push(' ');
            ssh_opts.push_str(opts);
        }
        argv.push("-e".to_owned());
        argv.push(ssh_opts);

        let target = endpoint.target();
        match direction {
            "up" => {
                argv.push(src.to_owned());
                argv.push(format!("{target}:{dst}"));
            }
            "down" => {
                argv.push(format!("{target}:{src}"));
                argv.push(dst.to_owned());
            }
            other => {
                return Err(ToolError::InvalidInput(format!(
                    "direction must be 'up' or 'down', got {other}"
                )));
            }
        }
        run_capture(&argv, ctx.timeout, ctx.max_output_bytes, "rsync").await
    }
}

async fn run_capture(
    argv: &[String],
    duration: Duration,
    max_bytes: usize,
    label: &str,
) -> Result<ToolOutput, ToolError> {
    if argv.is_empty() {
        return Err(ToolError::Other("empty argv".to_owned()));
    }
    let started = Instant::now();
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ToolError::Other("failed to capture stdout".to_owned()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| ToolError::Other("failed to capture stderr".to_owned()))?;
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).await?;
        Ok::<Vec<u8>, std::io::Error>(buf)
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        stderr.read_to_end(&mut buf).await?;
        Ok::<Vec<u8>, std::io::Error>(buf)
    });
    let status = match timeout(duration, child.wait()).await {
        Ok(r) => r?,
        Err(_) => {
            child.kill().await?;
            let _ = child.wait().await;
            let stdout_bytes = stdout_task
                .await
                .map_err(|e| ToolError::Other(e.to_string()))??;
            let stderr_bytes = stderr_task
                .await
                .map_err(|e| ToolError::Other(e.to_string()))??;
            return Ok(timed_out_output(
                label,
                &stdout_bytes,
                &stderr_bytes,
                started.elapsed(),
                max_bytes,
            ));
        }
    };
    let stdout_bytes = stdout_task
        .await
        .map_err(|e| ToolError::Other(e.to_string()))??;
    let stderr_bytes = stderr_task
        .await
        .map_err(|e| ToolError::Other(e.to_string()))??;
    let stdout_text = String::from_utf8_lossy(&stdout_bytes);
    let stderr_text = String::from_utf8_lossy(&stderr_bytes);
    let exit_code = status.code();
    let full = format!(
        "STDOUT:\n{stdout_text}\nSTDERR:\n{stderr_text}\n[exit code: {}]",
        exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_owned())
    );
    let (content, truncated) = truncate(&full, max_bytes);
    let mut metadata = Map::new();
    metadata.insert("tool".to_owned(), json!(label));
    metadata.insert(
        "exit_code".to_owned(),
        exit_code.map_or(Value::Null, Value::from),
    );
    metadata.insert(
        "duration_ms".to_owned(),
        json!(started.elapsed().as_millis()),
    );
    metadata.insert("stdout_bytes".to_owned(), json!(stdout_bytes.len()));
    metadata.insert("stderr_bytes".to_owned(), json!(stderr_bytes.len()));
    metadata.insert("truncated".to_owned(), Value::Bool(truncated));
    metadata.insert("argv".to_owned(), json!(argv));
    Ok(ToolOutput {
        content,
        success: status.success(),
        metadata,
    })
}

fn timed_out_output(
    label: &str,
    stdout: &[u8],
    stderr: &[u8],
    elapsed: Duration,
    max_bytes: usize,
) -> ToolOutput {
    let stdout_text = String::from_utf8_lossy(stdout);
    let stderr_text = String::from_utf8_lossy(stderr);
    let full = format!("STDOUT:\n{stdout_text}\nSTDERR:\n{stderr_text}\n[timed out]");
    let (content, truncated) = truncate(&full, max_bytes);
    let mut metadata = Map::new();
    metadata.insert("tool".to_owned(), json!(label));
    metadata.insert("timed_out".to_owned(), Value::Bool(true));
    metadata.insert("duration_ms".to_owned(), json!(elapsed.as_millis()));
    metadata.insert("truncated".to_owned(), Value::Bool(truncated));
    ToolOutput {
        content,
        success: false,
        metadata,
    }
}

fn truncate(input: &str, max_bytes: usize) -> (String, bool) {
    let bytes = input.as_bytes();
    if bytes.len() <= max_bytes {
        return (input.to_owned(), false);
    }
    let omitted = bytes.len().saturating_sub(max_bytes);
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let mut out = input[..end].to_owned();
    out.push_str(&format!("\n[output truncated; {omitted} bytes omitted]"));
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_endpoint_resolves_inline_host() {
        let input = serde_json::json!({
            "host": "prod.example.com",
            "user": "ubuntu",
            "port": 2222,
            "ssh_opts": "-i ~/.ssh/prod"
        });
        let endpoint = RemoteEndpoint::resolve(input.as_object().unwrap()).unwrap();

        assert_eq!(endpoint.target(), "ubuntu@prod.example.com");
        assert_eq!(
            endpoint.ssh_argv(),
            vec![
                "ssh",
                "-i",
                "~/.ssh/prod",
                "-p",
                "2222",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "ubuntu@prod.example.com"
            ]
        );
    }

    #[test]
    fn truncate_preserves_utf8_boundaries() {
        let input = "alpha beta gamma delta 😀";
        let (output, truncated) = truncate(input, 10);

        assert!(truncated);
        assert!(std::str::from_utf8(output.as_bytes()).is_ok());
        assert!(output.contains("[output truncated;"));
    }

    #[test]
    fn timed_out_output_marks_failure() {
        let output = timed_out_output("ssh", b"hello", b"world", Duration::from_millis(125), 64);

        assert!(!output.success);
        assert!(output.content.contains("[timed out]"));
        assert_eq!(output.metadata.get("tool"), Some(&json!("ssh")));
        assert_eq!(output.metadata.get("timed_out"), Some(&Value::Bool(true)));
    }
}
