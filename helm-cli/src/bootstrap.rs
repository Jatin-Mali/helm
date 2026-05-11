//! Q3 — `helm bootstrap <host>`.
//!
//! Two-step install onto a reachable Linux host:
//!
//! 1. Detect remote OS + architecture via `ssh <host> uname -srm`.
//! 2. Copy the locally-running helm binary to `~/.helm/bin/helm` on the
//!    remote, chmod +x, and verify with `helm --version`.
//!
//! When a `--release` URL is supplied the binary is downloaded on the remote
//! via `curl -fsSL`; otherwise the local binary is uploaded via `scp`. After
//! a successful bootstrap, the remote is registered in `~/.helm/remotes.toml`
//! (if not already present).
//!
//! Bootstrap intentionally does *not* mutate the local installation, does
//! *not* read `~/.ssh/config` directly (we defer to ssh-agent + the user's
//! existing config), and does *not* run anything other than a version probe
//! after install.

use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result, anyhow, bail};
use tokio::process::Command;

use crate::remote::{RemoteEntry, RemoteRegistry};

#[derive(Debug, Clone)]
pub struct BootstrapPlan {
    pub host: String,
    pub user: Option<String>,
    pub port: u16,
    pub release_url: Option<String>,
    pub register_as: Option<String>,
}

impl BootstrapPlan {
    pub fn ssh_target(&self) -> String {
        match self.user.as_deref() {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }

    pub fn to_remote_entry(&self, name: &str) -> RemoteEntry {
        RemoteEntry {
            name: name.to_owned(),
            host: self.host.clone(),
            port: self.port,
            user: self.user.clone(),
            ssh_opts: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BootstrapReport {
    pub host: String,
    pub remote_uname: String,
    pub remote_helm_version: String,
    pub installed_path: String,
    pub registered_as: Option<String>,
}

pub async fn run(plan: BootstrapPlan, local_binary: PathBuf) -> Result<BootstrapReport> {
    let target = plan.ssh_target();
    let port_args: Vec<String> = if plan.port != 22 {
        vec!["-p".to_owned(), plan.port.to_string()]
    } else {
        Vec::new()
    };
    let common = [
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        "-o".to_owned(),
        "ConnectTimeout=10".to_owned(),
    ];

    let uname = run_ssh(&target, &port_args, &common, &["uname", "-srm"]).await?;
    let install_root = "$HOME/.helm/bin";
    run_ssh(&target, &port_args, &common, &["mkdir", "-p", install_root]).await?;

    let installed_path = "$HOME/.helm/bin/helm".to_owned();
    if let Some(url) = plan.release_url.as_deref() {
        let curl_cmd = format!(
            "curl -fsSL {} -o $HOME/.helm/bin/helm && chmod +x $HOME/.helm/bin/helm",
            shell_escape(url)
        );
        run_ssh(&target, &port_args, &common, &["sh", "-c", &curl_cmd]).await?;
    } else {
        scp_upload(&local_binary, &target, plan.port, "$HOME/.helm/bin/helm").await?;
        run_ssh(
            &target,
            &port_args,
            &common,
            &["chmod", "+x", "$HOME/.helm/bin/helm"],
        )
        .await?;
    }

    let version = run_ssh(
        &target,
        &port_args,
        &common,
        &["$HOME/.helm/bin/helm", "--version"],
    )
    .await
    .context("verifying remote helm install via --version")?;

    let registered_as = match plan.register_as.as_deref() {
        Some(name) => {
            let mut registry = RemoteRegistry::load().unwrap_or_default();
            registry.upsert(plan.to_remote_entry(name));
            registry.save()?;
            Some(name.to_owned())
        }
        None => None,
    };

    Ok(BootstrapReport {
        host: plan.host,
        remote_uname: uname.trim().to_owned(),
        remote_helm_version: version.trim().to_owned(),
        installed_path,
        registered_as,
    })
}

async fn run_ssh(
    target: &str,
    port_args: &[String],
    common: &[String],
    remote_argv: &[&str],
) -> Result<String> {
    let mut argv: Vec<String> = vec!["ssh".to_owned()];
    argv.extend_from_slice(port_args);
    argv.extend_from_slice(common);
    argv.push(target.to_owned());
    for arg in remote_argv {
        argv.push((*arg).to_owned());
    }
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd
        .output()
        .await
        .with_context(|| format!("spawning ssh {target}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "remote ssh `{}` failed ({}): {}",
            remote_argv.join(" "),
            output.status,
            stderr.trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(stdout)
}

async fn scp_upload(
    local: &std::path::Path,
    target: &str,
    port: u16,
    remote_path: &str,
) -> Result<()> {
    if !local.exists() {
        return Err(anyhow!(
            "local helm binary not found at {}",
            local.display()
        ));
    }
    let mut argv: Vec<String> = vec!["scp".to_owned()];
    if port != 22 {
        argv.push("-P".to_owned());
        argv.push(port.to_string());
    }
    argv.push("-o".to_owned());
    argv.push("BatchMode=yes".to_owned());
    argv.push("-o".to_owned());
    argv.push("ConnectTimeout=10".to_owned());
    argv.push(local.display().to_string());
    argv.push(format!("{target}:{remote_path}"));
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd.output().await.context("spawning scp")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "scp upload to {target}:{remote_path} failed ({}): {}",
            output.status,
            stderr.trim()
        );
    }
    Ok(())
}

fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_owned();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '.' | '-' | ':' | '@'));
    if safe {
        return s.to_owned();
    }
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_ssh_target_uses_user() {
        let plan = BootstrapPlan {
            host: "host.example".into(),
            user: Some("ubuntu".into()),
            port: 22,
            release_url: None,
            register_as: None,
        };
        assert_eq!(plan.ssh_target(), "ubuntu@host.example");
    }

    #[test]
    fn plan_ssh_target_falls_back_to_bare_host() {
        let plan = BootstrapPlan {
            host: "host.example".into(),
            user: None,
            port: 22,
            release_url: None,
            register_as: None,
        };
        assert_eq!(plan.ssh_target(), "host.example");
    }

    #[test]
    fn shell_escape_passes_safe() {
        assert_eq!(
            shell_escape("https://example.com/helm"),
            "https://example.com/helm"
        );
    }

    #[test]
    fn shell_escape_quotes_unsafe() {
        assert_eq!(shell_escape("a b"), "'a b'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }
}
