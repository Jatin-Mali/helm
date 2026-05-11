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

pub fn registry_path() -> Result<PathBuf> {
    let base = dirs::home_dir().ok_or_else(|| anyhow!("could not resolve home directory"))?;
    Ok(base.join(".helm").join("remotes.toml"))
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
