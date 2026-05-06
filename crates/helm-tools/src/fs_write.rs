//! Filesystem write tool with atomic overwrite/create behavior.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;

use crate::{
    fs_read::validate_write_path,
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

/// Tool that writes files using safe defaults and atomic replacement.
#[derive(Debug, Default)]
pub struct FsWriteTool;

#[async_trait]
impl Tool for FsWriteTool {
    fn name(&self) -> &'static str {
        "fs_write"
    }

    fn description(&self) -> &'static str {
        "Create, overwrite, or append to a file using atomic writes and overwrite backups."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" },
                "mode": { "type": "string", "enum": ["overwrite", "append", "create_only"] },
                "create_parents": { "type": "boolean" }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let parsed = WriteInput::parse(input)?;
        let path = validate_write_path(&parsed.path, ctx, parsed.create_parents)?;
        if parsed.create_parents {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
        }
        let exists = path.exists();
        let backup_path = match parsed.mode {
            WriteMode::CreateOnly if exists => {
                return Err(ToolError::Other(format!(
                    "refusing to overwrite existing file: {}",
                    path.display()
                )));
            }
            WriteMode::CreateOnly => {
                atomic_write(&path, parsed.content.as_bytes())?;
                None
            }
            WriteMode::Overwrite => atomic_overwrite(&path, parsed.content.as_bytes())?,
            WriteMode::Append => {
                let mut combined = if exists { fs::read(&path)? } else { Vec::new() };
                combined.extend_from_slice(parsed.content.as_bytes());
                atomic_write(&path, &combined)?;
                None
            }
        };

        let mut metadata = Map::new();
        metadata.insert("path".to_owned(), json!(path.to_string_lossy().to_string()));
        metadata.insert("bytes_written".to_owned(), json!(parsed.content.len()));
        metadata.insert("mode_used".to_owned(), json!(parsed.mode.as_str()));
        metadata.insert("created_new".to_owned(), json!(!exists));
        if let Some(backup_path) = backup_path {
            metadata.insert(
                "backup_path".to_owned(),
                json!(backup_path.to_string_lossy().to_string()),
            );
        }

        Ok(ToolOutput {
            content: format!("wrote {} bytes to {}", parsed.content.len(), path.display()),
            success: true,
            metadata,
        })
    }
}

#[derive(Debug)]
struct WriteInput {
    path: PathBuf,
    content: String,
    mode: WriteMode,
    create_parents: bool,
}

impl WriteInput {
    fn parse(input: Value) -> Result<Self, ToolError> {
        let object = input.as_object().ok_or_else(|| {
            ToolError::InvalidInput("fs_write input must be an object".to_owned())
        })?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path is required".to_owned()))?;
        let content = object
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("content is required".to_owned()))?
            .to_owned();
        let mode = match object.get("mode").and_then(Value::as_str) {
            Some("overwrite") => WriteMode::Overwrite,
            Some("append") => WriteMode::Append,
            Some("create_only") | None => WriteMode::CreateOnly,
            Some(other) => {
                return Err(ToolError::InvalidInput(format!(
                    "unsupported write mode: {other}"
                )));
            }
        };
        let create_parents = match object.get("create_parents") {
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(ToolError::InvalidInput(
                    "create_parents must be a boolean".to_owned(),
                ));
            }
            None => false,
        };

        Ok(Self {
            path: PathBuf::from(path),
            content,
            mode,
            create_parents,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteMode {
    CreateOnly,
    Overwrite,
    Append,
}

impl WriteMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::CreateOnly => "create_only",
            Self::Overwrite => "overwrite",
            Self::Append => "append",
        }
    }
}

fn atomic_overwrite(path: &Path, bytes: &[u8]) -> Result<Option<PathBuf>, ToolError> {
    let backup = if path.exists() {
        let backup = backup_path(path)?;
        fs::copy(path, &backup)?;
        Some(backup)
    } else {
        None
    };
    atomic_write(path, bytes)?;
    Ok(backup)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    let parent = path
        .parent()
        .ok_or_else(|| ToolError::InvalidInput("path must have a parent".to_owned()))?;
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| ToolError::IoError(error.error))?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn backup_path(path: &Path) -> Result<PathBuf, ToolError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ToolError::Other(format!("system clock before unix epoch: {error}")))?
        .as_secs();
    Ok(PathBuf::from(format!(
        "{}.helm-backup-{timestamp}",
        path.to_string_lossy()
    )))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::tool::{Tool, ToolContext};

    use super::{FsWriteTool, WriteInput, backup_path};

    fn ctx_at(dir: &tempfile::TempDir) -> ToolContext {
        ToolContext::new(dir.path().to_path_buf())
    }

    #[tokio::test]
    async fn create_new_happy_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("new.txt");
        let output = FsWriteTool
            .execute(json!({"path": path, "content": "hello"}), &ctx_at(&dir))
            .await
            .unwrap();

        assert!(output.success);
        assert_eq!(fs::read_to_string(path).unwrap(), "hello");
    }

    #[tokio::test]
    async fn overwrite_creates_backup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        fs::write(&path, "old").unwrap();
        let output = FsWriteTool
            .execute(
                json!({"path": path, "content": "new", "mode": "overwrite"}),
                &ctx_at(&dir),
            )
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        let backup = output
            .metadata
            .get("backup_path")
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(fs::read_to_string(backup).unwrap(), "old");
    }

    #[tokio::test]
    async fn append_edge_case_new_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("append.txt");
        FsWriteTool
            .execute(
                json!({"path": path, "content": "tail", "mode": "append"}),
                &ctx_at(&dir),
            )
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(path).unwrap(), "tail");
    }

    #[tokio::test]
    async fn append_existing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("append.txt");
        fs::write(&path, "head").unwrap();
        FsWriteTool
            .execute(
                json!({"path": path, "content": "tail", "mode": "append"}),
                &ctx_at(&dir),
            )
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(path).unwrap(), "headtail");
    }

    #[tokio::test]
    async fn create_only_refuses_overwrite_error_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        fs::write(&path, "old").unwrap();
        let error = FsWriteTool
            .execute(json!({"path": path, "content": "new"}), &ctx_at(&dir))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("refusing"));
    }

    #[tokio::test]
    async fn parent_dirs_created_when_flagged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a/b/c.txt");
        FsWriteTool
            .execute(
                json!({"path": path, "content": "hello", "create_parents": true}),
                &ctx_at(&dir),
            )
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(path).unwrap(), "hello");
    }

    #[tokio::test]
    async fn denied_path_preserves_original_edge_case() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("credentials");
        fs::write(&path, "old").unwrap();
        let error = FsWriteTool
            .execute(
                json!({"path": path, "content": "new", "mode": "overwrite"}),
                &ctx_at(&dir),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("path denied"));
        assert_eq!(fs::read_to_string(path).unwrap(), "old");
    }

    #[test]
    fn parse_rejects_invalid_mode_error_path() {
        let error =
            WriteInput::parse(json!({"path": "/tmp/a", "content": "", "mode": "bad"})).unwrap_err();

        assert!(error.to_string().contains("unsupported"));
    }

    #[test]
    fn backup_path_has_expected_suffix_edge_case() {
        let path = backup_path(std::path::Path::new("/tmp/file")).unwrap();

        assert!(path.to_string_lossy().contains(".helm-backup-"));
    }
}
