//! Typed, bounded, read-only system collectors.
//!
//! Every collector:
//! - Is typed and bounded (TRD §5).
//! - Uses a per-collector timeout from the profile.
//! - Never mutates system state.
//! - Tolerates missing binaries gracefully.
//! - Returns domain-specific errors in CollectorError format.

pub mod backups;
pub mod certs;
pub mod compose;
pub mod containers;
pub mod db_health;
pub mod disks;
pub mod firewall;
pub mod host;
pub mod kubernetes;
pub mod libvirt;
pub mod load;
pub mod logs;
pub mod network;
pub mod packages;
pub mod ports;
pub mod processes;
pub mod services;
pub mod timers;
pub mod web_servers;

use std::process::Output;

use tokio::process::Command;

use crate::snapshot::CollectorError;
use crate::snapshot::{MonitorProfile, Seconds};

/// Common trait for all collectors.
#[allow(async_fn_in_trait)]
pub trait Collector: Send + Sync {
    type Output: Send;

    fn domain(&self) -> &'static str;

    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError>;
}

/// Launch a command with the profile's per-collector timeout.
async fn run_timed(
    program: &str,
    args: &[&str],
    profile: MonitorProfile,
) -> Result<Output, CollectorError> {
    let timeout = profile.per_collector_timeout();
    let mut command = Command::new(program);
    command.args(args).stdin(std::process::Stdio::null());
    let output = crate::process_runner::run_command(
        command,
        crate::process_runner::ProcessLimits {
            timeout: std::time::Duration::from_secs(timeout),
            ..Default::default()
        },
        &Default::default(),
    )
    .await
    .map_err(|error| CollectorError {
        domain: String::new(),
        message: format!("{program}: {error}"),
        is_timeout: error.timed_out,
    })?;
    if !accepted_exit(program, args, output.status.code()) {
        return Err(err(
            "",
            helm_core::redact_secrets(&format!(
                "{program}: exited with {}; stderr: {}; stdout: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            )),
        ));
    }
    Ok(output)
}

fn accepted_exit(program: &str, args: &[&str], code: Option<i32>) -> bool {
    match (program, code) {
        (_, Some(0)) => true,
        // SMART bits 3–7 report disk health; bits 0–2 mean the probe failed.
        ("smartctl", Some(code)) => (0..=255).contains(&code) && code & 7 == 0,
        ("dnf", Some(100)) => args.first() == Some(&"check-update"),
        ("checkupdates", Some(2)) => true,
        _ => false,
    }
}

/// Read `/proc` file contents, respecting the profile timeout.
async fn read_proc(path: &str, timeout_secs: Seconds) -> Result<String, std::io::Error> {
    tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::fs::read_to_string(path),
    )
    .await
    .unwrap_or_else(|_| Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout")))
}

/// Check if a binary is available on PATH.
fn bin_exists(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| {
            std::fs::metadata(directory.join(name)).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
    })
}

/// Make a CollectorError for a given domain.
fn err(domain: &str, msg: impl Into<String>) -> CollectorError {
    CollectorError {
        domain: domain.to_string(),
        message: msg.into(),
        is_timeout: false,
    }
}

/// Make a timeout CollectorError for a given domain.
fn timeout_err(domain: &str, msg: impl Into<String>) -> CollectorError {
    CollectorError {
        domain: domain.to_string(),
        message: msg.into(),
        is_timeout: true,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn expected_nonzero_statuses_are_narrowly_scoped() {
        assert!(super::accepted_exit(
            "smartctl",
            &["-H", "/dev/fixture"],
            Some(8)
        ));
        assert!(!super::accepted_exit("smartctl", &[], Some(9)));
        assert!(super::accepted_exit(
            "dnf",
            &["check-update", "-q"],
            Some(100)
        ));
        assert!(!super::accepted_exit("dnf", &["updateinfo"], Some(100)));
        assert!(super::accepted_exit("checkupdates", &[], Some(2)));
        assert!(!super::accepted_exit("systemctl", &[], Some(1)));
        assert!(!super::accepted_exit("sh", &[], None));
    }
    #[tokio::test]
    async fn nonzero_exit_is_a_collector_failure_with_diagnostics() {
        let result = super::run_timed(
            "sh",
            &["-c", "printf fixture-error >&2; exit 7"],
            crate::MonitorProfile::Quick,
        )
        .await;
        let error =
            result.expect_err("nonzero collector exit cannot become a healthy empty result");
        assert!(error.message.contains("7"));
        assert!(error.message.contains("fixture-error"));
    }

    #[test]
    fn binary_lookup_does_not_need_a_subprocess() {
        assert!(super::bin_exists("sh"));
        assert!(!super::bin_exists("helm-binary-that-does-not-exist-293057"));
    }
}
