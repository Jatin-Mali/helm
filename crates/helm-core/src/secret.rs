use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Secrets rotation policy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationPolicy {
    /// Number of days before a secret should be rotated.
    pub interval_days: u32,
    /// Number of days before rotation deadline to show a warning.
    pub alert_before_days: u32,
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            interval_days: 30,
            alert_before_days: 5,
        }
    }
}

/// Errors related to secrets storage.
#[derive(Debug, Error)]
pub enum SecretError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("secret not found: {0}")]
    NotFound(String),
}

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

/// Store for secret rotation history (JSON-backed).
pub struct SecretStore {
    path: std::path::PathBuf,
}

impl SecretStore {
    /// Create a new secret store at the given path.
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    /// Set a secret value in the store.
    pub fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        let mut data = self.load_data()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if !data.contains_key(key) {
            data.insert(
                key.to_string(),
                serde_json::json!({
                    "value": value,
                    "created_at": now,
                    "rotated_at": now,
                    "history": []
                }),
            );
        } else if let Some(obj) = data.get_mut(key).and_then(|v| v.as_object_mut()) {
            obj["value"] = serde_json::Value::String(value.to_string());
            obj["rotated_at"] = serde_json::Value::Number(now.into());
        }

        self.save_data(&data)?;
        Ok(())
    }

    /// Get a secret value from the store.
    pub fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        let data = self.load_data()?;
        Ok(data
            .get(key)
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()))
    }

    /// Delete a secret from the store.
    pub fn delete(&self, key: &str) -> Result<(), SecretError> {
        let mut data = self.load_data()?;
        data.remove(key);
        self.save_data(&data)?;
        Ok(())
    }

    /// List all secret keys in the store.
    pub fn list(&self) -> Result<Vec<String>, SecretError> {
        let data = self.load_data()?;
        Ok(data.keys().cloned().collect())
    }

    /// Rotate a secret (replace with new value, keep history).
    pub fn rotate(&self, key: &str, new_value: &str) -> Result<(), SecretError> {
        let mut data = self.load_data()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if let Some(entry) = data.get_mut(key) {
            if let Some(obj) = entry.as_object_mut() {
                let old_val = obj.get("value").cloned();
                let rotated_at = obj.get("rotated_at").cloned();

                if let Some(old_v) = old_val {
                    if let Some(hist) = obj.get_mut("history").and_then(|h| h.as_array_mut()) {
                        hist.push(serde_json::json!({
                            "value": old_v,
                            "rotated_at": rotated_at
                        }));
                    }
                }
                obj["value"] = serde_json::Value::String(new_value.to_string());
                obj["rotated_at"] = serde_json::Value::Number(now.into());
            }
        }

        self.save_data(&data)?;
        Ok(())
    }

    /// Get rotation history for a secret key.
    pub fn rotation_history(&self, key: &str) -> Result<Vec<(String, i64)>, SecretError> {
        let data = self.load_data()?;
        if let Some(entry) = data.get(key) {
            if let Some(hist) = entry.get("history") {
                if let Some(arr) = hist.as_array() {
                    let mut results = Vec::new();
                    for item in arr {
                        if let (Some(val), Some(ts)) = (
                            item.get("value").and_then(|v| v.as_str()),
                            item.get("rotated_at").and_then(|t| t.as_i64()),
                        ) {
                            results.push((val.to_string(), ts));
                        }
                    }
                    return Ok(results);
                }
            }
        }
        Ok(Vec::new())
    }

    /// Check which keys need rotation based on the policy.
    pub fn check_rotation_needed(
        &self,
        policy: &RotationPolicy,
    ) -> Result<Vec<String>, SecretError> {
        let data = self.load_data()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let threshold_secs = (policy.interval_days as i64) * 86400;

        let mut needs_rotation = Vec::new();
        for (key, entry) in data.iter() {
            if let Some(rotated_at) = entry.get("rotated_at").and_then(|v| v.as_i64()) {
                if now - rotated_at > threshold_secs {
                    needs_rotation.push(key.clone());
                }
            }
        }
        Ok(needs_rotation)
    }

    fn load_data(&self) -> Result<serde_json::Map<String, serde_json::Value>, SecretError> {
        if self.path.exists() {
            let content = std::fs::read_to_string(&self.path)?;
            let data: serde_json::Value = serde_json::from_str(&content)?;
            Ok(data.as_object().cloned().unwrap_or_default())
        } else {
            Ok(serde_json::Map::new())
        }
    }

    fn save_data(
        &self,
        data: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), SecretError> {
        let json = serde_json::to_string_pretty(&serde_json::Value::Object(data.clone()))?;
        std::fs::write(&self.path, json)?;
        #[cfg(unix)]
        {
            use std::fs;
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&self.path, perms)?;
        }
        Ok(())
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

    #[test]
    fn secret_store_set_get() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.json"));
        store.set("KEY", "value1").unwrap();
        assert_eq!(store.get("KEY").unwrap(), Some("value1".to_string()));
    }

    #[test]
    fn secret_store_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.json"));
        store.set("KEY", "value1").unwrap();
        store.delete("KEY").unwrap();
        assert_eq!(store.get("KEY").unwrap(), None);
    }

    #[test]
    fn secret_store_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.json"));
        store.set("KEY1", "value1").unwrap();
        store.set("KEY2", "value2").unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&"KEY1".to_string()));
        assert!(list.contains(&"KEY2".to_string()));
    }

    #[test]
    fn secret_store_rotate() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.json"));
        store.set("KEY", "value1").unwrap();
        assert_eq!(store.get("KEY").unwrap(), Some("value1".to_string()));
        store.rotate("KEY", "value2").unwrap();
        assert_eq!(store.get("KEY").unwrap(), Some("value2".to_string()));
        let hist = store.rotation_history("KEY").unwrap();
        assert_eq!(hist.len(), 1);
    }
}
