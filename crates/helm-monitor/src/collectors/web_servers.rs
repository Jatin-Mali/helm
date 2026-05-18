//! Web server site summary collector: nginx/Caddy vhost discovery.
//! Parses config files to enumerate served sites and TLS status.

use crate::{
    collectors::{run_timed, Collector},
    snapshot::MonitorProfile,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSite {
    pub server_name: String,
    pub listen: String,
    pub tls: bool,
    pub tls_cert: Option<String>,
    pub root: Option<String>,
    pub proxy_pass: Option<String>,
    pub server_type: String, // "nginx" or "caddy"
    pub config_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebServerSnapshot {
    pub sites: Vec<WebSite>,
    pub nginx_found: bool,
    pub caddy_found: bool,
}

#[derive(Default)]
pub struct WebServerCollector;

impl Collector for WebServerCollector {
    type Output = WebServerSnapshot;

    fn domain(&self) -> &'static str {
        "web-servers"
    }

    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut snapshot = WebServerSnapshot::default();

        // Check for nginx
        snapshot.nginx_found = run_timed("which", &["nginx"], profile)
            .await
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);

        if snapshot.nginx_found {
            if let Ok(sites) = parse_nginx_sites().await {
                snapshot.sites.extend(sites);
            }
        }

        // Check for caddy
        snapshot.caddy_found = run_timed("which", &["caddy"], profile)
            .await
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);

        if snapshot.caddy_found {
            if let Ok(sites) = parse_caddy_sites().await {
                snapshot.sites.extend(sites);
            }
        }

        Ok(snapshot)
    }
}

async fn parse_nginx_sites() -> Result<Vec<WebSite>, std::io::Error> {
    let mut sites = Vec::new();

    // Common nginx config locations
    let config_dirs = [
        "/etc/nginx/sites-enabled",
        "/etc/nginx/conf.d",
    ];

    for dir in &config_dirs {
        let path = Path::new(dir);
        if !path.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let file_path = entry.path();
                if file_path.extension().map_or(false, |e| e == "conf") || 
                   file_path.file_name().map_or(false, |n| n == "default") {
                    if let Ok(content) = std::fs::read_to_string(&file_path) {
                        let mut server_name = String::new();
                        let mut listen = "80".to_string();
                        let mut tls = false;
                        let mut tls_cert = None;
                        let mut root = None;
                        let mut proxy_pass = None;

                        for line in content.lines() {
                            let trimmed = line.trim();
                            if trimmed.starts_with("server_name") {
                                server_name = trimmed
                                    .strip_prefix("server_name")
                                    .unwrap_or("")
                                    .trim()
                                    .trim_end_matches(';')
                                    .to_string();
                            }
                            if trimmed.starts_with("listen") {
                                let lp = trimmed
                                    .strip_prefix("listen")
                                    .unwrap_or("")
                                    .trim()
                                    .trim_end_matches(';');
                                listen = lp.to_string();
                                tls = lp.contains("443") || lp.contains("ssl");
                            }
                            if trimmed.starts_with("ssl_certificate ") && !trimmed.contains("ssl_certificate_key") {
                                tls_cert = Some(
                                    trimmed
                                        .strip_prefix("ssl_certificate")
                                        .unwrap_or("")
                                        .trim()
                                        .trim_end_matches(';')
                                        .to_string(),
                                );
                            }
                            if trimmed.starts_with("root ") {
                                root = Some(
                                    trimmed
                                        .strip_prefix("root")
                                        .unwrap_or("")
                                        .trim()
                                        .trim_end_matches(';')
                                        .to_string(),
                                );
                            }
                            if trimmed.starts_with("proxy_pass ") {
                                proxy_pass = Some(
                                    trimmed
                                        .strip_prefix("proxy_pass")
                                        .unwrap_or("")
                                        .trim()
                                        .trim_end_matches(';')
                                        .to_string(),
                                );
                            }
                        }

                        if !server_name.is_empty() || !listen.is_empty() {
                            sites.push(WebSite {
                                server_name,
                                listen,
                                tls,
                                tls_cert,
                                root,
                                proxy_pass,
                                server_type: "nginx".into(),
                                config_file: file_path.display().to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(sites)
}

async fn parse_caddy_sites() -> Result<Vec<WebSite>, std::io::Error> {
    let mut sites = Vec::new();
    let caddyfile = Path::new("/etc/caddy/Caddyfile");

    if caddyfile.exists() {
        if let Ok(content) = std::fs::read_to_string(caddyfile) {
            let mut current_server = String::new();
            let mut current_tls = false;
            let mut current_proxy: Option<String> = None;

            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('{') || trimmed.starts_with('}') {
                    continue;
                }
                // Site block starts with domain
                if !trimmed.starts_with("tls ") && !trimmed.starts_with("reverse_proxy ") {
                    if !current_server.is_empty() {
                        sites.push(WebSite {
                            server_name: current_server.clone(),
                            listen: if current_tls { "443".into() } else { "80".into() },
                            tls: current_tls,
                            tls_cert: None,
                            root: None,
                            proxy_pass: current_proxy.take(),
                            server_type: "caddy".into(),
                            config_file: caddyfile.display().to_string(),
                        });
                    }
                    current_server = trimmed.to_string();
                    current_tls = false;
                    current_proxy = None;
                } else if trimmed.starts_with("tls ") {
                    current_tls = true;
                } else if trimmed.starts_with("reverse_proxy ") {
                    current_proxy = Some(
                        trimmed
                            .strip_prefix("reverse_proxy ")
                            .unwrap_or("")
                            .to_string(),
                    );
                }
            }

            if !current_server.is_empty() {
                sites.push(WebSite {
                    server_name: current_server,
                    listen: if current_tls { "443".into() } else { "80".into() },
                    tls: current_tls,
                    tls_cert: None,
                    root: None,
                    proxy_pass: current_proxy,
                    server_type: "caddy".into(),
                    config_file: caddyfile.display().to_string(),
                });
            }
        }
    }
    Ok(sites)
}
