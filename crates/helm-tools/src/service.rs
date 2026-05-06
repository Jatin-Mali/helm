//! Typed systemd service control tool.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    command::{require_confirm, run_command, str_field, u64_field},
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

/// Tool wrapping common `systemctl` and `journalctl` service operations.
#[derive(Debug, Default)]
pub struct ServiceTool;

#[async_trait]
impl Tool for ServiceTool {
    fn name(&self) -> &'static str {
        "service"
    }

    fn description(&self) -> &'static str {
        "Typed systemd tool: status/start/stop/restart/enable/disable/logs for one unit."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": { "type": "string", "enum": ["status", "start", "stop", "restart", "enable", "disable", "logs"] },
                "unit": { "type": "string" },
                "lines": { "type": "integer", "minimum": 1, "maximum": 1000 },
                "confirm": { "type": "boolean" }
            },
            "required": ["action", "unit"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = str_field(&input, "action")?;
        let unit = str_field(&input, "unit")?;
        match action.as_str() {
            "status" => {
                run_command(
                    "systemctl",
                    &["--no-pager".into(), "status".into(), unit],
                    ctx,
                )
                .await
            }
            "logs" => {
                let lines = u64_field(&input, "lines", 100).to_string();
                run_command(
                    "journalctl",
                    &["--no-pager".into(), "-u".into(), unit, "-n".into(), lines],
                    ctx,
                )
                .await
            }
            "start" | "stop" | "restart" | "enable" | "disable" => {
                require_confirm(&input, &format!("service {action}"))?;
                run_command("systemctl", &[action, unit], ctx).await
            }
            _ => Err(ToolError::InvalidInput(format!(
                "unsupported service action: {action}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use crate::{service::ServiceTool, tool::ToolContext};

    use super::Tool;

    #[tokio::test]
    async fn status_schema_happy_path() {
        let schema = ServiceTool.input_schema();
        assert!(schema.to_string().contains("status"));
    }

    #[tokio::test]
    async fn start_requires_confirmation_error_path() {
        let dir = tempdir().unwrap();
        let err = ServiceTool
            .execute(
                json!({"action": "start", "unit": "ssh.service"}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("confirm=true"));
    }
}
