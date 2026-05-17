//! SSH-reachable remote target registry stored at `~/.helm/remotes.toml`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RemoteRegistry {
    #[serde(default)]
    pub remotes: Vec<RemoteEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum Credential {
    #[default]
    SshAgent,
    KeyFile(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEntry {
    #[serde(default = "Uuid::new_v4")]
    pub host_id: Uuid,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub ssh_opts: Option<String>,
    #[serde(default)]
    pub credential: Credential,
}

/// Finding from a remote host, tagged with host_id for aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct FleetFinding {
    pub host_id: Uuid,
    pub host_name: String,
    pub severity: String,
    pub title: String,
    pub description: Option<String>,
    pub last_seen: Option<i64>,
}

fn default_port() -> u16 {
    22
}

fn xdg_config_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("helm")
}

pub fn registry_path() -> Result<PathBuf> {
    Ok(xdg_config_dir().join("remotes.toml"))
}

impl RemoteRegistry {
    pub fn load() -> Result<Self> {
        let path = registry_path()?;
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("malformed remotes file at {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => {
                Err(anyhow!(error)).with_context(|| format!("reading {}", path.display()))
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = registry_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializing remotes registry")?;
        fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    }

    pub fn get(&self, name: &str) -> Option<&RemoteEntry> {
        self.remotes.iter().find(|r| r.name == name)
    }

    pub fn upsert(&mut self, entry: RemoteEntry) {
        if let Some(slot) = self.remotes.iter_mut().find(|r| r.name == entry.name) {
            *slot = entry;
        } else {
            self.remotes.push(entry);
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.remotes.len();
        self.remotes.retain(|r| r.name != name);
        self.remotes.len() != before
    }

    /// Collect findings from all remotes in parallel, bounded to 20 concurrent tasks.
    /// Returns a vec of (host_id, result) tuples.
    #[allow(dead_code)]
    pub async fn parallel_collect(&self) -> Vec<(Uuid, Result<Vec<FleetFinding>>)> {
        use tokio::task::JoinSet;

        let mut joinset = JoinSet::new();
        let mut results = Vec::new();
        let mut task_iter = self.remotes.iter().peekable();
        const MAX_CONCURRENT: usize = 20;

        // Spawn initial batch of tasks (up to MAX_CONCURRENT)
        while joinset.len() < MAX_CONCURRENT && task_iter.peek().is_some() {
            if let Some(remote) = task_iter.next() {
                let remote_clone = remote.clone();
                joinset.spawn(async move {
                    (remote_clone.host_id, remote_clone.collect_findings().await)
                });
            }
        }

        // Spawn remaining tasks as earlier ones complete
        while !joinset.is_empty() {
            if let Ok((host_id, findings_result)) = joinset.join_next().await.unwrap() {
                results.push((host_id, findings_result));
            }

            // Spawn one more task if available and we're below the max
            if joinset.len() < MAX_CONCURRENT {
                if let Some(remote) = task_iter.next() {
                    let remote_clone = remote.clone();
                    joinset.spawn(async move {
                        (remote_clone.host_id, remote_clone.collect_findings().await)
                    });
                }
            }
        }

        results
    }
}

impl RemoteEntry {
    /// Build the `ssh user@host -p port [opts]` command prefix as a vector of args.
    pub fn ssh_argv(&self) -> Vec<String> {
        let mut argv = vec!["ssh".to_owned()];
        if let Some(opts) = self.ssh_opts.as_deref() {
            for tok in opts.split_whitespace() {
                argv.push(tok.to_owned());
            }
        }
        if self.port != 22 {
            argv.push("-p".to_owned());
            argv.push(self.port.to_string());
        }
        argv.push("-o".to_owned());
        argv.push("BatchMode=yes".to_owned());
        argv.push("-o".to_owned());
        argv.push("ConnectTimeout=10".to_owned());
        if let Credential::KeyFile(path) = &self.credential {
            argv.push("-i".to_owned());
            argv.push(path.to_string_lossy().to_string());
        }
        argv.push(match self.user.as_deref() {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        });
        argv
    }

    /// Run `ssh remote true` and return whether the connection succeeded.
    pub async fn ping(&self) -> Result<bool> {
        let mut argv = self.ssh_argv();
        argv.push("true".to_owned());
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let output = cmd.output().await.context("spawning ssh")?;
        Ok(output.status.success())
    }

    /// Collect findings from remote host via SSH + helm monitor --format json.
    /// Returns a vector of FleetFinding tagged with this host's ID.
    /// Timeout: 30s per host.
    #[allow(dead_code)]
    pub async fn collect_findings(&self) -> Result<Vec<FleetFinding>> {
        let mut argv = self.ssh_argv();
        argv.extend_from_slice(&[
            "helm".to_owned(),
            "monitor".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ]);

        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output_future = cmd.output();
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), output_future)
            .await
            .context("timeout collecting findings from remote host")?
            .context("spawning ssh for findings collection")?;

        if !output.status.success() {
            return Err(anyhow!(
                "helm monitor on {} returned non-zero status",
                self.name
            ))
            .with_context(|| format!("collecting findings from {}", self.name));
        }

        // Parse stdout as JSON array
        let json_text = String::from_utf8_lossy(&output.stdout);
        let values: Vec<serde_json::Value> = serde_json::from_str(&json_text)
            .with_context(|| format!("parsing JSON from {}", self.name))?;

        // Map to FleetFinding, tagging with host_id
        let mut findings = Vec::new();
        for val in values {
            if let Ok(obj) = val.as_object().ok_or_else(|| anyhow!("expected object")) {
                let severity = obj
                    .get("severity")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned();
                let title = obj
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned();
                let description = obj
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                let last_seen = obj.get("last_seen").and_then(|v| v.as_i64());

                findings.push(FleetFinding {
                    host_id: self.host_id,
                    host_name: self.name.clone(),
                    severity,
                    title,
                    description,
                    last_seen,
                });
            }
        }

        Ok(findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn registry_load_from_missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let registry = RemoteRegistry::load_from(&dir.path().join("missing.toml")).unwrap();

        assert!(registry.remotes.is_empty());
    }

    #[test]
    fn registry_upsert_replaces_existing_entry() {
        let mut registry = RemoteRegistry::default();
        let host_id = Uuid::new_v4();
        registry.upsert(RemoteEntry {
            host_id,
            name: "prod".to_owned(),
            host: "prod-1.example.com".to_owned(),
            port: 22,
            user: Some("root".to_owned()),
            ssh_opts: None,
            credential: Credential::SshAgent,
        });
        registry.upsert(RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "prod".to_owned(),
            host: "prod-2.example.com".to_owned(),
            port: 2222,
            user: Some("ubuntu".to_owned()),
            ssh_opts: Some("-i ~/.ssh/prod".to_owned()),
            credential: Credential::SshAgent,
        });

        assert_eq!(registry.remotes.len(), 1);
        assert_eq!(registry.get("prod").unwrap().host, "prod-2.example.com");
        assert_eq!(registry.get("prod").unwrap().port, 2222);
    }

    #[test]
    fn remote_entry_ssh_argv_includes_port_user_and_opts() {
        let entry = RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "prod".to_owned(),
            host: "prod.example.com".to_owned(),
            port: 2222,
            user: Some("ubuntu".to_owned()),
            ssh_opts: Some("-i ~/.ssh/prod -o StrictHostKeyChecking=no".to_owned()),
            credential: Credential::SshAgent,
        };

        assert_eq!(
            entry.ssh_argv(),
            vec![
                "ssh",
                "-i",
                "~/.ssh/prod",
                "-o",
                "StrictHostKeyChecking=no",
                "-p",
                "2222",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "ubuntu@prod.example.com"
            ]
        );
    }

    #[test]
    fn test_ssh_argv_with_keyfile_injects_identity_flag() {
        let entry = RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "prod".to_owned(),
            host: "prod.example.com".to_owned(),
            port: 22,
            user: Some("ubuntu".to_owned()),
            ssh_opts: None,
            credential: Credential::KeyFile(PathBuf::from("/home/user/.ssh/id_rsa")),
        };

        let argv = entry.ssh_argv();
        assert!(argv.contains(&"-i".to_owned()));
        assert!(argv.contains(&"/home/user/.ssh/id_rsa".to_owned()));

        // Verify -i comes before the host argument
        let i_idx = argv.iter().position(|x| x == "-i").unwrap();
        let path_idx = argv
            .iter()
            .position(|x| x == "/home/user/.ssh/id_rsa")
            .unwrap();
        let host_idx = argv
            .iter()
            .position(|x| x.contains("prod.example.com"))
            .unwrap();
        assert!(i_idx < path_idx);
        assert!(path_idx < host_idx);
    }

    #[test]
    fn test_credential_default_is_ssh_agent() {
        let cred = Credential::default();
        assert!(matches!(cred, Credential::SshAgent));
    }

    #[test]
    fn test_credential_serialization_roundtrip() {
        let entry = RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "prod".to_owned(),
            host: "prod.example.com".to_owned(),
            port: 22,
            user: Some("ubuntu".to_owned()),
            ssh_opts: None,
            credential: Credential::KeyFile(PathBuf::from("/home/user/.ssh/id_rsa")),
        };

        // Serialize to TOML and back
        let toml_str = toml::to_string(&entry).unwrap();
        let deserialized: RemoteEntry = toml::from_str(&toml_str).unwrap();

        // Verify credential matches
        match (&entry.credential, &deserialized.credential) {
            (Credential::KeyFile(path1), Credential::KeyFile(path2)) => {
                assert_eq!(path1, path2);
            }
            _ => panic!("Credential serialization roundtrip failed"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_fleet_parallel_refresh_20_hosts() {
        // Create a mock RemoteRegistry with 20 RemoteEntry fixtures
        let mut registry = RemoteRegistry::default();
        let start = std::time::Instant::now();

        for i in 0..20 {
            registry.upsert(RemoteEntry {
                host_id: Uuid::new_v4(),
                name: format!("host-{}", i),
                host: format!("host-{}.invalid", i), // invalid hosts will timeout
                port: 22,
                user: Some("ubuntu".to_owned()),
                ssh_opts: None,
                credential: Credential::SshAgent,
            });
        }

        // Call parallel_collect and measure elapsed time
        let results = registry.parallel_collect().await;
        let elapsed = start.elapsed();

        // Assert all 20 hosts return results (success or error)
        assert_eq!(results.len(), 20, "expected 20 results");

        // Each result should be a (host_id, Result<...>)
        for (host_id, result) in &results {
            // host_id should not be nil
            assert_ne!(host_id.as_bytes(), &[0u8; 16]);
            // result will be an error due to invalid hosts, which is fine
            let _ = result; // silence unused warning
        }

        // Assert elapsed time is reasonable (parallel should be much faster than serial)
        // With 30s timeout per host and max 20 concurrent, worst case is ~30s
        // If parallel is working, 20 invalid hosts should timeout in ~30s total
        // not 20 * 30s = 600s
        eprintln!("20-host parallel collection took: {:?}", elapsed);
        assert!(
            elapsed.as_secs() <= 35,
            "parallel collection took too long: {:?}",
            elapsed
        );
    }
}
