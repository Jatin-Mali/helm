use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Protected HELM state, including canonical aliases, must not be read or overwritten by tools.
pub fn is_protected_local_path(path: &std::path::Path) -> bool {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("helm");
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("helm");
    let resolve = |value: &std::path::Path| {
        value.canonicalize().unwrap_or_else(|_| {
            let parent = value.parent().unwrap_or(std::path::Path::new("."));
            let parent = parent
                .canonicalize()
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default().join(parent));
            parent.join(value.file_name().unwrap_or_default())
        })
    };
    let candidate = resolve(path);
    [
        config.join("secrets.toml"),
        config.join(".secrets.toml.lock"),
        data.join("helm.db"),
        data.join("helm.db-wal"),
        data.join("helm.db-shm"),
    ]
    .iter()
    .any(|protected| candidate == resolve(protected))
        || candidate.starts_with(resolve(&data.join("logs")))
}

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

/// JSON-backed store with rotation metadata.
///
/// Deprecated for provider credentials: the CLI's `secrets.toml` store is the designated,
/// advisory-locked home for those. Retired values are no longer retained here; only
/// rotation timestamps are kept, and legacy `history[].value` fields are scrubbed on the
/// next write.
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

        if let Some(entry) = data.get_mut(key)
            && let Some(obj) = entry.as_object_mut()
        {
            let rotated_at = obj.get("rotated_at").cloned();
            if let Some(hist) = obj.get_mut("history").and_then(|h| h.as_array_mut()) {
                hist.push(serde_json::json!({ "rotated_at": rotated_at }));
            }
            obj["value"] = serde_json::Value::String(new_value.to_string());
            obj["rotated_at"] = serde_json::Value::Number(now.into());
        }

        self.save_data(&data)?;
        Ok(())
    }

    /// Timestamps (unix seconds) of previous rotations; retired values are not kept.
    pub fn rotation_history(&self, key: &str) -> Result<Vec<i64>, SecretError> {
        let data = self.load_data()?;
        Ok(data
            .get(key)
            .and_then(|entry| entry.get("history"))
            .and_then(|hist| hist.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("rotated_at").and_then(|t| t.as_i64()))
                    .collect()
            })
            .unwrap_or_default())
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
            if let Some(rotated_at) = entry.get("rotated_at").and_then(|v| v.as_i64())
                && now - rotated_at > threshold_secs
            {
                needs_rotation.push(key.clone());
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

    /// Scrubs legacy retired values, then replaces the file atomically with mode 0600 from
    /// creation (never a window where secrets are world-readable).
    fn save_data(
        &self,
        data: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), SecretError> {
        use std::io::Write;
        let mut data = data.clone();
        for entry in data.values_mut() {
            if let Some(history) = entry.get_mut("history").and_then(|h| h.as_array_mut()) {
                for item in history {
                    if let Some(item) = item.as_object_mut() {
                        item.remove("value");
                    }
                }
            }
        }
        let json = serde_json::to_string_pretty(&serde_json::Value::Object(data))?;
        let tmp = self
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        let written = file
            .write_all(json.as_bytes())
            .and_then(|()| file.sync_all())
            .and_then(|()| std::fs::rename(&tmp, &self.path));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        Ok(written?)
    }
}

/// Redacts credentials and protected local-state paths from text bound for persistence,
/// exports, traces or providers. It is defence in depth: arbitrary secrets that carry no
/// recognisable marker cannot be found by pattern, so callers should still minimise data.
pub fn redact_secrets(input: &str) -> String {
    use std::sync::OnceLock;
    struct Rules {
        pem: regex::Regex,
        keys: regex::Regex,
        bearer: regex::Regex,
        url_credentials: regex::Regex,
        assignment: regex::Regex,
        path: regex::Regex,
    }
    static RULES: OnceLock<Rules> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        let build = |pattern: &str| {
            regex::Regex::new(pattern)
                .unwrap_or_else(|error| panic!("invalid built-in redaction regex: {error}"))
        };
        Rules {
            pem: build(
                r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)",
            ),
            keys: build(
                r"(?i)(sk-[a-zA-Z0-9_-]{20,}|gsk_[a-zA-Z0-9_-]{20,}|nvapi-[a-zA-Z0-9_-]{20,}|key-[a-zA-Z0-9_-]{20,}|AIza[0-9A-Za-z-_]{35}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|xox[abprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{4,})",
            ),
            bearer: build(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}"),
            url_credentials: build(r"([a-zA-Z][a-zA-Z0-9+.-]*://[^/\s:@]+):[^/\s@]+@"),
            // key=value, key: value and JSON "key": "value" for credential-looking names. A quoted
            // value is consumed whole (it may contain spaces); a bare one stops at punctuation.
            assignment: build(
                r#"(?i)(["']?[\w.-]*(?:password|passwd|pwd|passphrase|secret|token|api[_-]?key|apikey|credential|authorization|private[_-]?key)[\w.-]*["']?\s*[:=]\s*)("(?:[^"\\]|\\.)*"|'[^']*'|[^\s"',;&}\]]+)"#,
            ),
            // Resolved XDG or legacy locations of HELM's own protected state.
            path: build(
                r#"(?x)
                (?:~|(?:/[^\s"'`]+)+)
                /(?:\.helm|helm)/
                (?:secrets\.toml|\.secrets\.toml\.lock|audit\.key|helm\.db(?:-wal|-shm|\.execution\.lock)?
                  |helm\.log|logs/[^\s"'`/]*\.log|[^\s"'`/]*\.key)
                "#,
            ),
        }
    });
    let text = rules.pem.replace_all(input, "***REDACTED***");
    let text = rules.keys.replace_all(&text, "***REDACTED***");
    let text = rules.bearer.replace_all(&text, "$1 ***REDACTED***");
    let text = rules
        .url_credentials
        .replace_all(&text, "$1:***REDACTED***@");
    // Counters such as `max_tokens: 4096` are not credentials.
    let text = rules
        .assignment
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            let name = caps[1].trim_end_matches(|c: char| !c.is_alphanumeric());
            if name.to_ascii_lowercase().ends_with("tokens") {
                return caps[0].to_owned();
            }
            let quote = match caps[2].chars().next() {
                Some(q @ ('"' | '\'')) => q.to_string(),
                _ => String::new(),
            };
            format!("{}{quote}***REDACTED***{quote}", &caps[1])
        });
    rules
        .path
        .replace_all(&text, "[REDACTED_PATH]")
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
    fn redacts_resolved_xdg_state_paths_for_custom_roots() {
        for text in [
            "/srv/xdg/config/helm/secrets.toml",
            "/srv/xdg/config/helm/.secrets.toml.lock",
            "/var/lib/me/.local/share/helm/helm.db",
            "/var/lib/me/.local/share/helm/helm.db-wal",
            "/var/lib/me/.local/share/helm/logs/helm.log",
            "/var/lib/me/.local/share/helm/logs/helmopsd.log",
        ] {
            let redacted = redact_secrets(&format!("opened {text} ok"));
            assert_eq!(redacted, "opened [REDACTED_PATH] ok", "{text}");
        }
        assert_eq!(
            redact_secrets("/etc/helm/other.conf"),
            "/etc/helm/other.conf"
        );
    }

    #[test]
    fn redacts_arbitrary_credential_shapes() {
        for sentinel in [
            "PASSWORD=hunter2-sentinel",
            "db_password: hunter2-sentinel",
            r#"{"api_key": "hunter2-sentinel"}"#,
            "Authorization: Bearer hunter2sentinelvalue",
            "postgres://app:hunter2-sentinel@db.internal:5432/app",
            "aws AKIAABCDEFGHIJKLMNOP",
            "token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.sig_nature",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\n-----END OPENSSH PRIVATE KEY-----",
        ] {
            let redacted = redact_secrets(sentinel);
            for leaked in ["hunter2", "AKIAABC", "eyJhbGci", "ghp_abc", "b3BlbnNzaC"] {
                assert!(!redacted.contains(leaked), "{sentinel:?} -> {redacted:?}");
            }
            assert!(
                redacted.contains("REDACTED"),
                "{sentinel:?} -> {redacted:?}"
            );
        }
    }

    #[test]
    fn redacts_quoted_values_containing_spaces_and_escapes_without_breaking_json() {
        let json = r#"{"db_password": "my pass \"phrase\" here", "name": "web", "secret": 'x y'}"#;
        let redacted = redact_secrets(json);
        for leaked in ["my pass", "phrase", "here", "x y"] {
            assert!(!redacted.contains(leaked), "{redacted}");
        }
        assert!(redacted.contains(r#""name": "web""#), "{redacted}");
        let valid = r#"{"db_password": "a b c", "n": 1}"#;
        assert!(serde_json::from_str::<serde_json::Value>(&redact_secrets(valid)).is_ok());
    }

    #[test]
    fn redaction_keeps_ordinary_operational_text_and_is_idempotent() {
        let text = "max_tokens: 4096 unit sshd.service failed; host=web1 user:bob";
        assert_eq!(redact_secrets(text), text);
        let once = redact_secrets("password=abc123 url=https://u:p@h/x");
        assert_eq!(redact_secrets(&once), once);
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

    #[test]
    fn store_never_retains_retired_values_and_scrubs_legacy_history() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        // A pre-upgrade file that kept the retired plaintext value.
        std::fs::write(
            &path,
            r#"{"KEY":{"value":"current","created_at":1,"rotated_at":2,
               "history":[{"value":"retired-sentinel","rotated_at":1}]}}"#,
        )
        .unwrap();
        let store = SecretStore::new(path.clone());
        store.rotate("KEY", "newer").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("retired-sentinel") && !raw.contains("\"current\""),
            "{raw}"
        );
        assert_eq!(store.get("KEY").unwrap().as_deref(), Some("newer"));
        assert_eq!(store.rotation_history("KEY").unwrap().len(), 2);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
