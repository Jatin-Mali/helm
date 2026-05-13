//! Search tool: grep and glob via ignore-aware traversal.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use async_trait::async_trait;
use globset::Glob;
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::{
    fs_read::validate_canonical_path,
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

pub struct SearchTool;

impl SearchTool {
    fn run_grep(
        &self,
        path: &PathBuf,
        pattern: &str,
        file_pattern: Option<&str>,
        max_matches: usize,
    ) -> Result<String, ToolError> {
        let re =
            Regex::new(pattern).map_err(|e| ToolError::Other(format!("invalid regex: {e}")))?;

        let fp_re: Option<Regex> = if let Some(fp) = file_pattern {
            Some(
                Regex::new(fp)
                    .map_err(|e| ToolError::Other(format!("invalid file pattern: {e}")))?,
            )
        } else {
            None
        };

        let walk = WalkBuilder::new(path)
            .hidden(true)
            .ignore(true)
            .git_ignore(true)
            .add_custom_ignore_filename(".helmignore")
            .build();

        let mut results = Vec::new();

        for entry in walk.filter_map(|e| e.ok()) {
            let entry_path = entry.path();
            if !entry_path.is_file() {
                continue;
            }
            let fp_matches = match &fp_re {
                Some(re_fp) => {
                    let name = entry_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    re_fp.is_match(name)
                }
                None => true,
            };
            if !fp_matches {
                continue;
            }

            let file_results = self.grep_file(entry_path, &re, max_matches - results.len());
            results.extend(file_results);

            if results.len() >= max_matches {
                break;
            }
        }

        if results.is_empty() {
            return Ok(String::new());
        }

        let output = results.join("\n");
        if results.len() >= max_matches {
            Ok(format!(
                "{}...\n(truncated at {} matches)",
                output, max_matches
            ))
        } else {
            Ok(output)
        }
    }

    fn grep_file(&self, path: &std::path::Path, re: &Regex, max: usize) -> Vec<String> {
        let mut results = Vec::new();
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if re.is_match(&line) {
                    results.push(format!("{}:{}", path.display(), line));
                    if results.len() >= max {
                        break;
                    }
                }
            }
        }
        results
    }

    fn run_glob(
        &self,
        path: &PathBuf,
        pattern: &str,
        max_files: usize,
    ) -> Result<String, ToolError> {
        let walk = WalkBuilder::new(path)
            .hidden(true)
            .ignore(true)
            .git_ignore(true)
            .add_custom_ignore_filename(".helmignore")
            .build();

        let globber = Glob::new(pattern)
            .map_err(|e| ToolError::Other(format!("invalid glob: {e}")))?
            .compile_matcher();

        let mut matches = Vec::new();
        for entry in walk.filter_map(|e| e.ok()) {
            if globber.is_match(entry.path()) {
                matches.push(entry.path().display().to_string());
                if matches.len() >= max_files {
                    break;
                }
            }
        }

        Ok(matches.join("\n"))
    }
}

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &'static str {
        "search"
    }

    fn description(&self) -> &'static str {
        "File search: grep (regex search with context) and glob (pattern match)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": { "type": "string", "enum": ["grep", "glob"] },
                "path": { "type": "string", "description": "Directory to search in (defaults to tool working dir)" },
                "pattern": { "type": "string", "description": "Regex for grep, glob pattern for glob" },
                "file_pattern": { "type": "string", "description": "Optional file name regex filter for grep" },
                "max_matches": { "type": "integer", "description": "Maximum matches to return", "default": 100 }
            },
            "required": ["action", "pattern"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = input
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("action required".into()))?;

        let path = input
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());
        let search_root = if path.is_absolute() {
            path
        } else {
            ctx.working_dir.join(path)
        };
        let canonical_root = search_root.canonicalize().map_err(ToolError::IoError)?;
        validate_canonical_path(&canonical_root, ctx)?;

        let pattern = input
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("pattern required".into()))?;

        let max = input
            .get("max_matches")
            .and_then(Value::as_u64)
            .unwrap_or(100) as usize;

        let file_pattern = input.get("file_pattern").and_then(Value::as_str);

        let content = match action {
            "grep" => self.run_grep(&canonical_root, pattern, file_pattern, max)?,
            "glob" => self.run_glob(&canonical_root, pattern, max)?,
            _ => {
                return Err(ToolError::InvalidInput(format!(
                    "unsupported action: {action}"
                )));
            }
        };

        let mut metadata = Map::new();
        metadata.insert("path".into(), json!(canonical_root.to_string_lossy()));
        metadata.insert("matches".into(), json!(content.lines().count()));

        Ok(ToolOutput {
            content,
            success: true,
            metadata,
        })
    }

    fn allowed_in_diagnose(&self) -> bool {
        true
    }

    fn all_write_ops_gated_in_diagnose(&self) -> bool {
        true // search has only read-only actions (grep, glob)
    }
}

impl Default for SearchTool {
    fn default() -> Self {
        Self
    }
}

#[cfg(test)]
#[allow(clippy::default_constructed_unit_structs)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use crate::tool::{Tool, ToolContext};

    use super::SearchTool;

    #[tokio::test]
    async fn grep_finds_pattern() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello world\nfoo bar\nhello again\n").unwrap();

        let tool = SearchTool;
        let result = tool
            .execute(
                json!({"action": "grep", "path": dir.path().to_str().unwrap(), "pattern": "hello"}),
                &ToolContext::new(dir.path().into()),
            )
            .await
            .unwrap();

        assert!(result.content.contains("hello"));
        assert!(result.content.contains("world"));
    }

    #[tokio::test]
    async fn glob_matches_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("foo.rs"), "").unwrap();
        fs::write(dir.path().join("bar.rs"), "").unwrap();
        fs::write(dir.path().join("baz.txt"), "").unwrap();

        let tool = SearchTool;
        let result = tool
            .execute(
                json!({"action": "glob", "path": dir.path().to_str().unwrap(), "pattern": "*.rs"}),
                &ToolContext::new(dir.path().into()),
            )
            .await
            .unwrap();

        assert!(result.content.contains("foo.rs"));
        assert!(result.content.contains("bar.rs"));
        assert!(!result.content.contains("baz.txt"));
    }
}
