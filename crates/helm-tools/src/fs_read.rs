//! Filesystem read tool with allowlist, denylist, and binary-safe output.

use std::{
    env, io,
    path::{Component, Path, PathBuf},
};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use tokio::fs as tokio_fs;

use crate::tool::{Tool, ToolContext, ToolError, ToolOutput};

const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Tool that reads UTF-8 or binary files from approved filesystem roots.
#[derive(Debug, Default)]
pub struct FsReadTool;

#[async_trait]
impl Tool for FsReadTool {
    fn name(&self) -> &'static str {
        "fs_read"
    }

    fn description(&self) -> &'static str {
        "Read a file with optional line range and byte limit, returning base64 for non-UTF-8 data."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "start_line": { "type": "integer", "minimum": 1 },
                "end_line": { "type": "integer", "minimum": 1 },
                "max_bytes": { "type": "integer", "minimum": 1 }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let parsed = ReadInput::parse(input)?;
        let path = validate_existing_path(&parsed.path, ctx)?;
        let metadata = tokio_fs::metadata(&path).await?;
        if !metadata.is_file() {
            return Err(ToolError::InvalidInput(format!(
                "path is not a regular file: {}",
                path.display()
            )));
        }
        if metadata.len() > parsed.max_bytes {
            return Err(ToolError::OutputTooLarge);
        }
        let bytes = tokio_fs::read(&path).await?;
        let mut output_metadata = Map::new();
        output_metadata.insert("path".to_owned(), json!(path.to_string_lossy().to_string()));
        output_metadata.insert("bytes_read".to_owned(), json!(bytes.len()));

        match String::from_utf8(bytes) {
            Ok(text) => {
                let total_lines = count_lines(&text);
                let (content, truncated_at_eof) =
                    select_lines(&text, parsed.start_line, parsed.end_line)?;
                output_metadata.insert("total_lines".to_owned(), json!(total_lines));
                output_metadata.insert("encoding".to_owned(), json!("utf-8"));
                if truncated_at_eof {
                    output_metadata.insert("truncated_at_eof".to_owned(), Value::Bool(true));
                }
                Ok(ToolOutput {
                    content,
                    success: true,
                    metadata: output_metadata,
                })
            }
            Err(error) => {
                let encoded = STANDARD.encode(error.into_bytes());
                output_metadata.insert("total_lines".to_owned(), json!(0));
                output_metadata.insert("encoding".to_owned(), json!("base64"));
                Ok(ToolOutput {
                    content: encoded,
                    success: true,
                    metadata: output_metadata,
                })
            }
        }
    }
}

#[derive(Debug)]
struct ReadInput {
    path: PathBuf,
    start_line: Option<u32>,
    end_line: Option<u32>,
    max_bytes: u64,
}

impl ReadInput {
    fn parse(input: Value) -> Result<Self, ToolError> {
        let object = input
            .as_object()
            .ok_or_else(|| ToolError::InvalidInput("fs_read input must be an object".to_owned()))?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path is required".to_owned()))?;
        let start_line = optional_u32(object.get("start_line"), "start_line")?;
        let end_line = optional_u32(object.get("end_line"), "end_line")?;
        if matches!((start_line, end_line), (Some(start), Some(end)) if start > end) {
            return Err(ToolError::InvalidInput(
                "start_line cannot be greater than end_line".to_owned(),
            ));
        }
        let max_bytes = match object.get("max_bytes") {
            Some(value) => value
                .as_u64()
                .ok_or_else(|| ToolError::InvalidInput("max_bytes must be a u64".to_owned()))?,
            None => DEFAULT_MAX_FILE_BYTES,
        };
        if max_bytes == 0 {
            return Err(ToolError::InvalidInput(
                "max_bytes must be positive".to_owned(),
            ));
        }

        Ok(Self {
            path: PathBuf::from(path),
            start_line,
            end_line,
            max_bytes,
        })
    }
}

pub(crate) fn optional_u32(value: Option<&Value>, name: &str) -> Result<Option<u32>, ToolError> {
    match value {
        Some(value) => {
            let raw = value
                .as_u64()
                .ok_or_else(|| ToolError::InvalidInput(format!("{name} must be a u32")))?;
            if raw == 0 || raw > u64::from(u32::MAX) {
                return Err(ToolError::InvalidInput(format!(
                    "{name} must be between 1 and {}",
                    u32::MAX
                )));
            }
            Ok(Some(u32::try_from(raw).map_err(|error| {
                ToolError::InvalidInput(format!("{name} is outside u32 range: {error}"))
            })?))
        }
        None => Ok(None),
    }
}

pub(crate) fn validate_existing_path(path: &Path, ctx: &ToolContext) -> Result<PathBuf, ToolError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.working_dir.join(path)
    };
    let canonical = absolute.canonicalize()?;
    // Denylist applies to the resolved target regardless of origin.
    validate_denylist(&canonical)?;
    // Primary check: canonical path is within an allowed root.
    if validate_canonical_path(&canonical, ctx).is_ok() {
        return Ok(canonical);
    }
    // Symlink-from-HOME: allow a symlink whose origin is inside $HOME even
    // when its resolved target lives outside the static allowlist (e.g.
    // ~/Downloads -> /mnt/hdd/Downloads).
    if let Some(home) = env::var_os("HOME") {
        if absolute.starts_with(&home) {
            return Ok(canonical);
        }
    }
    Err(ToolError::PathDenied(canonical))
}

pub(crate) fn validate_write_path(
    path: &Path,
    ctx: &ToolContext,
    create_parents: bool,
) -> Result<PathBuf, ToolError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.working_dir.join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| ToolError::InvalidInput("path must have a parent".to_owned()))?;
    if parent.exists() {
        let canonical_parent = parent.canonicalize()?;
        validate_canonical_path(&canonical_parent, ctx)?;
    } else if create_parents {
        let ancestor = nearest_existing_ancestor(parent)?;
        let canonical_ancestor = ancestor.canonicalize()?;
        validate_canonical_path(&canonical_ancestor, ctx)?;
    } else {
        return Err(ToolError::IoError(io::Error::new(
            io::ErrorKind::NotFound,
            format!("parent directory does not exist: {}", parent.display()),
        )));
    }
    let normalized = normalize_nonexistent_path(&absolute);
    validate_denylist(&normalized)?;
    Ok(absolute)
}

pub(crate) fn validate_canonical_path(path: &Path, ctx: &ToolContext) -> Result<(), ToolError> {
    validate_denylist(path)?;
    let working_dir = ctx.working_dir.canonicalize()?;
    if path.starts_with(&working_dir)
        || allowed_global_roots()
            .iter()
            .any(|root| path.starts_with(root))
    {
        Ok(())
    } else {
        Err(ToolError::PathDenied(path.to_path_buf()))
    }
}

fn allowed_global_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/etc"),
        PathBuf::from("/var/log"),
        PathBuf::from("/tmp"),
        PathBuf::from("/proc"),
        PathBuf::from("/mnt"),
        PathBuf::from("/media"),
    ];
    if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(home));
    }
    roots
}

fn nearest_existing_ancestor(path: &Path) -> Result<PathBuf, ToolError> {
    let mut current = path;
    loop {
        if current.exists() {
            return Ok(current.to_path_buf());
        }
        current = current.parent().ok_or_else(|| {
            ToolError::IoError(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no existing ancestor for {}", path.display()),
            ))
        })?;
    }
}

fn normalize_nonexistent_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn validate_denylist(path: &Path) -> Result<(), ToolError> {
    let text = path.to_string_lossy();
    if text == "/etc/shadow" || text.starts_with("/etc/sudoers") {
        return Err(ToolError::PathDenied(path.to_path_buf()));
    }
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    for index in 0..components.len() {
        let component = &components[index];
        if component == ".gnupg" || component == ".aws" || component == "credentials" {
            return Err(ToolError::PathDenied(path.to_path_buf()));
        }
        if component == ".ssh" {
            if let Some(next) = components.get(index + 1) {
                if next.starts_with("id_") {
                    return Err(ToolError::PathDenied(path.to_path_buf()));
                }
            }
        }
    }
    Ok(())
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn select_lines(
    text: &str,
    start_line: Option<u32>,
    end_line: Option<u32>,
) -> Result<(String, bool), ToolError> {
    if start_line.is_none() && end_line.is_none() {
        return Ok((text.to_owned(), false));
    }
    let start = start_line.unwrap_or(1);
    let mut truncated_at_eof = false;
    if start == 0 {
        return Err(ToolError::InvalidInput(
            "start_line must be at least 1".to_owned(),
        ));
    }
    let lines = text.lines().collect::<Vec<_>>();
    let total = u32::try_from(lines.len()).unwrap_or(u32::MAX);
    let end = end_line.unwrap_or(total);
    if end > total {
        truncated_at_eof = true;
    }
    let selected = lines
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let line_number = u32::try_from(index + 1).ok()?;
            if line_number >= start && line_number <= end {
                Some(line)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok((selected, truncated_at_eof))
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    use tempfile::tempdir;

    use crate::tool::{Tool, ToolContext, ToolError};

    use super::{FsReadTool, select_lines, validate_existing_path};

    fn ctx_at(dir: &tempfile::TempDir) -> ToolContext {
        ToolContext::new(dir.path().to_path_buf())
    }

    fn outside_disallowed_tempdir() -> tempfile::TempDir {
        for root in ["/var/tmp", "/dev/shm", "/run"] {
            if let Ok(dir) = tempfile::Builder::new()
                .prefix("helm-tools")
                .tempdir_in(root)
            {
                return dir;
            }
        }
        panic!("no writable disallowed temp root available");
    }

    #[tokio::test]
    async fn utf8_happy_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "hello\nworld").unwrap();
        let output = FsReadTool
            .execute(json!({"path": path}), &ctx_at(&dir))
            .await
            .unwrap();

        assert_eq!(output.content, "hello\nworld");
        assert_eq!(output.metadata.get("encoding"), Some(&json!("utf-8")));
    }

    #[tokio::test]
    async fn binary_file_returns_base64() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.bin");
        fs::write(&path, [0xff, 0x00, 0x01]).unwrap();
        let output = FsReadTool
            .execute(json!({"path": path}), &ctx_at(&dir))
            .await
            .unwrap();

        assert_eq!(output.content, STANDARD.encode([0xff, 0x00, 0x01]));
        assert_eq!(output.metadata.get("encoding"), Some(&json!("base64")));
    }

    #[tokio::test]
    async fn line_range_edge_case_truncated_at_eof() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "one\ntwo\nthree").unwrap();
        let output = FsReadTool
            .execute(
                json!({"path": path, "start_line": 2, "end_line": 10}),
                &ctx_at(&dir),
            )
            .await
            .unwrap();

        assert_eq!(output.content, "two\nthree");
        assert_eq!(output.metadata.get("truncated_at_eof"), Some(&json!(true)));
    }

    #[test]
    fn allowlist_rejection_error_path() {
        let dir = tempdir().unwrap();
        let outside = outside_disallowed_tempdir();
        let path = outside.path().join("secret.txt");
        fs::write(&path, "secret").unwrap();

        let error = validate_existing_path(&path, &ctx_at(&dir)).unwrap_err();

        assert!(matches!(error, ToolError::PathDenied(_)));
    }

    #[test]
    fn denylist_rejects_etc_shadow() {
        let dir = tempdir().unwrap();
        let error =
            validate_existing_path(std::path::Path::new("/etc/shadow"), &ctx_at(&dir)).unwrap_err();

        assert!(matches!(error, ToolError::PathDenied(_)));
    }

    #[tokio::test]
    async fn symlink_escape_rejected() {
        let dir = tempdir().unwrap();
        let outside = outside_disallowed_tempdir();
        let outside_file = outside.path().join("outside.txt");
        fs::write(&outside_file, "outside").unwrap();
        let link = dir.path().join("link.txt");
        symlink(&outside_file, &link).unwrap();

        let error = FsReadTool
            .execute(json!({"path": link}), &ctx_at(&dir))
            .await
            .unwrap_err();

        assert!(matches!(error, ToolError::PathDenied(_)));
    }

    #[test]
    fn symlink_from_home_to_outside_allowed_edge_case() {
        let ctx_dir = tempdir().unwrap();
        let outside = outside_disallowed_tempdir();
        let outside_file = outside.path().join("data.txt");
        fs::write(&outside_file, "data").unwrap();

        // Simulate ~/some-link -> outside by placing the symlink inside $HOME.
        let home = std::env::var_os("HOME").expect("HOME must be set");
        let link_dir = tempfile::Builder::new()
            .prefix(".helm-test-")
            .tempdir_in(&home)
            .expect("need write access to HOME for this test");
        let link = link_dir.path().join("data.txt");
        symlink(&outside_file, &link).unwrap();

        // Should be allowed because the symlink origin is inside $HOME.
        let result = validate_existing_path(&link, &ctx_at(&ctx_dir));
        assert!(
            result.is_ok(),
            "symlink from HOME should be allowed: {result:?}"
        );
    }

    #[tokio::test]
    async fn file_too_large_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("large.txt");
        fs::write(&path, "abcdef").unwrap();
        let error = FsReadTool
            .execute(json!({"path": path, "max_bytes": 2}), &ctx_at(&dir))
            .await
            .unwrap_err();

        assert!(matches!(error, ToolError::OutputTooLarge));
    }

    #[test]
    fn select_lines_start_after_eof_returns_empty_edge_case() {
        let (selected, truncated) = select_lines("one\ntwo", Some(5), Some(8)).unwrap();

        assert_eq!(selected, "");
        assert!(truncated);
    }
}
