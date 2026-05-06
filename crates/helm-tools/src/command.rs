//! Small command runner shared by typed Linux tools.

use std::{path::Path, time::Instant};

use serde_json::{Map, Value, json};
use tokio::{process::Command, time};

use crate::tool::{ToolContext, ToolError, ToolOutput};

pub async fn run_command(
    program: &str,
    args: &[String],
    ctx: &ToolContext,
) -> Result<ToolOutput, ToolError> {
    let started = Instant::now();
    let mut command = Command::new(program);
    command.args(args).current_dir(&ctx.working_dir);
    command.kill_on_drop(true);
    let output = match time::timeout(ctx.timeout, command.output()).await {
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
