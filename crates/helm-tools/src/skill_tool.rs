//! Adapts a skill (id + description + shell commands) to the `Tool` trait.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::tool::{Tool, ToolContext, ToolError, ToolOutput};

/// Wraps a skill's shell commands as an agent-callable tool.
///
/// `name()` and `description()` return `&'static str` because the `Tool` trait requires
/// it. Both strings are leaked at construction time — safe because skills are registered
/// once per process and never removed.
pub struct SkillTool {
    name: &'static str,
    description: &'static str,
    commands: Vec<String>,
}

impl SkillTool {
    /// Create a `SkillTool` from raw skill data.
    pub fn new(id: &str, description: &str, commands: Vec<String>) -> Self {
        let name: &'static str = Box::leak(format!("skill.{id}").into_boxed_str());
        let desc: &'static str = Box::leak(description.to_owned().into_boxed_str());
        Self {
            name,
            description: desc,
            commands,
        }
    }

    fn substitute(template: &str, input: &Value) -> String {
        let Some(obj) = input.as_object() else {
            return template.to_owned();
        };
        let mut out = template.to_owned();
        for (key, val) in obj {
            let placeholder = format!("{{{{{key}}}}}");
            let replacement = match val {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out = out.replace(&placeholder, &replacement);
        }
        out
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        self.description
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": { "type": "string" }
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let mut combined = String::new();
        let mut all_success = true;

        for template in &self.commands {
            let cmd = Self::substitute(template, &input);
            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .output()
                .await?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stdout.is_empty() {
                combined.push_str(&stdout);
            }
            if !stderr.is_empty() {
                combined.push_str(&stderr);
            }
            if !output.status.success() {
                all_success = false;
                combined.push_str(&format!(
                    "\n[exited with status {:?}]",
                    output.status.code()
                ));
            }
        }

        Ok(ToolOutput {
            content: combined,
            success: all_success,
            metadata: Map::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    #[test]
    fn name_has_skill_prefix() {
        let t = SkillTool::new("git-status", "desc", vec![]);
        assert_eq!(t.name(), "skill.git-status");
    }

    #[test]
    fn description_is_preserved() {
        let t = SkillTool::new("x", "my description", vec![]);
        assert_eq!(t.description(), "my description");
    }

    #[test]
    fn substitution_replaces_placeholders() {
        let result = SkillTool::substitute("echo {{msg}}", &json!({"msg": "hi"}));
        assert_eq!(result, "echo hi");
    }

    #[test]
    fn substitution_noop_when_no_placeholders() {
        let result = SkillTool::substitute("ls -la", &json!({}));
        assert_eq!(result, "ls -la");
    }

    #[test]
    fn substitution_handles_missing_key() {
        let result = SkillTool::substitute("echo {{missing}}", &json!({}));
        assert_eq!(result, "echo {{missing}}");
    }

    #[tokio::test]
    async fn execute_git_status_happy_path() {
        let t = SkillTool::new(
            "git-status",
            "Show git status",
            vec!["git status --short".to_owned()],
        );
        let ctx = ToolContext::new(PathBuf::from("."));
        let out = t.execute(json!({}), &ctx).await.unwrap();
        assert!(out.success, "git status failed: {}", out.content);
    }

    #[tokio::test]
    async fn execute_with_substitution() {
        let t = SkillTool::new(
            "echo-test",
            "Echo input",
            vec!["echo {{message}}".to_owned()],
        );
        let ctx = ToolContext::new(PathBuf::from("."));
        let out = t
            .execute(json!({"message": "hello-world"}), &ctx)
            .await
            .unwrap();
        assert!(out.success);
        assert!(out.content.contains("hello-world"));
    }
}
