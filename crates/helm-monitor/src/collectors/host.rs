//! Host identity collector: OS, kernel, hostname, uptime.

use crate::{
    collectors::{Collector, err, read_proc, timeout_err},
    snapshot::{HostIdentity, MonitorProfile},
};

#[derive(Default)]
pub struct HostCollector;

impl Collector for HostCollector {
    type Output = HostIdentity;

    fn domain(&self) -> &'static str {
        "host"
    }

    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let t = profile.per_collector_timeout();
        let mut out = HostIdentity::default();

        let u = tokio::task::spawn_blocking(move || {
            std::process::Command::new("uname").arg("-n").output()
        })
        .await
        .map_err(|e| err("host", e.to_string()))?;
        if let Ok(o) = u {
            if o.status.success() {
                out.hostname = String::from_utf8_lossy(&o.stdout).trim().to_string();
            }
        }
        let u = tokio::task::spawn_blocking(move || {
            std::process::Command::new("uname").arg("-s").output()
        })
        .await
        .map_err(|e| err("host", e.to_string()))?;
        if let Ok(o) = u {
            if o.status.success() {
                out.kernel_name = String::from_utf8_lossy(&o.stdout).trim().to_string();
            }
        }
        let u = tokio::task::spawn_blocking(move || {
            std::process::Command::new("uname").arg("-r").output()
        })
        .await
        .map_err(|e| err("host", e.to_string()))?;
        if let Ok(o) = u {
            if o.status.success() {
                out.kernel_release = String::from_utf8_lossy(&o.stdout).trim().to_string();
            }
        }
        let u = tokio::task::spawn_blocking(move || {
            std::process::Command::new("uname").arg("-m").output()
        })
        .await
        .map_err(|e| err("host", e.to_string()))?;
        if let Ok(o) = u {
            if o.status.success() {
                out.machine = String::from_utf8_lossy(&o.stdout).trim().to_string();
            }
        }

        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
                    out.os_pretty_name = Some(v.trim_matches('"').to_string());
                } else if let Some(v) = line.strip_prefix("ID=") {
                    out.os_id = Some(v.trim_matches('"').to_string());
                } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
                    out.os_version_id = Some(v.trim_matches('"').to_string());
                }
            }
        }
        match read_proc("/proc/uptime", t).await {
            Ok(c) => {
                if let Some(f) = c.split_whitespace().next() {
                    out.uptime_seconds = f.parse::<f64>().map(|v| v as u64).unwrap_or(0);
                }
            }
            Err(e) => return Err(timeout_err("host", format!("/proc/uptime: {e}"))),
        }
        Ok(out)
    }
}
