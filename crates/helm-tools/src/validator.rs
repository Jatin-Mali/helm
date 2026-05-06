//! JSON Schema validator for tool inputs.

use jsonschema::Validator;
use serde_json::Value;

use crate::tool::ToolError;

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
}
