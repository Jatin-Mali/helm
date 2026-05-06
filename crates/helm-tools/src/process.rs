//! Typed Linux process inspection and control tool.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    command::{require_confirm, run_command, str_field, u64_field},
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

/// Tool for listing, inspecting, and terminating Linux processes.
#[derive(Debug, Default)]
pub struct ProcessTool;

#[async_trait]
impl Tool for ProcessTool {
    fn name(&self) -> &'static str {
        "process"
    }

    fn description(&self) -> &'static str {
        "Typed Linux process tool: list, inspect pid, kill with confirmation, top memory/cpu."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": { "type": "string", "enum": ["list", "inspect", "kill", "top_memory", "top_cpu"] },
                "pid": { "type": "integer", "minimum": 1 },
                "signal": { "type": "string", "enum": ["TERM", "KILL", "INT", "HUP"] },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                "confirm": { "type": "boolean" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = str_field(&input, "action")?;
        match action.as_str() {
            "list" => {
                run_ps(
                    &["-eo", "pid,ppid,comm,%cpu,%mem", "--sort=-%mem"],
                    ctx,
                    None,
                )
                .await
            }
            "inspect" => {
                let pid = u64_field(&input, "pid", 0);
                if pid == 0 {
                    return Err(ToolError::InvalidInput("pid is required".to_owned()));
                }
                run_ps(
                    &[
                        "-p",
                        &pid.to_string(),
                        "-o",
                        "pid,ppid,user,stat,etime,comm,args",
                    ],
                    ctx,
                    None,
                )
                .await
            }
            "kill" => {
                require_confirm(&input, "process kill")?;
                let pid = u64_field(&input, "pid", 0);
                if pid == 0 {
                    return Err(ToolError::InvalidInput("pid is required".to_owned()));
                }
                let signal = input
                    .get("signal")
                    .and_then(Value::as_str)
                    .unwrap_or("TERM");
                run_command("kill", &[format!("-{signal}"), pid.to_string()], ctx).await
            }
            "top_memory" => {
                let limit = usize::try_from(u64_field(&input, "limit", 10)).unwrap_or(10);
                run_ps(
                    &["-eo", "pid,ppid,comm,%cpu,%mem", "--sort=-%mem"],
                    ctx,
                    Some(limit),
                )
                .await
            }
            "top_cpu" => {
                let limit = usize::try_from(u64_field(&input, "limit", 10)).unwrap_or(10);
                run_ps(
                    &["-eo", "pid,ppid,comm,%cpu,%mem", "--sort=-%cpu"],
                    ctx,
                    Some(limit),
                )
                .await
            }
            _ => Err(ToolError::InvalidInput(format!(
                "unsupported process action: {action}"
            ))),
        }
    }
}

async fn run_ps(
    args: &[&str],
    ctx: &ToolContext,
    limit: Option<usize>,
) -> Result<ToolOutput, ToolError> {
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let mut output = run_command("ps", &args, ctx).await?;
    if let Some(limit) = limit {
        let mut lines = output.content.lines();
        let kept = lines
            .by_ref()
            .take(limit.saturating_add(4))
            .collect::<Vec<_>>();
        output.content = kept.join("\n");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use crate::{process::ProcessTool, tool::ToolContext};

    use super::Tool;

    #[tokio::test]
    async fn list_processes_happy_path() {
        let dir = tempdir().unwrap();
        let out = ProcessTool
            .execute(
                json!({"action": "list"}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap();
        assert!(out.content.contains("PID"));
    }

    #[tokio::test]
    async fn kill_requires_confirmation_error_path() {
        let dir = tempdir().unwrap();
        let err = ProcessTool
            .execute(
                json!({"action": "kill", "pid": 1}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("confirm=true"));
    }

    #[tokio::test]
    async fn inspect_requires_pid_edge_case() {
        let dir = tempdir().unwrap();
        let err = ProcessTool
            .execute(
                json!({"action": "inspect"}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("pid"));
    }
}
