//! Typed disk and filesystem usage inspection tool.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    command::{path_or_default, run_command, run_command_with_timeout, str_field, u64_field},
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

const DISK_SCAN_TIMEOUT_SECS: u64 = 5 * 60;

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
            "df" => {
                let path = path_or_default(&input, "path", &ctx.working_dir);
                run_command("df", &["-h".into(), path], ctx).await
            }
            "du" => {
                let path = path_or_default(&input, "path", &ctx.working_dir);
                let limit = usize::try_from(u64_field(&input, "limit", 25)).unwrap_or(25);
                disk_usage_tree(&path, limit, ctx).await
            }
            "largest_files" => {
                let path = path_or_default(&input, "path", &ctx.working_dir);
                let limit = usize::try_from(u64_field(&input, "limit", 20)).unwrap_or(20);
                largest_files(&path, limit, ctx).await
            }
            "mount_info" => run_command("findmnt", &Vec::<String>::new(), ctx).await,
            "inode_usage" => run_command("df", &["-ih".into()], ctx).await,
            _ => Err(ToolError::InvalidInput(format!(
                "unsupported disk action: {action}"
            ))),
        }
    }

    fn allowed_in_diagnose(&self) -> bool {
        true
    }

    fn all_write_ops_gated_in_diagnose(&self) -> bool {
        true // disk has only read-only actions (df, du, lsblk, smart, largest_files)
    }
}

async fn disk_usage_tree(
    path: &str,
    limit: usize,
    ctx: &ToolContext,
) -> Result<ToolOutput, ToolError> {
    let timeout_duration = ctx
        .timeout
        .max(std::time::Duration::from_secs(DISK_SCAN_TIMEOUT_SECS));
    let mut output = run_command_with_timeout(
        "du",
        &[
            "-x".into(),
            "-B1".into(),
            "--max-depth=1".into(),
            path.to_owned(),
        ],
        ctx,
        timeout_duration,
    )
    .await?;
    output = sort_largest(output, limit);
    output.content = output
        .content
        .lines()
        .map(format_du_row)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(output)
}

async fn largest_files(
    path: &str,
    limit: usize,
    ctx: &ToolContext,
) -> Result<ToolOutput, ToolError> {
    let timeout_duration = ctx
        .timeout
        .max(std::time::Duration::from_secs(DISK_SCAN_TIMEOUT_SECS));
    let script =
        r#"find "$1" -xdev -type f -printf '%s %p\n' 2>/dev/null | sort -nr | head -n "$2""#;
    let mut output = run_command_with_timeout(
        "sh",
        &[
            "-c".into(),
            script.into(),
            "helm-disk-largest-files".into(),
            path.to_owned(),
            limit.to_string(),
        ],
        ctx,
        timeout_duration,
    )
    .await?;
    output.content = output
        .content
        .lines()
        .map(format_du_row)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(output)
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

fn format_du_row(row: &str) -> String {
    let Some(size) = size_prefix(row) else {
        return row.to_owned();
    };
    let rest = row
        .split_once(char::is_whitespace)
        .map(|(_, tail)| tail.trim_start())
        .unwrap_or("");
    if rest.is_empty() {
        human_bytes(size)
    } else {
        format!("{}\t{rest}", human_bytes(size))
    }
}

fn size_prefix(row: &str) -> Option<u64> {
    row.split_whitespace().next()?.parse().ok()
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::{disk::DiskTool, tool::ToolContext};

    use super::{Tool, format_du_row};

    #[tokio::test]
    async fn df_happy_path() {
        let dir = tempdir().unwrap();
        let out = DiskTool
            .execute(
                json!({"action": "df", "path": dir.path()}),
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

    #[tokio::test]
    async fn du_returns_top_level_usage_happy_path() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("cache")).unwrap();
        fs::write(dir.path().join("cache").join("blob"), vec![0_u8; 4096]).unwrap();

        let out = DiskTool
            .execute(
                json!({"action": "du", "path": dir.path(), "limit": 5}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap();

        assert!(out.content.contains("cache"));
        assert!(out.content.contains("KiB") || out.content.contains("B"));
    }

    #[test]
    fn byte_rows_are_humanized_edge_case() {
        assert_eq!(format_du_row("1048576 /tmp/blob"), "1.0 MiB\t/tmp/blob");
        assert_eq!(format_du_row("not-a-size"), "not-a-size");
    }
}
