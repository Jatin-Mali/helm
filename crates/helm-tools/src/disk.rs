//! Typed disk and filesystem usage inspection tool.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    command::{path_or_default, run_command, str_field, u64_field},
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

/// Tool for df, du, largest-file, mount, and inode checks.
#[derive(Debug, Default)]
pub struct DiskTool;

#[async_trait]
impl Tool for DiskTool {
    fn name(&self) -> &'static str {
        "disk"
    }

    fn description(&self) -> &'static str {
        "Typed disk tool: df, du, largest files, mount info, inode usage."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": { "type": "string", "enum": ["df", "du", "largest_files", "mount_info", "inode_usage"] },
                "path": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = str_field(&input, "action")?;
        match action.as_str() {
            "df" => run_command("df", &["-h".into()], ctx).await,
            "du" => {
                let path = path_or_default(&input, "path", &ctx.working_dir);
                run_command("du", &["-sh".into(), path], ctx).await
            }
            "largest_files" => {
                let path = path_or_default(&input, "path", &ctx.working_dir);
                let limit = usize::try_from(u64_field(&input, "limit", 20)).unwrap_or(20);
                let output = run_command(
                    "find",
                    &[
                        path,
                        "-type".into(),
                        "f".into(),
                        "-printf".into(),
                        "%s %p\n".into(),
                    ],
                    ctx,
                )
                .await?;
                Ok(sort_largest(output, limit))
            }
            "mount_info" => run_command("findmnt", &Vec::<String>::new(), ctx).await,
            "inode_usage" => run_command("df", &["-ih".into()], ctx).await,
            _ => Err(ToolError::InvalidInput(format!(
                "unsupported disk action: {action}"
            ))),
        }
    }
}

fn sort_largest(mut output: ToolOutput, limit: usize) -> ToolOutput {
    let mut rows = output
        .content
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        size_prefix(right)
            .unwrap_or(0)
            .cmp(&size_prefix(left).unwrap_or(0))
    });
    output.content = rows.into_iter().take(limit).collect::<Vec<_>>().join("\n");
    output
}

fn size_prefix(row: &str) -> Option<u64> {
    row.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::{disk::DiskTool, tool::ToolContext};

    use super::Tool;

    #[tokio::test]
    async fn df_happy_path() {
        let dir = tempdir().unwrap();
        let out = DiskTool
            .execute(
                json!({"action": "df"}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap();
        assert!(out.content.contains("Filesystem"));
    }

    #[tokio::test]
    async fn largest_files_edge_case() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a"), "a").unwrap();
        fs::write(dir.path().join("b"), "bbbb").unwrap();
        let out = DiskTool
            .execute(
                json!({"action": "largest_files", "path": dir.path(), "limit": 1}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap();
        assert!(out.content.contains("b"));
    }
}
