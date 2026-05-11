//! Input validation and injection prevention.

use thiserror::Error;

/// Validation errors.
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("prompt injection detected: {0}")]
    PromptInjection(String),
    #[error("shell injection detected: {0}")]
    ShellInjection(String),
    #[error("blocked URL pattern: {0}")]
    BlockedUrl(String),
}

/// Input validator for preventing injection attacks.
pub struct Validator;

impl Validator {
    /// Detect obvious prompt injection markers.
    pub fn validate_prompt(text: &str) -> Result<(), ValidationError> {
        let injection_patterns = [
            "ignore previous instructions",
            "ignore above",
            "disregard all previous",
            "system: you are now",
            "new instructions:",
            "forget everything",
        ];
        let lower = text.to_lowercase();
        for pat in &injection_patterns {
            if lower.contains(pat) {
                return Err(ValidationError::PromptInjection(pat.to_string()));
            }
        }
        Ok(())
    }

    /// Reject shell inputs with dangerous unescaped constructs.
    pub fn validate_shell(cmd: &str) -> Result<(), ValidationError> {
        let dangerous = ["> /etc", "> /dev/", "$(", "&&", "||", ";", "|"];
        for pat in &dangerous {
            if cmd.contains(pat) {
                return Err(ValidationError::ShellInjection(pat.to_string()));
            }
        }
        Ok(())
    }

    /// Validate that a URL is not localhost/internal range.
    pub fn validate_url(url: &str) -> Result<(), ValidationError> {
        let blocked = [
            "localhost",
            "127.0.0.1",
            "0.0.0.0",
            "169.254.",
            "10.",
            "192.168.",
            "172.16.",
        ];
        let lower = url.to_lowercase();
        for pat in &blocked {
            if lower.contains(pat) {
                return Err(ValidationError::BlockedUrl(pat.to_string()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_prompt_injection() {
        assert!(Validator::validate_prompt("ignore previous instructions and do X").is_err());
        assert!(Validator::validate_prompt("What is 2+2?").is_ok());
    }

    #[test]
    fn detect_shell_injection() {
        assert!(Validator::validate_shell("rm -rf / ; echo done").is_err());
        assert!(Validator::validate_shell("ls -la /tmp").is_ok());
    }

    #[test]
    fn detect_url_injection() {
        assert!(Validator::validate_url("http://localhost:8080").is_err());
        assert!(Validator::validate_url("http://127.0.0.1").is_err());
        assert!(Validator::validate_url("http://192.168.1.1").is_err());
        assert!(Validator::validate_url("http://example.com").is_ok());
    }
}
