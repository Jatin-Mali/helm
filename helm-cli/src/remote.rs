//! SSH-reachable remote target registry stored at `~/.helm/remotes.toml`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RemoteRegistry {
    #[serde(default)]
    pub remotes: Vec<RemoteEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub ssh_opts: Option<String>,
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
        registry.upsert(RemoteEntry {
            name: "prod".to_owned(),
            host: "prod-1.example.com".to_owned(),
            port: 22,
            user: Some("root".to_owned()),
            ssh_opts: None,
        });
        registry.upsert(RemoteEntry {
            name: "prod".to_owned(),
            host: "prod-2.example.com".to_owned(),
            port: 2222,
            user: Some("ubuntu".to_owned()),
            ssh_opts: Some("-i ~/.ssh/prod".to_owned()),
        });

        assert_eq!(registry.remotes.len(), 1);
        assert_eq!(registry.get("prod").unwrap().host, "prod-2.example.com");
        assert_eq!(registry.get("prod").unwrap().port, 2222);
    }

    #[test]
    fn remote_entry_ssh_argv_includes_port_user_and_opts() {
        let entry = RemoteEntry {
            name: "prod".to_owned(),
            host: "prod.example.com".to_owned(),
            port: 2222,
            user: Some("ubuntu".to_owned()),
            ssh_opts: Some("-i ~/.ssh/prod -o StrictHostKeyChecking=no".to_owned()),
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
}
