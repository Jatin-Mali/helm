use secrecy::{ExposeSecret, SecretString};

/// A string that must never appear in logs, debug output, or audit entries.
#[derive(Clone)]
pub struct Secret(SecretString);

impl Secret {
    /// Wraps a raw secret value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(SecretString::from(s.into()))
    }

    /// Returns the raw secret value.
    ///
    /// Use this only at the boundary where the value is actually needed, such as
    /// HTTP header construction or explicit `helm secrets get` output.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    /// Returns whether the wrapped string is empty.
    pub fn is_empty(&self) -> bool {
        self.expose().is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret(***{}chars)", self.expose().len())
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// Redacts known API key patterns from a string.
pub fn redact_secrets(input: &str) -> String {
    use std::sync::OnceLock;
    static KEY_RE: OnceLock<regex::Regex> = OnceLock::new();
    static PATH_RE: OnceLock<regex::Regex> = OnceLock::new();
    let key_re = KEY_RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(sk-[a-zA-Z0-9_-]{20,}|gsk_[a-zA-Z0-9_-]{20,}|nvapi-[a-zA-Z0-9_-]{20,}|key-[a-zA-Z0-9_-]{20,}|AIza[0-9A-Za-z-_]{35})",
        )
        .unwrap_or_else(|error| panic!("invalid built-in secret redaction regex: {error}"))
    });
    let path_re = PATH_RE.get_or_init(|| {
        regex::Regex::new(
            r#"(?x)
            (?:~|(?:/[^\s"'`]+)+)
            /\.helm/
            (?:secrets\.toml|\.secrets\.toml\.lock|helm\.db|helm\.log)
            "#,
        )
        .unwrap_or_else(|error| panic!("invalid built-in path redaction regex: {error}"))
    });
    let redacted_keys = key_re.replace_all(input, "***REDACTED***").into_owned();
    path_re
        .replace_all(&redacted_keys, "[REDACTED_PATH]")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_leaks_value() {
        let s = Secret::new("super-secret-key-12345");
        let debug = format!("{:?}", s);
        assert!(!debug.contains("super-secret-key-12345"));
        assert!(debug.contains("***"));
    }

    #[test]
    fn expose_returns_real_value() {
        let s = Secret::new("my-api-key");
        assert_eq!(s.expose(), "my-api-key");
    }

    #[test]
    fn clone_works() {
        let s = Secret::new("cloneable");
        let c = s.clone();
        assert_eq!(c.expose(), "cloneable");
    }

    #[test]
    fn is_empty_works() {
        assert!(Secret::new("").is_empty());
        assert!(!Secret::new("x").is_empty());
    }

    #[test]
    fn from_string_and_str() {
        let s1: Secret = "from_str".into();
        let s2: Secret = "from_string".to_owned().into();
        assert_eq!(s1.expose(), "from_str");
        assert_eq!(s2.expose(), "from_string");
    }

    #[test]
    fn redacts_provider_keys() {
        let text = "Bearer sk-or-v1-abcdefghijklmnopqrstuvwxyz123456";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyz123456"));
        assert!(redacted.contains("***REDACTED***"));
    }

    #[test]
    fn redacts_local_helm_state_paths() {
        let text = "read /home/test/.helm/secrets.toml and ~/.helm/helm.db";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains(".helm/secrets.toml"));
        assert!(!redacted.contains(".helm/helm.db"));
        assert_eq!(redacted.matches("[REDACTED_PATH]").count(), 2);
    }
}
