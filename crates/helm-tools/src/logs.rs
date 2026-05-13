//! Typed journal log retrieval tool.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    command::{optional_str, run_command, str_field, u64_field},
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

/// Tool wrapping `journalctl` with unit/time/output limits.
#[derive(Debug, Default)]
pub struct LogsTool;

#[async_trait]
impl Tool for LogsTool {
    fn name(&self) -> &'static str {
        "logs"
    }

    fn description(&self) -> &'static str {
        "Typed journalctl wrapper with unit, since, until, and max-line filters."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": { "type": "string", "enum": ["journal"] },
                "unit": { "type": "string" },
                "since": { "type": "string" },
                "until": { "type": "string" },
                "lines": { "type": "integer", "minimum": 1, "maximum": 2000 }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = str_field(&input, "action")?;
        if action != "journal" {
            return Err(ToolError::InvalidInput(format!(
                "unsupported logs action: {action}"
            )));
        }
        let mut args = vec![
            "--no-pager".to_owned(),
            "-n".to_owned(),
            u64_field(&input, "lines", 100).to_string(),
        ];
        if let Some(unit) = optional_str(&input, "unit") {
            args.push("-u".to_owned());
            args.push(unit);
        }
        if let Some(since) = optional_str(&input, "since") {
            args.push("--since".to_owned());
            args.push(since);
        }
        if let Some(until) = optional_str(&input, "until") {
            args.push("--until".to_owned());
            args.push(until);
        }
        run_command("journalctl", &args, ctx).await
    }

    fn allowed_in_diagnose(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use crate::{logs::LogsTool, tool::ToolContext};

    use super::Tool;

    #[tokio::test]
    async fn schema_mentions_journal_happy_path() {
        assert!(LogsTool.input_schema().to_string().contains("journal"));
    }

    #[tokio::test]
    async fn unknown_action_errors_error_path() {
        let dir = tempdir().unwrap();
        let err = LogsTool
            .execute(
                json!({"action": "tail"}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unsupported"));
    }
}
