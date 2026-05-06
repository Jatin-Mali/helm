//! Typed Linux package manager tool.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    command::{require_confirm, run_command, str_field},
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

/// Tool for detecting and using apt, dnf, or pacman.
#[derive(Debug, Default)]
pub struct PackageTool;

#[async_trait]
impl Tool for PackageTool {
    fn name(&self) -> &'static str {
        "package"
    }

    fn description(&self) -> &'static str {
        "Typed package manager tool: detect apt/dnf/pacman, search, install, remove, update metadata."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": { "type": "string", "enum": ["detect", "search", "install", "remove", "update_metadata"] },
                "name": { "type": "string" },
                "manager": { "type": "string", "enum": ["apt", "dnf", "pacman"] },
                "confirm": { "type": "boolean" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = str_field(&input, "action")?;
        let manager = input
            .get("manager")
            .and_then(Value::as_str)
            .unwrap_or("apt");
        match action.as_str() {
            "detect" => detect_manager(ctx).await,
            "search" => {
                let name = str_field(&input, "name")?;
                match manager {
                    "apt" => run_command("apt-cache", &["search".into(), name], ctx).await,
                    "dnf" => run_command("dnf", &["search".into(), name], ctx).await,
                    "pacman" => run_command("pacman", &["-Ss".into(), name], ctx).await,
                    _ => Err(ToolError::InvalidInput(format!(
                        "unknown manager: {manager}"
                    ))),
                }
            }
            "install" => {
                require_confirm(&input, "package install")?;
                let name = str_field(&input, "name")?;
                match manager {
                    "apt" => {
                        run_command("apt-get", &["install".into(), "-y".into(), name], ctx).await
                    }
                    "dnf" => run_command("dnf", &["install".into(), "-y".into(), name], ctx).await,
                    "pacman" => {
                        run_command("pacman", &["-S".into(), "--noconfirm".into(), name], ctx).await
                    }
                    _ => Err(ToolError::InvalidInput(format!(
                        "unknown manager: {manager}"
                    ))),
                }
            }
            "remove" => {
                require_confirm(&input, "package remove")?;
                let name = str_field(&input, "name")?;
                match manager {
                    "apt" => {
                        run_command("apt-get", &["remove".into(), "-y".into(), name], ctx).await
                    }
                    "dnf" => run_command("dnf", &["remove".into(), "-y".into(), name], ctx).await,
                    "pacman" => {
                        run_command("pacman", &["-R".into(), "--noconfirm".into(), name], ctx).await
                    }
                    _ => Err(ToolError::InvalidInput(format!(
                        "unknown manager: {manager}"
                    ))),
                }
            }
            "update_metadata" => {
                require_confirm(&input, "package metadata update")?;
                match manager {
                    "apt" => run_command("apt-get", &["update".into()], ctx).await,
                    "dnf" => run_command("dnf", &["makecache".into()], ctx).await,
                    "pacman" => run_command("pacman", &["-Sy".into()], ctx).await,
                    _ => Err(ToolError::InvalidInput(format!(
                        "unknown manager: {manager}"
                    ))),
                }
            }
            _ => Err(ToolError::InvalidInput(format!(
                "unsupported package action: {action}"
            ))),
        }
    }
}

async fn detect_manager(ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
    for candidate in ["apt-get", "dnf", "pacman"] {
        let output = run_command("which", &[candidate.to_owned()], ctx).await;
        if let Ok(output) = output
            && output.success
        {
            return Ok(output);
        }
    }
    Err(ToolError::Other(
        "no supported package manager found".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use crate::{package::PackageTool, tool::ToolContext};

    use super::Tool;

    #[tokio::test]
    async fn detect_package_manager_edge_case() {
        let dir = tempdir().unwrap();
        let _ = PackageTool
            .execute(
                json!({"action": "detect"}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await;
    }

    #[tokio::test]
    async fn install_requires_confirmation_error_path() {
        let dir = tempdir().unwrap();
        let err = PackageTool
            .execute(
                json!({"action": "install", "name": "curl"}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("confirm=true"));
    }
}
