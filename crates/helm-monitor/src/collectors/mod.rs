//! Typed, bounded, read-only system collectors.
//!
//! Every collector:
//! - Is typed and bounded (TRD §5).
//! - Uses a per-collector timeout from the profile.
//! - Never mutates system state.
//! - Tolerates missing binaries gracefully.
//! - Returns domain-specific errors in CollectorError format.

pub mod backups;
pub mod compose;
pub mod containers;
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
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout),
        Command::new(program).args(args).output(),
    )
    .await;

    match output {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(CollectorError {
            domain: String::new(),
            message: format!("{program}: {e}"),
            is_timeout: false,
        }),
        Err(_elapsed) => Err(CollectorError {
            domain: String::new(),
            message: format!("{program} timed out after {timeout}s"),
            is_timeout: true,
        }),
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
    std::process::Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
