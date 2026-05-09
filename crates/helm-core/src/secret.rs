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
}
