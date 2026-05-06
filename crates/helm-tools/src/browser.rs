//! Browser automation tool backed by the PinchTab CLI.

use std::{ffi::OsString, process::Stdio};

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use tokio::{process::Command, time::timeout};

use crate::{Tool, ToolContext, ToolError, ToolOutput};

/// Browser automation tool using `pinchtab` commands.
#[derive(Debug, Clone)]
pub struct BrowserTool {
    bin: OsString,
}

impl BrowserTool {
    /// Creates a browser tool using the `pinchtab` binary from `PATH`.
    pub fn new() -> Self {
        Self {
            bin: OsString::from("pinchtab"),
        }
    }

    /// Creates a browser tool using an explicit binary path, primarily for tests.
    pub fn with_binary(bin: impl Into<OsString>) -> Self {
        Self { bin: bin.into() }
    }

    fn command_args(input: &Value) -> Result<Vec<String>, ToolError> {
        let action = required_str(input, "action")?;
        match action {
            "open" => Ok(vec![
                "nav".to_owned(),
                required_str(input, "url")?.to_owned(),
                "--snap".to_owned(),
            ]),
            "snapshot" => {
                let full = input.get("full").and_then(Value::as_bool).unwrap_or(false);
                let mut args = vec!["snap".to_owned()];
                if full {
                    args.push("--full".to_owned());
                }
                Ok(args)
            }
            "click" => {
                let mut args = vec![
                    "click".to_owned(),
                    required_str(input, "target")?.to_owned(),
                ];
                if input
                    .get("wait_nav")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    args.push("--wait-nav".to_owned());
                }
                args.push("--snap-diff".to_owned());
                Ok(args)
            }
            "type_text" => Ok(vec![
                "type".to_owned(),
                required_str(input, "target")?.to_owned(),
                required_str(input, "text")?.to_owned(),
            ]),
            "fill" => Ok(vec![
                "fill".to_owned(),
                required_str(input, "target")?.to_owned(),
                required_str(input, "text")?.to_owned(),
                "--snap-diff".to_owned(),
            ]),
            "press_key" => Ok(vec![
                "press".to_owned(),
                required_str(input, "key")?.to_owned(),
            ]),
            "wait" => {
                if let Some(ms) = input.get("duration_ms").and_then(Value::as_u64) {
                    Ok(vec!["wait".to_owned(), ms.min(30_000).to_string()])
                } else if let Some(text) = input.get("text").and_then(Value::as_str) {
                    Ok(vec![
                        "wait".to_owned(),
                        "--text".to_owned(),
                        text.to_owned(),
                    ])
                } else {
                    Ok(vec!["wait".to_owned(), "500".to_owned()])
                }
            }
            "screenshot" => {
                let mut args = vec!["screenshot".to_owned()];
                if let Some(path) = input.get("output_path").and_then(Value::as_str) {
                    args.push("-o".to_owned());
                    args.push(path.to_owned());
                }
                Ok(args)
            }
            "extract_text" => {
                let mut args = vec!["text".to_owned()];
                if input.get("full").and_then(Value::as_bool).unwrap_or(false) {
                    args.push("--full".to_owned());
                }
                Ok(args)
            }
            "close" => {
                if let Some(tab_id) = input.get("tab_id").and_then(Value::as_str) {
                    Ok(vec![
                        "tab".to_owned(),
                        "close".to_owned(),
                        tab_id.to_owned(),
                    ])
                } else {
                    Ok(vec!["tab".to_owned(), "close".to_owned()])
                }
            }
            _ => Err(ToolError::InvalidInput(format!(
                "unknown browser action: {action}"
            ))),
        }
    }
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &'static str {
        "browser"
    }

    fn description(&self) -> &'static str {
        "Control a browser through PinchTab. Browser text is external-tainted. Actions: open, snapshot, click, type_text, fill, press_key, wait, screenshot, extract_text, close."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["open", "snapshot", "click", "type_text", "fill", "press_key", "wait", "screenshot", "extract_text", "close"],
                    "description": "Browser action to perform."
                },
                "url": {"type": "string", "description": "URL for open."},
                "target": {"type": "string", "description": "PinchTab ref, CSS selector, text selector, XPath, or find: query."},
                "text": {"type": "string", "description": "Text for fill/type or wait --text."},
                "key": {"type": "string", "description": "Key for press_key, e.g. Enter or Tab."},
                "duration_ms": {"type": "integer", "minimum": 0, "maximum": 30000},
                "output_path": {"type": "string", "description": "Screenshot output path."},
                "tab_id": {"type": "string", "description": "Optional tab id for close."},
                "full": {"type": "boolean", "default": false},
                "wait_nav": {"type": "boolean", "default": false}
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let args = Self::command_args(&input)?;
        let child = Command::new(&self.bin)
            .args(&args)
            .current_dir(&ctx.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    ToolError::Other(
                        "pinchtab binary not found; install PinchTab or remove browser tasks"
                            .to_owned(),
                    )
                } else {
                    ToolError::IoError(error)
                }
            })?;
        let output = timeout(ctx.timeout, child.wait_with_output())
            .await
            .map_err(|_| ToolError::Timeout)?
            .map_err(ToolError::IoError)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut metadata = Map::new();
        metadata.insert("action".to_owned(), input["action"].clone());
        metadata.insert("external_taint".to_owned(), Value::Bool(true));
        metadata.insert("stdout_bytes".to_owned(), json!(output.stdout.len()));
        metadata.insert("stderr_bytes".to_owned(), json!(output.stderr.len()));
        metadata.insert(
            "exit_code".to_owned(),
            output.status.code().map(Value::from).unwrap_or(Value::Null),
        );
        Ok(ToolOutput {
            content: format!("STDOUT:\n{stdout}\nSTDERR:\n{stderr}"),
            success: output.status.success(),
            metadata,
        })
    }
}

fn required_str<'a>(input: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    input.get(key).and_then(Value::as_str).ok_or_else(|| {
        ToolError::InvalidInput(format!(
            "{key} is required and must be a string for browser action"
        ))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::browser::BrowserTool;
    use crate::tool::Tool;

    #[test]
    fn schema_contains_required_actions_happy_path() {
        let schema = BrowserTool::new().input_schema();
        let actions = schema["properties"]["action"]["enum"]
            .as_array()
            .cloned()
            .unwrap();

        assert!(actions.contains(&json!("open")));
        assert!(actions.contains(&json!("extract_text")));
    }

    #[test]
    fn command_args_for_open_happy_path() {
        let args = BrowserTool::command_args(&json!({
            "action": "open",
            "url": "https://example.com"
        }))
        .unwrap();

        assert_eq!(args, ["nav", "https://example.com", "--snap"]);
    }

    #[test]
    fn command_args_reject_missing_target_error_path() {
        let error = BrowserTool::command_args(&json!({"action": "click"})).unwrap_err();

        assert!(error.to_string().contains("target is required"));
    }

    #[test]
    fn command_args_caps_wait_duration_edge_case() {
        let args = BrowserTool::command_args(&json!({
            "action": "wait",
            "duration_ms": 999999
        }))
        .unwrap();

        assert_eq!(args, ["wait", "30000"]);
    }
}
