//! Custom service registry loaded from `services.toml`.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{net::TcpStream, time::timeout};
use tracing::warn;
use uuid::Uuid;

use crate::findings::{Confidence, Finding, MonitorDomain, Severity};

const MAX_REGISTERED_SERVICES: usize = 256;
const MAX_CONNECTION_STRING_LEN: usize = 2048;
const HEALTH_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbKind {
    PostgreSQL,
    MySQL,
    Redis,
    MongoDB,
}

impl DbKind {
    fn default_port(self) -> u16 {
        match self {
            Self::PostgreSQL => 5432,
            Self::MySQL => 3306,
            Self::Redis => 6379,
            Self::MongoDB => 27017,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::PostgreSQL => "postgresql",
            Self::MySQL => "mysql",
            Self::Redis => "redis",
            Self::MongoDB => "mongodb",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ServiceType {
    DockerContainer {
        name_or_label: String,
    },
    Database {
        kind: DbKind,
        connection_string: String,
    },
    Kubernetes {
        kubeconfig_path: PathBuf,
    },
    Pipeline {
        endpoint: String,
    },
    WebServer {
        url: String,
        expected_status: u16,
    },
    TcpEndpoint {
        host: String,
        port: u16,
    },
    HttpEndpoint {
        url: String,
        #[serde(default)]
        expected_status: Option<u16>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredService {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub service_type: ServiceType,
    pub environment: String,
    pub team: String,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub last_healthy: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ServiceRegistry {
    services: Vec<RegisteredService>,
    config_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum ServiceRegistryError {
    #[error("failed to read {path}: {message}")]
    Read { path: String, message: String },
    #[error("failed to parse {path}: {message}")]
    Parse { path: String, message: String },
    #[error("invalid service `{service_id}`: {message}")]
    Validation { service_id: String, message: String },
    #[error("service `{0}` already exists")]
    DuplicateId(String),
    #[error("service `{0}` was not found")]
    NotFound(String),
    #[error("service registry cannot exceed {MAX_REGISTERED_SERVICES} entries")]
    TooManyServices,
    #[error("failed to persist {path}: {message}")]
    Persist { path: String, message: String },
}

#[derive(Debug, Clone)]
struct HealthCheckOutcome {
    healthy: bool,
    detail: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ServiceFile {
    #[serde(default)]
    service: Vec<RegisteredService>,
}

impl ServiceRegistry {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            services: Vec::new(),
            config_path: path.into(),
        }
    }

    pub fn load(path: &Path) -> Result<Self, ServiceRegistryError> {
        let path = path.to_path_buf();
        let Ok(content) = fs::read_to_string(&path) else {
            return Ok(Self::new(path));
        };
        let parsed = toml::from_str::<ServiceFile>(&content).map_err(|error| {
            ServiceRegistryError::Parse {
                path: path.display().to_string(),
                message: error.to_string(),
            }
        })?;
        let mut services = Vec::new();
        for service in parsed.service {
            match validate_service(&service) {
                Ok(()) => services.push(service),
                Err(error) => {
                    warn!(config = %path.display(), error = %error, "ignoring invalid service entry")
                }
            }
        }
        if services.len() > MAX_REGISTERED_SERVICES {
            return Err(ServiceRegistryError::TooManyServices);
        }
        Ok(Self {
            services,
            config_path: path,
        })
    }

    pub fn load_default() -> Result<Self, ServiceRegistryError> {
        Self::load(&default_services_path())
    }

    pub fn services(&self) -> &[RegisteredService] {
        &self.services
    }

    pub fn add(&mut self, service: RegisteredService) -> Result<(), ServiceRegistryError> {
        validate_service(&service)?;
        if self
            .services
            .iter()
            .any(|existing| existing.id == service.id)
        {
            return Err(ServiceRegistryError::DuplicateId(service.id));
        }
        if self.services.len() >= MAX_REGISTERED_SERVICES {
            return Err(ServiceRegistryError::TooManyServices);
        }
        self.services.push(service);
        self.persist()
    }

    pub fn remove(&mut self, id: &str) -> Result<(), ServiceRegistryError> {
        let original_len = self.services.len();
        self.services.retain(|service| service.id != id);
        if self.services.len() == original_len {
            return Err(ServiceRegistryError::NotFound(id.to_owned()));
        }
        self.persist()
    }

    pub async fn check_health(&mut self) -> Vec<Finding> {
        let mut outcomes = Vec::with_capacity(self.services.len());
        for service in &self.services {
            outcomes.push(check_service_health(service).await);
        }

        let mut findings = Vec::new();
        let now = Utc::now();
        for (service, outcome) in self.services.iter_mut().zip(outcomes.into_iter()) {
            if outcome.healthy {
                service.consecutive_failures = 0;
                service.last_healthy = Some(now);
                continue;
            }

            service.consecutive_failures = service.consecutive_failures.saturating_add(1);
            if let Some(severity) = escalation_severity(service.consecutive_failures) {
                findings.push(build_service_finding(service, severity, &outcome.detail));
            }
        }

        if let Err(error) = self.persist() {
            warn!(error = %error, "failed to persist service registry health state");
        }

        findings
    }

    fn persist(&self) -> Result<(), ServiceRegistryError> {
        let parent = self
            .config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        fs::create_dir_all(&parent).map_err(|error| ServiceRegistryError::Persist {
            path: parent.display().to_string(),
            message: error.to_string(),
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&parent, fs::Permissions::from_mode(0o700));
        }

        let payload = toml::to_string_pretty(&ServiceFile {
            service: self.services.clone(),
        })
        .map_err(|error| ServiceRegistryError::Persist {
            path: self.config_path.display().to_string(),
            message: error.to_string(),
        })?;

        let tmp_path = self
            .config_path
            .with_extension(format!("{}.tmp", Uuid::new_v4()));
        fs::write(&tmp_path, payload).map_err(|error| ServiceRegistryError::Persist {
            path: tmp_path.display().to_string(),
            message: error.to_string(),
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600)).map_err(|error| {
                ServiceRegistryError::Persist {
                    path: tmp_path.display().to_string(),
                    message: error.to_string(),
                }
            })?;
        }
        fs::rename(&tmp_path, &self.config_path).map_err(|error| {
            ServiceRegistryError::Persist {
                path: self.config_path.display().to_string(),
                message: error.to_string(),
            }
        })?;
        Ok(())
    }
}

pub fn default_services_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("helm")
        .join("services.toml")
}

fn validate_service(service: &RegisteredService) -> Result<(), ServiceRegistryError> {
    if service.id.trim().is_empty() {
        return Err(validation_error(service, "id must not be empty"));
    }
    if service.name.trim().is_empty() {
        return Err(validation_error(service, "name must not be empty"));
    }
    if service.environment.trim().is_empty() {
        return Err(validation_error(service, "environment must not be empty"));
    }
    if service.team.trim().is_empty() {
        return Err(validation_error(service, "team must not be empty"));
    }

    match &service.service_type {
        ServiceType::DockerContainer { name_or_label } => {
            if name_or_label.trim().is_empty() {
                return Err(validation_error(
                    service,
                    "docker container name_or_label must not be empty",
                ));
            }
        }
        ServiceType::Database {
            connection_string, ..
        } => {
            if connection_string.trim().is_empty() {
                return Err(validation_error(
                    service,
                    "database connection_string must not be empty",
                ));
            }
            if connection_string.len() > MAX_CONNECTION_STRING_LEN {
                return Err(validation_error(
                    service,
                    "database connection_string exceeds 2048 characters",
                ));
            }
            parse_connection_target(&service.service_type)
                .map_err(|message| validation_error(service, &message))?;
        }
        ServiceType::Kubernetes { kubeconfig_path } => {
            if kubeconfig_path.as_os_str().is_empty() {
                return Err(validation_error(
                    service,
                    "kubernetes kubeconfig_path must not be empty",
                ));
            }
        }
        ServiceType::Pipeline { endpoint } => {
            validate_url_field(service, "pipeline endpoint", endpoint)?
        }
        ServiceType::WebServer {
            url,
            expected_status,
        } => {
            validate_url_field(service, "web server url", url)?;
            if *expected_status < 100 || *expected_status > 599 {
                return Err(validation_error(
                    service,
                    "web server expected_status must be a valid HTTP code",
                ));
            }
        }
        ServiceType::TcpEndpoint { host, port } => {
            if host.trim().is_empty() {
                return Err(validation_error(service, "tcp host must not be empty"));
            }
            if *port == 0 {
                return Err(validation_error(service, "tcp port must be greater than 0"));
            }
        }
        ServiceType::HttpEndpoint {
            url,
            expected_status,
        } => {
            validate_url_field(service, "http endpoint url", url)?;
            if let Some(status) = expected_status
                && (*status < 100 || *status > 599)
            {
                return Err(validation_error(
                    service,
                    "http endpoint expected_status must be a valid HTTP code",
                ));
            }
        }
    }

    Ok(())
}

fn validate_url_field(
    service: &RegisteredService,
    field: &str,
    value: &str,
) -> Result<(), ServiceRegistryError> {
    if value.trim().is_empty() {
        return Err(validation_error(
            service,
            &format!("{field} must not be empty"),
        ));
    }
    let url = Url::parse(value)
        .map_err(|error| validation_error(service, &format!("{field} is invalid: {error}")))?;
    match url.scheme() {
        "http" | "https" => Ok(()),
        scheme => Err(validation_error(
            service,
            &format!("{field} must use http or https, got {scheme}"),
        )),
    }
}

fn validation_error(service: &RegisteredService, message: &str) -> ServiceRegistryError {
    ServiceRegistryError::Validation {
        service_id: service.id.clone(),
        message: message.to_owned(),
    }
}

fn parse_connection_target(service_type: &ServiceType) -> Result<(String, u16), String> {
    match service_type {
        ServiceType::Database {
            kind,
            connection_string,
        } => {
            let url = Url::parse(connection_string)
                .map_err(|error| format!("database connection_string is invalid: {error}"))?;
            let host = url
                .host_str()
                .map(str::to_owned)
                .ok_or_else(|| "database connection_string is missing a host".to_owned())?;
            let port = url.port().unwrap_or_else(|| kind.default_port());
            Ok((host, port))
        }
        _ => Err("service type does not contain a database connection string".to_owned()),
    }
}

fn escalation_severity(consecutive_failures: u32) -> Option<Severity> {
    match consecutive_failures {
        0..=2 => None,
        3 | 4 => Some(Severity::Warning),
        _ => Some(Severity::Critical),
    }
}

fn build_service_finding(service: &RegisteredService, severity: Severity, detail: &str) -> Finding {
    let category = service_monitor_domain(&service.service_type);
    let title = match severity {
        Severity::Critical => format!("{} is failing health checks repeatedly", service.name),
        Severity::Warning => format!("{} health checks are degraded", service.name),
        Severity::Info => format!("{} health checks need attention", service.name),
    };
    let impact = format!(
        "{} in {} is unhealthy; team {} should investigate.",
        service.name, service.environment, service.team
    );
    let check_hint = match &service.service_type {
        ServiceType::DockerContainer { name_or_label } => {
            format!("docker inspect {name_or_label}")
        }
        ServiceType::Database {
            kind,
            connection_string,
        } => format!(
            "check {} reachability for {}",
            kind.label(),
            connection_string
        ),
        ServiceType::Kubernetes { kubeconfig_path } => {
            format!(
                "kubectl cluster-info --kubeconfig {}",
                kubeconfig_path.display()
            )
        }
        ServiceType::Pipeline { endpoint } => format!("curl -fsS {endpoint}"),
        ServiceType::WebServer { url, .. } => format!("curl -I {url}"),
        ServiceType::TcpEndpoint { host, port } => format!("nc -vz {host} {port}"),
        ServiceType::HttpEndpoint { url, .. } => format!("curl -fsS {url}"),
    };

    Finding::new(
        "custom-service",
        "custom-service",
        &service.id,
        &title,
        severity,
        Confidence::High,
        category,
    )
    .with_evidence(
        "services.registry",
        detail,
        "custom service health check result",
    )
    .with_impact(impact)
    .with_read_only_check(check_hint)
    .normalize_for_emission("custom-service", Utc::now())
}

fn service_monitor_domain(service_type: &ServiceType) -> MonitorDomain {
    match service_type {
        ServiceType::DockerContainer { .. } => MonitorDomain::Containers,
        ServiceType::Database { .. } => MonitorDomain::Services,
        ServiceType::Kubernetes { .. } => MonitorDomain::Kubernetes,
        ServiceType::Pipeline { .. } => MonitorDomain::Services,
        ServiceType::WebServer { .. } => MonitorDomain::Services,
        ServiceType::TcpEndpoint { .. } => MonitorDomain::Network,
        ServiceType::HttpEndpoint { .. } => MonitorDomain::Network,
    }
}

async fn check_service_health(service: &RegisteredService) -> HealthCheckOutcome {
    match &service.service_type {
        ServiceType::DockerContainer { name_or_label } => {
            let output = timeout(
                Duration::from_secs(HEALTH_TIMEOUT_SECS),
                crate::process_runner::run_program("docker", &[
                        "inspect",
                        "--format",
                        "{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}",
                        name_or_label,
                    ], Duration::from_secs(HEALTH_TIMEOUT_SECS)),
            )
            .await;
            match output {
                Ok(Ok(output)) if output.status.success() => {
                    let status = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                    if matches!(status.as_str(), "healthy" | "running") {
                        HealthCheckOutcome {
                            healthy: true,
                            detail: format!("docker status={status}"),
                        }
                    } else {
                        HealthCheckOutcome {
                            healthy: false,
                            detail: format!("docker status={status}"),
                        }
                    }
                }
                Ok(Ok(output)) => HealthCheckOutcome {
                    healthy: false,
                    detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                },
                Ok(Err(error)) => HealthCheckOutcome {
                    healthy: false,
                    detail: format!("docker inspect failed: {error}"),
                },
                Err(_) => HealthCheckOutcome {
                    healthy: false,
                    detail: "docker inspect timed out".to_owned(),
                },
            }
        }
        ServiceType::Database {
            kind,
            connection_string,
        } => match parse_connection_target(&service.service_type) {
            Ok((host, port)) => {
                tcp_connect_health(
                    &host,
                    port,
                    &format!("{} {}", kind.label(), connection_string),
                )
                .await
            }
            Err(message) => HealthCheckOutcome {
                healthy: false,
                detail: message,
            },
        },
        ServiceType::Kubernetes { kubeconfig_path } => {
            let output = timeout(
                Duration::from_secs(HEALTH_TIMEOUT_SECS),
                crate::process_runner::run_program(
                    "kubectl",
                    &[
                        std::ffi::OsStr::new("cluster-info"),
                        std::ffi::OsStr::new("--kubeconfig"),
                        kubeconfig_path.as_os_str(),
                    ],
                    Duration::from_secs(HEALTH_TIMEOUT_SECS),
                ),
            )
            .await;
            match output {
                Ok(Ok(output)) if output.status.success() => HealthCheckOutcome {
                    healthy: true,
                    detail: "kubectl cluster-info succeeded".to_owned(),
                },
                Ok(Ok(output)) => HealthCheckOutcome {
                    healthy: false,
                    detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                },
                Ok(Err(error)) => HealthCheckOutcome {
                    healthy: false,
                    detail: format!("kubectl cluster-info failed: {error}"),
                },
                Err(_) => HealthCheckOutcome {
                    healthy: false,
                    detail: "kubectl cluster-info timed out".to_owned(),
                },
            }
        }
        ServiceType::Pipeline { endpoint } => http_health(endpoint, None).await,
        ServiceType::WebServer {
            url,
            expected_status,
        } => http_health(url, Some(*expected_status)).await,
        ServiceType::TcpEndpoint { host, port } => {
            tcp_connect_health(host, *port, &format!("{host}:{port}")).await
        }
        ServiceType::HttpEndpoint {
            url,
            expected_status,
        } => http_health(url, *expected_status).await,
    }
}

async fn tcp_connect_health(host: &str, port: u16, label: &str) -> HealthCheckOutcome {
    match timeout(
        Duration::from_secs(HEALTH_TIMEOUT_SECS),
        TcpStream::connect((host, port)),
    )
    .await
    {
        Ok(Ok(_)) => HealthCheckOutcome {
            healthy: true,
            detail: format!("tcp connect ok: {label}"),
        },
        Ok(Err(error)) => HealthCheckOutcome {
            healthy: false,
            detail: format!("tcp connect failed for {label}: {error}"),
        },
        Err(_) => HealthCheckOutcome {
            healthy: false,
            detail: format!("tcp connect timed out for {label}"),
        },
    }
}

async fn http_health(url: &str, expected_status: Option<u16>) -> HealthCheckOutcome {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(HEALTH_TIMEOUT_SECS))
        .build();
    let Ok(client) = client else {
        return HealthCheckOutcome {
            healthy: false,
            detail: "failed to build HTTP client".to_owned(),
        };
    };
    match client.get(url).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let healthy = match expected_status {
                Some(expected) => status == expected,
                None => (200..=299).contains(&status),
            };
            HealthCheckOutcome {
                healthy,
                detail: format!("http status={status} url={url}"),
            }
        }
        Err(error) => HealthCheckOutcome {
            healthy: false,
            detail: format!("http request failed for {url}: {error}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_services_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("helm_services_{name}_{}.toml", Uuid::new_v4()))
    }

    fn sample_service(id: &str) -> RegisteredService {
        RegisteredService {
            id: id.to_owned(),
            name: "Staging API".to_owned(),
            service_type: ServiceType::HttpEndpoint {
                url: "https://example.com/health".to_owned(),
                expected_status: Some(200),
            },
            environment: "staging".to_owned(),
            team: "platform".to_owned(),
            consecutive_failures: 0,
            last_healthy: None,
        }
    }

    #[test]
    fn missing_registry_loads_empty() {
        let path = temp_services_path("missing");
        let registry = ServiceRegistry::load(&path).unwrap();
        assert!(registry.services().is_empty());
        assert_eq!(registry.config_path, path);
    }

    #[test]
    fn add_persists_and_round_trips() {
        let path = temp_services_path("roundtrip");
        let mut registry = ServiceRegistry::new(&path);
        registry.add(sample_service("svc-1")).unwrap();

        let loaded = ServiceRegistry::load(&path).unwrap();
        assert_eq!(loaded.services().len(), 1);
        assert_eq!(loaded.services()[0].id, "svc-1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn invalid_service_registration_is_rejected() {
        let path = temp_services_path("invalid");
        let mut registry = ServiceRegistry::new(&path);
        let mut service = sample_service("svc-bad");
        service.service_type = ServiceType::Database {
            kind: DbKind::PostgreSQL,
            connection_string: "postgres://".repeat(300),
        };

        let error = registry.add(service).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("database connection_string exceeds 2048 characters")
        );
    }

    #[test]
    fn invalid_entries_are_ignored_during_load() {
        let path = temp_services_path("partial");
        fs::write(
            &path,
            r#"
[[service]]
id = "ok-1"
name = "Healthy API"
type = "http_endpoint"
url = "https://example.com/health"
expected_status = 200
environment = "staging"
team = "platform"

[[service]]
id = "bad-1"
name = ""
type = "tcp_endpoint"
host = "127.0.0.1"
port = 9000
environment = "staging"
team = "platform"
"#,
        )
        .unwrap();

        let registry = ServiceRegistry::load(&path).unwrap();
        assert_eq!(registry.services().len(), 1);
        assert_eq!(registry.services()[0].id, "ok-1");
    }

    #[test]
    fn failure_escalation_matches_thresholds() {
        assert_eq!(escalation_severity(2), None);
        assert_eq!(escalation_severity(3), Some(Severity::Warning));
        assert_eq!(escalation_severity(4), Some(Severity::Warning));
        assert_eq!(escalation_severity(5), Some(Severity::Critical));
        assert_eq!(escalation_severity(9), Some(Severity::Critical));
    }

    // ── Property tests ──────────────────────────────────────────────────────

    #[cfg(test)]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Property 1: Invalid service IDs (empty string) are always rejected.
            #[test]
            fn property_empty_id_rejected(_dummy in 0u8..10) {
                let path = temp_services_path("prop_empty_id");
                let mut registry = ServiceRegistry::new(&path);
                let mut svc = sample_service("");
                svc.id = String::new();
                let err = registry.add(svc).unwrap_err();
                prop_assert!(err.to_string().to_lowercase().contains("id"));
            }

            /// Property 2: Connection strings > 2048 chars are always rejected.
            #[test]
            fn property_oversized_connection_string_rejected(extra in 1usize..500) {
                let path = temp_services_path("prop_connstr");
                let mut registry = ServiceRegistry::new(&path);
                let conn = "x".repeat(2048 + extra);
                let mut svc = sample_service("svc-big");
                svc.service_type = ServiceType::Database {
                    kind: DbKind::PostgreSQL,
                    connection_string: conn,
                };
                let err = registry.add(svc).unwrap_err();
                prop_assert!(err.to_string().contains("2048"));
            }

            /// Property 3: Escalation severity is monotonically non-decreasing with
            /// failure count: once Critical is reached, it stays Critical.
            #[test]
            fn property_escalation_monotone(n in 5u32..100) {
                // At and above 5 failures, must be Critical.
                prop_assert_eq!(escalation_severity(n), Some(Severity::Critical));
            }

            /// Property 4: Below 3 failures, escalation is always None.
            #[test]
            fn property_no_escalation_below_threshold(n in 0u32..3) {
                prop_assert_eq!(escalation_severity(n), None);
            }
        }
    }
}
