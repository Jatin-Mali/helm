//! SSH-reachable remote target registry stored at
//! `$XDG_CONFIG_HOME/helm/remotes.toml`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use helm_monitor::{EvidenceRef, Finding, SystemSnapshot};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RemoteRegistry {
    #[serde(default)]
    pub remotes: Vec<RemoteEntry>,
}

const FLEET_COLLECTION_TIMEOUT_SECS: u64 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Credential {
    #[default]
    SshAgent,
    KeyFile(PathBuf),
    Password,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteTransport {
    #[default]
    Ssh,
    InstalledAgent,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentTrustMode {
    #[default]
    ManualRegistration,
    SshBootstrap,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentSupervisor {
    #[default]
    DetachedProcess,
    SystemdUser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    #[serde(default = "default_agent_service_name")]
    pub service_name: String,
    #[serde(default = "default_agent_install_path")]
    pub install_path: String,
    #[serde(default = "default_agent_interval_secs")]
    pub interval_secs: u64,
    #[serde(default)]
    pub supervisor: AgentSupervisor,
    #[serde(default)]
    pub trust: AgentTrustMode,
    #[serde(default = "now_rfc3339")]
    pub registered_at: String,
}

impl Default for AgentMetadata {
    fn default() -> Self {
        Self {
            service_name: default_agent_service_name(),
            install_path: default_agent_install_path(),
            interval_secs: default_agent_interval_secs(),
            supervisor: AgentSupervisor::DetachedProcess,
            trust: AgentTrustMode::ManualRegistration,
            registered_at: now_rfc3339(),
        }
    }
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
    #[serde(default)]
    pub transport: RemoteTransport,
    #[serde(default)]
    pub agent: Option<AgentMetadata>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetHostReport {
    pub host_id: Uuid,
    pub host_name: String,
    pub findings: Vec<Finding>,
    pub snapshot: Option<SystemSnapshot>,
}

fn default_port() -> u16 {
    22
}

fn default_agent_service_name() -> String {
    "helmopsd.service".to_owned()
}

fn default_agent_install_path() -> String {
    "$HOME/.helm/bin/helmops".to_owned()
}

fn default_agent_interval_secs() -> u64 {
    60
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
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
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(FLEET_COLLECTION_TIMEOUT_SECS),
                        remote_clone.collect_findings(),
                    )
                    .await
                    .map_err(|_| {
                        anyhow!(
                            "timed out after {}s collecting findings from {}",
                            FLEET_COLLECTION_TIMEOUT_SECS,
                            remote_clone.name
                        )
                    })
                    .and_then(|result| result);
                    (remote_clone.host_id, result)
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
                        let result = tokio::time::timeout(
                            std::time::Duration::from_secs(FLEET_COLLECTION_TIMEOUT_SECS),
                            remote_clone.collect_findings(),
                        )
                        .await
                        .map_err(|_| {
                            anyhow!(
                                "timed out after {}s collecting findings from {}",
                                FLEET_COLLECTION_TIMEOUT_SECS,
                                remote_clone.name
                            )
                        })
                        .and_then(|result| result);
                        (remote_clone.host_id, result)
                    });
                }
            }
        }

        results
    }

    /// Collect a richer remote report from all remotes in parallel, bounded to 20 tasks.
    #[allow(dead_code)]
    pub async fn parallel_collect_reports(&self) -> Vec<(Uuid, Result<FleetHostReport>)> {
        use tokio::task::JoinSet;

        let mut joinset = JoinSet::new();
        let mut results = Vec::new();
        let mut task_iter = self.remotes.iter().peekable();
        const MAX_CONCURRENT: usize = 20;

        while joinset.len() < MAX_CONCURRENT && task_iter.peek().is_some() {
            if let Some(remote) = task_iter.next() {
                let remote_clone = remote.clone();
                joinset.spawn(async move {
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(FLEET_COLLECTION_TIMEOUT_SECS),
                        remote_clone.collect_report(),
                    )
                    .await
                    .map_err(|_| {
                        anyhow!(
                            "timed out after {}s collecting report from {}",
                            FLEET_COLLECTION_TIMEOUT_SECS,
                            remote_clone.name
                        )
                    })
                    .and_then(|result| result);
                    (remote_clone.host_id, result)
                });
            }
        }

        while !joinset.is_empty() {
            if let Ok((host_id, report_result)) = joinset.join_next().await.unwrap() {
                results.push((host_id, report_result));
            }

            if joinset.len() < MAX_CONCURRENT
                && let Some(remote) = task_iter.next()
            {
                let remote_clone = remote.clone();
                joinset.spawn(async move {
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(FLEET_COLLECTION_TIMEOUT_SECS),
                        remote_clone.collect_report(),
                    )
                    .await
                    .map_err(|_| {
                        anyhow!(
                            "timed out after {}s collecting report from {}",
                            FLEET_COLLECTION_TIMEOUT_SECS,
                            remote_clone.name
                        )
                    })
                    .and_then(|result| result);
                    (remote_clone.host_id, result)
                });
            }
        }

        results
    }
}

impl RemoteEntry {
    fn remote_helm_command(&self) -> String {
        self.agent
            .as_ref()
            .map(|agent| agent.install_path.clone())
            .unwrap_or_else(|| "helmops".to_owned())
    }

    pub fn summary_label(&self) -> String {
        match (&self.transport, &self.agent) {
            (RemoteTransport::InstalledAgent, Some(agent)) => {
                let supervisor = match agent.supervisor {
                    AgentSupervisor::SystemdUser => "systemd-user",
                    AgentSupervisor::DetachedProcess => "detached",
                };
                format!("agent/{supervisor}")
            }
            (RemoteTransport::InstalledAgent, None) => "agent".to_owned(),
            (RemoteTransport::Ssh, _) => "ssh".to_owned(),
        }
    }

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
        argv.push(format!("ConnectTimeout={FLEET_COLLECTION_TIMEOUT_SECS}"));
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

    pub async fn run_shell(&self, script: &str) -> Result<String> {
        let mut argv = self.ssh_argv();
        argv.push("sh".to_owned());
        argv.push("-lc".to_owned());
        argv.push(script.to_owned());
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = cmd.output().await.context("spawning ssh shell")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "remote shell failed on {} ({}): {}",
                self.name,
                output.status,
                stderr.trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
        let raw_findings = self.collect_raw_findings().await?;
        let traced_findings = self.annotate_remote_findings(raw_findings);

        // Map to FleetFinding, tagging with host_id
        let mut findings = Vec::new();
        for finding in traced_findings {
            findings.push(FleetFinding {
                host_id: self.host_id,
                host_name: self.name.clone(),
                severity: finding.severity.as_str().to_owned(),
                title: finding.title.clone(),
                description: (!finding.impact.trim().is_empty()).then(|| finding.impact.clone()),
                last_seen: None,
            });
        }

        Ok(findings)
    }

    pub async fn collect_report(&self) -> Result<FleetHostReport> {
        let findings = self.annotate_remote_findings(self.collect_raw_findings().await?);
        let snapshot = self.collect_snapshot().await.ok();
        Ok(FleetHostReport {
            host_id: self.host_id,
            host_name: self.name.clone(),
            findings,
            snapshot,
        })
    }

    async fn collect_raw_findings(&self) -> Result<Vec<Finding>> {
        let stdout = self
            .run_helm_json(&["monitor", "--format", "json"], "findings collection")
            .await?;
        serde_json::from_str::<Vec<Finding>>(&stdout)
            .with_context(|| format!("parsing findings JSON from {}", self.name))
    }

    async fn collect_snapshot(&self) -> Result<SystemSnapshot> {
        let stdout = self
            .run_helm_json(&["snapshot", "--json"], "snapshot collection")
            .await?;
        serde_json::from_str::<SystemSnapshot>(&stdout)
            .with_context(|| format!("parsing snapshot JSON from {}", self.name))
    }

    async fn run_helm_json(&self, remote_argv: &[&str], action: &str) -> Result<String> {
        let mut argv = self.ssh_argv();
        argv.push(self.remote_helm_command());
        argv.extend(remote_argv.iter().map(|arg| (*arg).to_owned()));

        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output_future = cmd.output();
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), output_future)
            .await
            .with_context(|| format!("timeout during {action} from remote host"))?
            .with_context(|| format!("spawning ssh for {action}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "{} on {} returned non-zero status: {}",
                action,
                self.name,
                stderr.trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn annotate_remote_findings(&self, findings: Vec<Finding>) -> Vec<Finding> {
        let host_id = self.host_id.to_string();
        findings
            .into_iter()
            .map(|mut finding| {
                if !finding
                    .evidence
                    .iter()
                    .any(|evidence| evidence.source == "remote.host_id")
                {
                    finding.evidence.push(EvidenceRef {
                        source: "remote.host_id".to_owned(),
                        value: host_id.clone(),
                        note: format!("Collected from remote host {}", self.name),
                    });
                }
                finding
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helm_monitor::{Confidence, MonitorDomain, Severity};
    use proptest::prelude::*;
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
            transport: RemoteTransport::Ssh,
            agent: None,
        });
        registry.upsert(RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "prod".to_owned(),
            host: "prod-2.example.com".to_owned(),
            port: 2222,
            user: Some("ubuntu".to_owned()),
            ssh_opts: Some("-i ~/.ssh/prod".to_owned()),
            credential: Credential::SshAgent,
            transport: RemoteTransport::Ssh,
            agent: None,
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
            transport: RemoteTransport::Ssh,
            agent: None,
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
                "ConnectTimeout=2",
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
            transport: RemoteTransport::Ssh,
            agent: None,
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
    fn test_installed_agent_summary_label() {
        let entry = RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "prod".to_owned(),
            host: "prod.example.com".to_owned(),
            port: 22,
            user: Some("ubuntu".to_owned()),
            ssh_opts: None,
            credential: Credential::SshAgent,
            transport: RemoteTransport::InstalledAgent,
            agent: Some(AgentMetadata {
                supervisor: AgentSupervisor::SystemdUser,
                ..AgentMetadata::default()
            }),
        };

        assert_eq!(entry.summary_label(), "agent/systemd-user");
        assert_eq!(entry.remote_helm_command(), "$HOME/.helm/bin/helmops");
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
            transport: RemoteTransport::Ssh,
            agent: None,
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

    #[test]
    fn password_credential_serializes_without_secret_material() {
        let entry = RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "prod".to_owned(),
            host: "prod.example.com".to_owned(),
            port: 22,
            user: Some("ubuntu".to_owned()),
            ssh_opts: None,
            credential: Credential::Password,
            transport: RemoteTransport::Ssh,
            agent: None,
        };

        let toml_str = toml::to_string(&entry).unwrap();
        assert!(toml_str.contains("password"));
        assert!(!toml_str.contains("secret"));
        let deserialized: RemoteEntry = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.credential, Credential::Password);
    }

    #[test]
    fn test_host_id_uuid_persistence() {
        use tempfile::tempdir;

        // Test 1: RemoteEntry::Default produces unique UUIDs
        let entry1 = RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "host1".to_owned(),
            host: "host1.example.com".to_owned(),
            port: 22,
            user: None,
            ssh_opts: None,
            credential: Credential::SshAgent,
            transport: RemoteTransport::Ssh,
            agent: None,
        };
        let entry2 = RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "host2".to_owned(),
            host: "host2.example.com".to_owned(),
            port: 22,
            user: None,
            ssh_opts: None,
            credential: Credential::SshAgent,
            transport: RemoteTransport::Ssh,
            agent: None,
        };
        assert_ne!(entry1.host_id, entry2.host_id, "UUIDs should be unique");

        // Test 2: RemoteEntry serializes and deserializes with stable host_id
        let original_uuid = entry1.host_id;
        let toml_str = toml::to_string(&entry1).unwrap();
        let deserialized: RemoteEntry = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            deserialized.host_id, original_uuid,
            "host_id should persist across serialize/deserialize"
        );

        // Test 3: RemoteRegistry save and load preserves host_ids
        let mut registry = RemoteRegistry::default();
        let entry_with_uuid = RemoteEntry {
            host_id: Uuid::new_v4(),
            name: "test-host".to_owned(),
            host: "test.example.com".to_owned(),
            port: 22,
            user: Some("ubuntu".to_owned()),
            ssh_opts: None,
            credential: Credential::SshAgent,
            transport: RemoteTransport::Ssh,
            agent: None,
        };
        let original_host_id = entry_with_uuid.host_id;
        registry.upsert(entry_with_uuid.clone());

        // Write to temp file and reload
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_remotes.toml");
        let registry_to_save = registry.clone();
        let toml_text = toml::to_string_pretty(&registry_to_save).unwrap();
        std::fs::write(&path, toml_text).unwrap();

        // Load it back
        let loaded_registry = RemoteRegistry::load_from(&path).unwrap();
        let reloaded_entry = loaded_registry.get("test-host").unwrap();
        assert_eq!(
            reloaded_entry.host_id, original_host_id,
            "host_id should persist across registry save/load cycles"
        );

        // Test 4: Old TOML entries without host_id get auto-generated on load
        // Create a TOML string without host_id field (backward compatibility)
        let old_toml = r#"
[[remotes]]
name = "legacy-host"
host = "legacy.example.com"
port = 22
"#;
        let legacy_registry: RemoteRegistry = toml::from_str(old_toml).unwrap();
        let legacy_entry = legacy_registry.remotes.first().unwrap();
        assert_ne!(
            legacy_entry.host_id.as_bytes(),
            &[0u8; 16],
            "host_id should be auto-generated (non-nil UUID) for old TOML entries"
        );
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
                transport: RemoteTransport::Ssh,
                agent: None,
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

        // Assert elapsed time is bounded by the fleet timeout budget.
        eprintln!("20-host parallel collection took: {:?}", elapsed);
        assert!(
            elapsed.as_secs() <= FLEET_COLLECTION_TIMEOUT_SECS + 1,
            "parallel collection took too long: {:?}",
            elapsed
        );
    }

    proptest! {
        #[test]
        fn property_34_remote_findings_include_host_traceability(
            resource in "[a-z0-9/_-]{1,24}",
            title in "[A-Za-z0-9 .:_/-]{1,48}"
        ) {
            let entry = RemoteEntry {
                host_id: Uuid::new_v4(),
                name: "remote-a".to_owned(),
                host: "remote-a.example.com".to_owned(),
                port: 22,
                user: Some("ubuntu".to_owned()),
                ssh_opts: None,
                credential: Credential::SshAgent,
                transport: RemoteTransport::Ssh,
                agent: None,
            };
            let finding = Finding::new(
                "snap-1",
                "test-detector",
                &resource,
                &title,
                Severity::Warning,
                Confidence::Medium,
                MonitorDomain::Services,
            )
            .with_evidence("services.units[0].active", "failed", "service failure");
            let traced = entry.annotate_remote_findings(vec![finding]);
            let host_id = entry.host_id.to_string();
            let all_traced = traced.iter().all(|finding| {
                finding
                    .evidence
                    .iter()
                    .any(|evidence| evidence.source == "remote.host_id" && evidence.value == host_id)
            });

            prop_assert!(all_traced);
        }
    }
}
