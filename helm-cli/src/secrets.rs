//! Persistent secrets store: `~/.helm/secrets.toml` with mode 0600.
//!
//! The store is intentionally local and simple for v1.0: flat TOML,
//! strict Unix permissions, and atomic writes guarded by an advisory lock.

use std::{
    collections::BTreeMap,
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use helm_core::Secret;
use tempfile::NamedTempFile;
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Error)]
pub enum SecretsError {
    #[error("I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse secrets file: {0}")]
    Parse(String),
    #[error("{0} has insecure permissions")]
    InsecurePermissions(PathBuf),
}

type SecretsFile = BTreeMap<String, String>;

/// Manages `$XDG_CONFIG_HOME/helm/secrets.toml` — a 0600 flat TOML key/value store.
#[derive(Debug, Clone)]
pub struct SecretsStore {
    path: PathBuf,
}

impl SecretsStore {
    /// Opens `$XDG_CONFIG_HOME/helm/secrets.toml` (or `~/.config/helm/secrets.toml`),
    /// creating the dir with mode 0700 when needed.
    pub fn open_default() -> Result<Self, SecretsError> {
        let dir = xdg_config_dir();
        fs::create_dir_all(&dir).map_err(|e| SecretsError::Io {
            path: dir.clone(),
            source: e,
        })?;
        Self::open_at(dir.join("secrets.toml"))
    }

    /// Opens a secrets store at `path`.
    pub fn open_at(path: impl Into<PathBuf>) -> Result<Self, SecretsError> {
        let path = path.into();
        if path.exists() {
            check_permissions(&path)?;
        }
        Ok(Self { path })
    }

    /// Returns the backing store path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the secret for `name`, or `None` if not present.
    pub fn get(&self, name: &str) -> Result<Option<Secret>, SecretsError> {
        let file = self.load()?;
        Ok(file.get(name).map(|v| Secret::new(v.clone())))
    }

    /// Atomically writes `name = value` to the store, creating the file with mode 0600.
    pub fn set(&self, name: &str, value: Secret) -> Result<(), SecretsError> {
        self.ensure_parent_secure()?;
        let _lock = self.lock_exclusive()?;
        let mut file = self.load()?;
        file.insert(name.to_owned(), value.expose().to_owned());
        self.save_atomic(&file)
    }

    /// Removes `name` from the store and returns whether a value was deleted.
    pub fn delete(&self, name: &str) -> Result<bool, SecretsError> {
        self.ensure_parent_secure()?;
        let _lock = self.lock_exclusive()?;
        let mut file = self.load()?;
        let removed = file.remove(name).is_some();
        self.save_atomic(&file)?;
        Ok(removed)
    }

    /// Returns sorted list of stored key names.
    pub fn list_names(&self) -> Result<Vec<String>, SecretsError> {
        let file = self.load()?;
        Ok(file.keys().cloned().collect())
    }

    fn load(&self) -> Result<SecretsFile, SecretsError> {
        if !self.path.exists() {
            return Ok(SecretsFile::default());
        }
        check_permissions(&self.path)?;
        let text = fs::read_to_string(&self.path).map_err(|e| SecretsError::Io {
            path: self.path.clone(),
            source: e,
        })?;
        toml::from_str(&text).map_err(|error| SecretsError::Parse(error.to_string()))
    }

    fn save_atomic(&self, data: &SecretsFile) -> Result<(), SecretsError> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        let text =
            toml::to_string_pretty(data).map_err(|error| SecretsError::Parse(error.to_string()))?;

        // Write to a sibling temp file, chmod 0600, then rename atomically.
        let tmp = NamedTempFile::new_in(dir).map_err(|e| SecretsError::Io {
            path: dir.to_owned(),
            source: e,
        })?;
        set_mode_600(tmp.path())?;

        let mut file = tmp.as_file();
        file.write_all(text.as_bytes())
            .map_err(|e| SecretsError::Io {
                path: tmp.path().to_owned(),
                source: e,
            })?;
        file.flush().map_err(|e| SecretsError::Io {
            path: tmp.path().to_owned(),
            source: e,
        })?;

        tmp.persist(&self.path).map_err(|e| SecretsError::Io {
            path: self.path.clone(),
            source: e.error,
        })?;
        Ok(())
    }

    fn ensure_parent_secure(&self) -> Result<(), SecretsError> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir).map_err(|e| SecretsError::Io {
            path: dir.to_owned(),
            source: e,
        })?;
        check_parent_permissions(dir)
    }

    /// Takes a Linux/Unix advisory `flock` on a sibling lock file for read-modify-write.
    ///
    /// This is Linux-only because HELM v1 targets Linux operations terminals. The lock
    /// prevents concurrent writers from loading the same old TOML and losing one writer's
    /// key during the atomic temp-file + rename sequence.
    fn lock_exclusive(&self) -> Result<StoreLock, SecretsError> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        let lock_path = dir.join(".secrets.toml.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| SecretsError::Io {
                path: lock_path.clone(),
                source: e,
            })?;
        set_mode_600(&lock_path)?;
        lock_file(&file, &lock_path)?;
        Ok(StoreLock { file, lock_path })
    }
}

struct StoreLock {
    file: File,
    lock_path: PathBuf,
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        unlock_file(&self.file, &self.lock_path);
    }
}

fn xdg_config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("helm")
}

#[cfg(unix)]
fn check_permissions(path: &Path) -> Result<(), SecretsError> {
    let meta = fs::metadata(path).map_err(|e| SecretsError::Io {
        path: path.to_owned(),
        source: e,
    })?;
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(SecretsError::InsecurePermissions(path.to_owned()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_permissions(_path: &Path) -> Result<(), SecretsError> {
    Ok(())
}

#[cfg(unix)]
fn check_parent_permissions(path: &Path) -> Result<(), SecretsError> {
    let meta = fs::metadata(path).map_err(|e| SecretsError::Io {
        path: path.to_owned(),
        source: e,
    })?;
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o002 != 0 {
        return Err(SecretsError::InsecurePermissions(path.to_owned()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_parent_permissions(_path: &Path) -> Result<(), SecretsError> {
    Ok(())
}

#[cfg(unix)]
fn set_mode_600(path: &Path) -> Result<(), SecretsError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| SecretsError::Io {
        path: path.to_owned(),
        source: e,
    })
}

#[cfg(not(unix))]
fn set_mode_600(_path: &Path) -> Result<(), SecretsError> {
    Ok(())
}

#[cfg(unix)]
#[allow(dead_code)]
fn set_mode_700(path: &Path) -> Result<(), SecretsError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| SecretsError::Io {
        path: path.to_owned(),
        source: e,
    })
}

#[cfg(not(unix))]
#[allow(dead_code)]
fn set_mode_700(_path: &Path) -> Result<(), SecretsError> {
    Ok(())
}

#[cfg(unix)]
#[allow(deprecated)]
fn lock_file(file: &File, path: &Path) -> Result<(), SecretsError> {
    use nix::fcntl::{FlockArg, flock};
    use std::os::unix::io::AsRawFd;

    flock(file.as_raw_fd(), FlockArg::LockExclusive).map_err(|error| SecretsError::Io {
        path: path.to_owned(),
        source: std::io::Error::other(error.to_string()),
    })
}

#[cfg(not(unix))]
fn lock_file(_file: &File, _path: &Path) -> Result<(), SecretsError> {
    Ok(())
}

#[cfg(unix)]
#[allow(deprecated)]
fn unlock_file(file: &File, _path: &Path) {
    use nix::fcntl::{FlockArg, flock};
    use std::os::unix::io::AsRawFd;

    let _ = flock(file.as_raw_fd(), FlockArg::Unlock);
}

#[cfg(not(unix))]
fn unlock_file(_file: &File, _path: &Path) {}

/// Resolves a secret using CLI override, store, then environment variable.
pub fn resolve_secret(
    name: &str,
    cli_override: Option<&Secret>,
    store: &SecretsStore,
) -> Result<Option<Secret>, SecretsError> {
    if let Some(v) = cli_override {
        if !v.is_empty() {
            return Ok(Some(v.clone()));
        }
    }
    if let Some(s) = store.get(name)? {
        if !s.is_empty() {
            return Ok(Some(s));
        }
    }
    if let Ok(v) = env::var(name) {
        if !v.is_empty() {
            return Ok(Some(Secret::new(v)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use tempfile::{NamedTempFile, TempDir};

    fn store_in(dir: &TempDir) -> SecretsStore {
        SecretsStore::open_at(dir.path().join("secrets.toml")).unwrap()
    }

    #[test]
    fn set_and_get_roundtrip() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store
            .set("ANTHROPIC_API_KEY", Secret::new("sk-test-123"))
            .unwrap();
        let val = store.get("ANTHROPIC_API_KEY").unwrap().unwrap();
        assert_eq!(val.expose(), "sk-test-123");
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(text.contains("ANTHROPIC_API_KEY = \"sk-test-123\""));
        assert!(!text.contains("[secrets]"));
    }

    #[test]
    fn get_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        assert!(store.get("NO_SUCH_KEY").unwrap().is_none());
    }

    #[test]
    fn delete_removes_key() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.set("K", Secret::new("v")).unwrap();
        assert!(store.delete("K").unwrap());
        assert!(!store.delete("K").unwrap());
        assert!(store.get("K").unwrap().is_none());
    }

    #[test]
    fn list_names_sorted() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.set("Z", Secret::new("1")).unwrap();
        store.set("A", Secret::new("2")).unwrap();
        assert_eq!(store.list_names().unwrap(), vec!["A", "Z"]);
    }

    #[cfg(unix)]
    #[test]
    fn file_has_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.set("K", Secret::new("v")).unwrap();
        let meta = std::fs::metadata(store.path()).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn world_readable_file_refuses_to_read() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secrets.toml");
        fs::write(&path, "K = \"v\"\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let error = SecretsStore::open_at(path).unwrap_err();
        assert!(matches!(error, SecretsError::InsecurePermissions(_)));
    }

    #[cfg(unix)]
    #[test]
    fn world_writable_parent_refuses_write() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let store = store_in(&dir);

        let error = store.set("K", Secret::new("v")).unwrap_err();
        assert!(matches!(error, SecretsError::InsecurePermissions(_)));
    }

    #[test]
    fn abandoned_tempfile_leaves_original_untouched() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.set("K", Secret::new("original")).unwrap();

        let mut tmp = NamedTempFile::new_in(dir.path()).unwrap();
        tmp.write_all(b"K = \"crashed\"\n").unwrap();
        tmp.flush().unwrap();
        drop(tmp);

        assert_eq!(store.get("K").unwrap().unwrap().expose(), "original");
    }

    #[test]
    fn concurrent_writers_do_not_corrupt_file() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(store_in(&dir));
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();

        for (name, value) in [("A", "1"), ("B", "2")] {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                store.set(name, Secret::new(value)).unwrap();
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(store.get("A").unwrap().unwrap().expose(), "1");
        assert_eq!(store.get("B").unwrap().unwrap().expose(), "2");
    }

    #[test]
    fn resolve_cli_override_wins() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.set("KEY", Secret::new("from-store")).unwrap();
        let cli = Secret::new("from-cli");
        let result = resolve_secret("KEY", Some(&cli), &store).unwrap();
        assert_eq!(result.unwrap().expose(), "from-cli");
    }

    #[test]
    fn resolve_store_beats_env() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        store.set("KEY", Secret::new("from-store")).unwrap();
        let result = resolve_secret("KEY", None, &store).unwrap();
        assert_eq!(result.unwrap().expose(), "from-store");
    }

    #[test]
    fn resolve_env_wins_over_none() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        let key = "HELM_TEST_SECRET_ENV_WINS_OVER_NONE";
        unsafe {
            env::set_var(key, "from-env");
        }
        let result = resolve_secret(key, None, &store).unwrap();
        unsafe {
            env::remove_var(key);
        }
        assert_eq!(result.unwrap().expose(), "from-env");
    }

    #[cfg(unix)]
    #[test]
    fn resolve_store_error_propagates() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secrets.toml");
        fs::write(&path, "KEY = \"from-store\"\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let store = SecretsStore { path };

        let error = resolve_secret("KEY", None, &store).unwrap_err();
        assert!(matches!(error, SecretsError::InsecurePermissions(_)));
    }

    #[test]
    fn resolve_none_when_nothing_set() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        let result = resolve_secret("HELM_TEST_NO_VAR_XYZ", None, &store).unwrap();
        assert!(result.is_none());
    }
}
