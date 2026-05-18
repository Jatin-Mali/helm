//! Certificate collector: discovers TLS certificates from listening services.
//! Uses `openssl s_client` to fetch and parse certificates.

use crate::{
    CollectorError,
    collectors::{Collector, run_timed},
    snapshot::{CertEntry, CertSnapshot, MonitorProfile},
};

#[derive(Default)]
pub struct CertsCollector;

impl Collector for CertsCollector {
    type Output = CertSnapshot;

    fn domain(&self) -> &'static str {
        "certs"
    }

    async fn collect(self, profile: MonitorProfile) -> Result<Self::Output, CollectorError> {
        // Check if openssl is available
        let openssl_available = run_timed("which", &["openssl"], profile)
            .await
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);

        if !openssl_available {
            return Ok(CertSnapshot {
                certificates: Vec::new(),
                openssl_available: false,
            });
        }

        // Discover listening TLS ports
        let listeners = discover_tls_listeners(profile).await;

        let mut certificates = Vec::new();
        for addr in &listeners {
            match check_cert(addr, profile).await {
                Ok(cert) => certificates.push(cert),
                Err(cert_with_error) => certificates.push(cert_with_error),
            }
        }

        Ok(CertSnapshot {
            certificates,
            openssl_available: true,
        })
    }
}

/// Discover listening ports that are likely TLS services.
async fn discover_tls_listeners(profile: MonitorProfile) -> Vec<String> {
    let output = run_timed("ss", &["-tlnp"], profile).await;
    let stdout = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return Vec::new(),
    };

    let mut addrs = Vec::new();
    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }
        let local = parts[4];
        if let Some(pos) = local.rfind(':') {
            let port_str = &local[pos + 1..];
            let port: u16 = port_str.parse().unwrap_or(0);
            let is_tls_port =
                matches!(port, 443 | 8443 | 636 | 993 | 995 | 465 | 853 | 6697 | 5223);
            let proc_info = parts.get(6).unwrap_or(&"");
            let is_tls_proc = proc_info.contains("nginx")
                || proc_info.contains("apache")
                || proc_info.contains("haproxy")
                || proc_info.contains("caddy")
                || proc_info.contains("traefik")
                || proc_info.contains("envoy");

            if is_tls_port || is_tls_proc {
                let host = &local[..pos];
                let host = if host == "0.0.0.0" || host == "[::]" || host == "*" {
                    "127.0.0.1"
                } else if host.starts_with('[') {
                    &host[1..host.len() - 1]
                } else {
                    host
                };
                if port > 1024 && !is_tls_proc {
                    continue;
                }
                addrs.push(format!("{}:{}", host, port));
            }
        }
    }
    addrs.sort();
    addrs.dedup();
    addrs
}

/// Check a TLS certificate using openssl s_client piped to openssl x509.
#[allow(clippy::result_large_err, clippy::manual_strip)]
async fn check_cert(addr: &str, profile: MonitorProfile) -> Result<CertEntry, CertEntry> {
    let servername = addr.split(':').next().unwrap_or("localhost");
    let shell_cmd = format!(
        "echo '' | openssl s_client -connect {} -servername {} 2>/dev/null | openssl x509 -noout -dates -subject -issuer -ext subjectAltName 2>/dev/null",
        addr, servername
    );

    let timeout = profile.per_collector_timeout();
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&shell_cmd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();

    let output = match tokio::time::timeout(std::time::Duration::from_secs(timeout), output).await {
        Ok(Ok(o)) => o,
        _ => {
            return Err(CertEntry {
                address: addr.to_string(),
                subject_cn: String::new(),
                issuer_cn: String::new(),
                not_after: String::new(),
                days_remaining: i64::MAX,
                sans: Vec::new(),
                self_signed: false,
                error: "timeout or spawn error".to_string(),
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Err(CertEntry {
            address: addr.to_string(),
            subject_cn: String::new(),
            issuer_cn: String::new(),
            not_after: String::new(),
            days_remaining: i64::MAX,
            sans: Vec::new(),
            self_signed: false,
            error: "no certificate or connection failed".to_string(),
        });
    }

    parse_x509_output(addr, &stdout)
}

#[allow(clippy::result_large_err, clippy::manual_strip)]
fn parse_x509_output(addr: &str, output: &str) -> Result<CertEntry, CertEntry> {
    let mut subject_cn = String::new();
    let mut issuer_cn = String::new();
    let mut not_after = String::new();
    let mut sans: Vec<String> = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(subj) = trimmed.strip_prefix("subject=") {
            for part in subj.split(',') {
                let part = part.trim();
                if part.starts_with("CN = ") || part.starts_with("CN=") {
                    subject_cn = part.split('=').nth(1).unwrap_or("").trim().to_string();
                }
            }
        } else if let Some(iss) = trimmed.strip_prefix("issuer=") {
            for part in iss.split(',') {
                let part = part.trim();
                if part.starts_with("CN = ") || part.starts_with("CN=") {
                    issuer_cn = part.split('=').nth(1).unwrap_or("").trim().to_string();
                }
            }
        } else if trimmed.starts_with("notAfter=") {
            not_after = trimmed["notAfter=".len()..].trim().to_string();
        } else if trimmed.starts_with("DNS:") {
            sans.push(trimmed.to_string());
        }
    }

    let self_signed = subject_cn == issuer_cn && !subject_cn.is_empty();
    let days_remaining = if not_after.is_empty() {
        i64::MAX
    } else {
        days_until(&not_after)
    };

    Ok(CertEntry {
        address: addr.to_string(),
        subject_cn,
        issuer_cn,
        not_after,
        days_remaining,
        sans,
        self_signed,
        error: String::new(),
    })
}

fn days_until(not_after: &str) -> i64 {
    let date_str = not_after.trim_end_matches(" GMT");
    let now = chrono::Utc::now().naive_utc();
    // Try openssl format: "May 19 12:00:00 2026"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(date_str, "%b %e %H:%M:%S %Y") {
        return (dt - now).num_days();
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(date_str, "%B %e %H:%M:%S %Y") {
        return (dt - now).num_days();
    }
    // ISO 8601 fallback — parse as FixedOffset then convert to naive
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(not_after) {
        let naive_dt = dt.naive_utc();
        return (naive_dt - now).num_days();
    }
    0
}
