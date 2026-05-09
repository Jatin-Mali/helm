//! Common tool trait, context, output, and error re-export.

use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use serde_json::{Map, Value};

pub use helm_core::ToolError;

/// Asynchronous HELM tool that can be exposed to a provider.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Stable tool name used by model tool calls.
    fn name(&self) -> &'static str;

    /// Human-readable description included in the provider tool schema.
    fn description(&self) -> &'static str;

    /// JSON Schema object describing accepted input.
    fn input_schema(&self) -> Value;

    /// Executes the tool with validated JSON input and runtime context.
    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError>;
}

/// Runtime constraints and filesystem root for tool execution.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Default working directory for process execution and relative paths.
    pub working_dir: PathBuf,
    /// Maximum wall-clock time for one tool invocation.
    pub timeout: Duration,
    /// Maximum output bytes before tool output is truncated.
    pub max_output_bytes: usize,
}

impl ToolContext {
    /// Creates a context rooted at `working_dir` with v1.0.1 defaults.
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            working_dir,
            timeout: Duration::from_secs(120),
            max_output_bytes: 1024 * 1024,
        }
    }
}

/// Result returned by a successfully executed tool, even when the command failed.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    /// Human-readable output sent back to the model.
    pub content: String,
    /// Whether the tool achieved its requested action.
    pub success: bool,
    /// Structured metadata for memory and diagnostics.
    pub metadata: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use serde_json::{Value, json};

    use super::{ToolContext, ToolOutput};

    #[test]
    fn context_defaults_happy_path() {
        let ctx = ToolContext::new(PathBuf::from("/tmp"));

        assert_eq!(ctx.working_dir, PathBuf::from("/tmp"));
        assert_eq!(ctx.timeout, Duration::from_secs(120));
        assert_eq!(ctx.max_output_bytes, 1024 * 1024);
    }

    #[test]
    fn output_can_represent_tool_failure_error_path() {
        let output = ToolOutput {
            content: "bad".to_owned(),
            success: false,
            metadata: serde_json::Map::new(),
        };

        assert!(!output.success);
    }

    #[test]
    fn output_metadata_edge_case_empty_content() {
        let mut metadata = serde_json::Map::new();
        metadata.insert("truncated".to_owned(), Value::Bool(false));
        let output = ToolOutput {
            content: String::new(),
            success: true,
            metadata,
        };

        assert_eq!(output.metadata.get("truncated"), Some(&json!(false)));
    }
}
