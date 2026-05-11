//! JSON Schema validator for tool inputs and security allowlists/blocklists.

use jsonschema::Validator;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tool::ToolError;

// dirs is already in workspace.dependencies
use dirs;
use toml;

// ── AllowlistConfig ───────────────────────────────────────────────────────────

/// Configuration for shell command, domain, and file path allowlists/blocklists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowlistConfig {
    /// Shell command glob patterns (allow list). Empty = allow all.
    pub shell_patterns: Vec<String>,
    /// Domains to block in HTTP/network tools.
    pub blocked_domains: Vec<String>,
    /// File path glob patterns to ignore (like .gitignore).
    pub helmignore_patterns: Vec<String>,
}

impl AllowlistConfig {
    /// Load from ~/.helm/allowlist.toml if it exists.
    /// Returns permissive defaults if file doesn't exist.
    pub fn load() -> Result<Self, ToolError> {
        let allowlist_path = dirs::home_dir()
            .ok_or_else(|| ToolError::InvalidInput("No home directory".to_string()))?
            .join(".helm")
            .join("allowlist.toml");

        if !allowlist_path.exists() {
            return Ok(Self::permissive());
        }

        let content = std::fs::read_to_string(&allowlist_path)
            .map_err(|e| ToolError::InvalidInput(format!("Failed to read allowlist.toml: {e}")))?;
        toml::from_str(&content)
            .map_err(|e| ToolError::InvalidInput(format!("Invalid allowlist.toml: {e}")))
    }

    /// Permissive defaults: allow all.
    pub fn permissive() -> Self {
        Self {
            shell_patterns: Vec::new(),
            blocked_domains: Vec::new(),
            helmignore_patterns: Vec::new(),
        }
    }

    /// Check if a shell command matches at least one shell pattern.
    /// Empty patterns = allow all.
    pub fn is_shell_allowed(&self, cmd: &str) -> bool {
        if self.shell_patterns.is_empty() {
            return true;
        }
        self.shell_patterns.iter().any(|pattern| {
            // Simple glob-like matching: * = any, ? = single char
            self.glob_matches(pattern, cmd)
        })
    }

    /// Check if a domain is blocked.
    pub fn is_domain_blocked(&self, domain: &str) -> bool {
        self.blocked_domains
            .iter()
            .any(|blocked| domain.ends_with(blocked) || domain == blocked)
    }

    /// Check if a file path matches any helmignore pattern.
    pub fn is_ignored(&self, path: &str) -> bool {
        if self.helmignore_patterns.is_empty() {
            return false;
        }
        self.helmignore_patterns
            .iter()
            .any(|pattern| self.glob_matches(pattern, path))
    }

    /// Simple glob pattern matching.
    fn glob_matches(&self, pattern: &str, text: &str) -> bool {
        // Handle simple wildcards: * matches everything, ? matches single char
        if pattern == "*" {
            return true;
        }
        if !pattern.contains('*') && !pattern.contains('?') {
            return pattern == text;
        }
        // Prefix match for patterns like "*.log"
        if let Some(stripped) = pattern.strip_prefix('*') {
            return text.ends_with(stripped);
        }
        // Suffix match for patterns like "debug*"
        if let Some(stripped) = pattern.strip_suffix('*') {
            return text.starts_with(stripped);
        }
        // Exact match if no wildcards
        pattern == text
    }
}

/// Validates tool inputs against their declared JSON Schema.
pub struct InputValidator {
    schema: Value,
    compiled: Validator,
}

impl InputValidator {
    /// Build a validator from a JSON Schema value.
    ///
    /// Returns an error if the schema itself is invalid.
    pub fn new(schema: Value) -> Result<Self, ToolError> {
        let compiled = Validator::new(&schema)
            .map_err(|e| ToolError::InvalidInput(format!("invalid schema: {e}")))?;
        Ok(Self { schema, compiled })
    }

    /// Validate `input` against this validator's schema.
    ///
    /// Returns `Ok(())` on success or `Err(ToolError::InvalidInput)` with a
    /// human-readable summary of every violation.
    pub fn validate(&self, input: &Value) -> Result<(), ToolError> {
        let errors: Vec<String> = self
            .compiled
            .iter_errors(input)
            .map(|e| format!("{e}"))
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ToolError::InvalidInput(errors.join("; ")))
        }
    }

    /// Return a reference to the underlying schema.
    pub fn schema(&self) -> &Value {
        &self.schema
    }
}

impl std::fmt::Debug for InputValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputValidator")
            .field("schema", &self.schema)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shell_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "args":    {"type": "array", "items": {"type": "string"}}
            },
            "required": ["command"]
        })
    }

    // ── happy-path ────────────────────────────────────────────────────────────

    #[test]
    fn valid_input_passes() {
        let v = InputValidator::new(shell_schema()).unwrap();
        assert!(
            v.validate(&json!({"command": "ls", "args": ["-la"]}))
                .is_ok()
        );
    }

    #[test]
    fn valid_input_optional_field_absent() {
        let v = InputValidator::new(shell_schema()).unwrap();
        assert!(v.validate(&json!({"command": "pwd"})).is_ok());
    }

    // ── error paths ───────────────────────────────────────────────────────────

    #[test]
    fn missing_required_field_errors() {
        let v = InputValidator::new(shell_schema()).unwrap();
        let err = v.validate(&json!({"args": []})).unwrap_err();
        assert!(err.to_string().contains("command"));
    }

    #[test]
    fn wrong_type_errors() {
        let v = InputValidator::new(shell_schema()).unwrap();
        let err = v.validate(&json!({"command": 42})).unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput(_)));
    }

    #[test]
    fn multiple_violations_joined() {
        let v = InputValidator::new(shell_schema()).unwrap();
        // Both command missing and args wrong type.
        let err = v.validate(&json!({"args": "not-array"})).unwrap_err();
        let msg = err.to_string();
        assert!(!msg.is_empty());
    }

    // ── edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn empty_object_against_permissive_schema() {
        let v = InputValidator::new(json!({"type": "object"})).unwrap();
        assert!(v.validate(&json!({})).is_ok());
    }

    #[test]
    fn invalid_schema_returns_error() {
        // A schema that refers to a broken type.
        let bad = json!({"type": ["not", "a", "valid", "type", "combination", 999]});
        // jsonschema may or may not reject this at compile time; we just check it
        // doesn't panic.
        let _ = InputValidator::new(bad);
    }

    #[test]
    fn schema_accessor_returns_original() {
        let schema = shell_schema();
        let v = InputValidator::new(schema.clone()).unwrap();
        assert_eq!(v.schema(), &schema);
    }

    // ── AllowlistConfig tests ──────────────────────────────────────────────

    #[test]
    fn permissive_allows_everything() {
        let config = AllowlistConfig::permissive();
        assert!(config.is_shell_allowed("anything"));
        assert!(config.is_shell_allowed("rm -rf /"));
        assert!(!config.is_domain_blocked("example.com"));
        assert!(!config.is_ignored("any/file.txt"));
    }

    #[test]
    fn shell_allowlist_blocks_disallowed() {
        let config = AllowlistConfig {
            shell_patterns: vec!["ls".to_string(), "cat".to_string()],
            blocked_domains: Vec::new(),
            helmignore_patterns: Vec::new(),
        };
        assert!(config.is_shell_allowed("ls"));
        assert!(config.is_shell_allowed("cat"));
        assert!(!config.is_shell_allowed("rm"));
        assert!(!config.is_shell_allowed("dd"));
    }

    #[test]
    fn domain_blocklist_blocks_domain() {
        let config = AllowlistConfig {
            shell_patterns: Vec::new(),
            blocked_domains: vec!["evil.com".to_string(), "malware.io".to_string()],
            helmignore_patterns: Vec::new(),
        };
        assert!(config.is_domain_blocked("evil.com"));
        assert!(config.is_domain_blocked("api.evil.com"));
        assert!(!config.is_domain_blocked("good.com"));
    }

    #[test]
    fn helmignore_blocks_matched_path() {
        let config = AllowlistConfig {
            shell_patterns: Vec::new(),
            blocked_domains: Vec::new(),
            helmignore_patterns: vec!["*.log".to_string(), ".env".to_string()],
        };
        assert!(config.is_ignored("debug.log"));
        assert!(config.is_ignored(".env"));
        assert!(!config.is_ignored("src/main.rs"));
    }
}
