//! Tool registry and provider schema export.

use std::collections::HashMap;

use helm_core::Capability;
use helm_providers::ToolSchema;
use serde_json::Value;

use crate::{
    browser::BrowserTool,
    disk::DiskTool,
    fs_read::FsReadTool,
    fs_write::FsWriteTool,
    git::GitTool,
    http::HttpTool,
    logs::LogsTool,
    mcp::McpTool,
    network::NetworkTool,
    package::PackageTool,
    process::ProcessTool,
    search::SearchTool,
    service::ServiceTool,
    shell::ShellTool,
    tool::{Tool, ToolContext, ToolError, ToolOutput},
    validator::InputValidator,
};

/// Name-indexed registry of tools available to the ReAct loop.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Creates a registry containing the default v1.1 tools.
    pub fn with_default_tools() -> Self {
        let mut registry = Self::new();
        registry.register(Box::<ShellTool>::default());
        registry.register(Box::<FsReadTool>::default());
        registry.register(Box::<FsWriteTool>::default());
        registry.register(Box::<ProcessTool>::default());
        registry.register(Box::<ServiceTool>::default());
        registry.register(Box::<PackageTool>::default());
        registry.register(Box::<DiskTool>::default());
        registry.register(Box::<NetworkTool>::default());
        registry.register(Box::<LogsTool>::default());
        registry.register(Box::<BrowserTool>::default());
        registry.register(Box::<GitTool>::default());
        registry.register(Box::<McpTool>::default());
        registry.register(Box::<HttpTool>::default());
        registry.register(Box::<SearchTool>::default());
        registry
    }

    /// Registers or replaces a tool by its declared name.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_owned(), tool);
    }

    /// Executes a registered tool by name.
    ///
    /// Validates `input` against the tool's declared JSON Schema before
    /// dispatching.  Returns `ToolError::InvalidInput` if validation fails.
    pub async fn execute(
        &self,
        name: &str,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::Other(format!("no tool named: {name}")))?;

        // Validate input schema before execution.
        match InputValidator::new(tool.input_schema()) {
            Ok(validator) => validator.validate(&input)?,
            Err(e) => {
                // Bad schema in the tool definition — log and skip validation.
                tracing::warn!(
                    "tool '{}' has invalid schema, skipping validation: {}",
                    name,
                    e
                );
            }
        }

        tool.execute(input, ctx).await
    }

    /// Returns provider tool schemas for all registered tools.
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .values()
            .map(|tool| ToolSchema {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                input_schema: tool.input_schema(),
            })
            .collect()
    }

    /// Returns the capability required for a tool call and input.
    pub fn required_capability(&self, name: &str, input: &Value) -> Capability {
        required_capability_for_tool(name, input)
    }

    /// Returns all registered tool names.
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
}

/// Maps a tool call to its required machine-control capability.
pub fn required_capability_for_tool(name: &str, input: &Value) -> Capability {
    match name {
        "fs_read" => Capability::FsRead,
        "fs_write" => Capability::FsWrite,
        "shell" => match input.get("mode").and_then(Value::as_str) {
            Some("shell") => Capability::ShellShell,
            _ => Capability::ShellExec,
        },
        "service" => match input.get("action").and_then(Value::as_str) {
            Some("status" | "logs") => Capability::ShellExec,
            _ => Capability::SystemService,
        },
        "package" => match input.get("action").and_then(Value::as_str) {
            Some("detect" | "search") => Capability::ShellExec,
            _ => Capability::PkgInstall,
        },
        "browser" | "browser_open" | "browser_snapshot" | "browser_click" => {
            Capability::BrowserControl
        }
        "network" => Capability::NetworkOut,
        "http" => Capability::NetworkOut,
        "search" => Capability::FsRead,
        "process" => Capability::ShellExec,
        "disk" => Capability::FsRead,
        "logs" => Capability::ShellExec,
        "git" => Capability::ShellExec,
        "mcp" => Capability::ShellExec,
        name if name.contains("delete") => Capability::FsDelete,
        _ => Capability::ShellShell,
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolRegistry")
            .field("tool_names", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_default_tools()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use crate::tool::ToolContext;

    use super::ToolRegistry;

    #[tokio::test]
    async fn dispatch_works_happy_path() {
        let dir = tempdir().unwrap();
        let registry = ToolRegistry::with_default_tools();
        let output = registry
            .execute(
                "shell",
                json!({"command": "printf", "args": ["ok"]}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap();

        assert!(output.content.contains("ok"));
    }

    #[tokio::test]
    async fn missing_tool_errors_error_path() {
        let dir = tempdir().unwrap();
        let registry = ToolRegistry::new();
        let error = registry
            .execute(
                "missing",
                json!({}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("no tool named"));
    }

    #[test]
    fn schemas_include_all_default_tools_edge_case() {
        let registry = ToolRegistry::with_default_tools();
        let names = registry
            .schemas()
            .into_iter()
            .map(|schema| schema.name)
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 14);
        assert!(names.iter().any(|name| name == "shell"));
        assert!(names.iter().any(|name| name == "browser"));
        assert!(names.iter().any(|name| name == "git"));
        assert!(names.iter().any(|name| name == "mcp"));
    }

    #[test]
    fn shell_mode_maps_to_shell_run_edge_case() {
        let registry = ToolRegistry::with_default_tools();

        assert_eq!(
            registry.required_capability("shell", &json!({"command": "date"})),
            helm_core::Capability::ShellExec
        );
        assert_eq!(
            registry.required_capability("shell", &json!({"command": "date", "mode": "shell"})),
            helm_core::Capability::ShellShell
        );
    }
}
