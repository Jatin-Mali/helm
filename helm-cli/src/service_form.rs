use clap::ValueEnum;
use helm_monitor::{DbKind, RegisteredService, ServiceType};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ServiceKind {
    DockerContainer,
    Database,
    Kubernetes,
    Pipeline,
    WebServer,
    TcpEndpoint,
    HttpEndpoint,
}

impl ServiceKind {
    pub(crate) fn all() -> &'static [Self] {
        &[
            Self::DockerContainer,
            Self::Database,
            Self::Kubernetes,
            Self::Pipeline,
            Self::WebServer,
            Self::TcpEndpoint,
            Self::HttpEndpoint,
        ]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::DockerContainer => "docker_container",
            Self::Database => "database",
            Self::Kubernetes => "kubernetes",
            Self::Pipeline => "pipeline",
            Self::WebServer => "web_server",
            Self::TcpEndpoint => "tcp_endpoint",
            Self::HttpEndpoint => "http_endpoint",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceFormField {
    Name,
    Id,
    ServiceKind,
    Environment,
    Team,
    DockerNameOrLabel,
    DatabaseKind,
    DatabaseConnectionString,
    KubeconfigPath,
    PipelineEndpoint,
    WebUrl,
    WebExpectedStatus,
    TcpHost,
    TcpPort,
    HttpUrl,
    HttpExpectedStatus,
    Submit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceFieldSpec {
    pub(crate) field: ServiceFormField,
    pub(crate) label: &'static str,
    pub(crate) value: String,
    pub(crate) hint: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceRegistrationDraft {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) service_kind: ServiceKind,
    pub(crate) environment: String,
    pub(crate) team: String,
    pub(crate) docker_name_or_label: String,
    pub(crate) database_kind: DbKind,
    pub(crate) database_connection_string: String,
    pub(crate) kubeconfig_path: String,
    pub(crate) pipeline_endpoint: String,
    pub(crate) web_url: String,
    pub(crate) web_expected_status: String,
    pub(crate) tcp_host: String,
    pub(crate) tcp_port: String,
    pub(crate) http_url: String,
    pub(crate) http_expected_status: String,
}

impl Default for ServiceRegistrationDraft {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            service_kind: ServiceKind::HttpEndpoint,
            environment: "production".to_owned(),
            team: "platform".to_owned(),
            docker_name_or_label: String::new(),
            database_kind: DbKind::PostgreSQL,
            database_connection_string: String::new(),
            kubeconfig_path: String::new(),
            pipeline_endpoint: String::new(),
            web_url: String::new(),
            web_expected_status: "200".to_owned(),
            tcp_host: String::new(),
            tcp_port: "443".to_owned(),
            http_url: String::new(),
            http_expected_status: "200".to_owned(),
        }
    }
}

impl ServiceRegistrationDraft {
    pub(crate) fn preview_id(&self) -> String {
        if !self.id.trim().is_empty() {
            return self.id.trim().to_owned();
        }
        let derived = slugify_service_id(&self.name);
        if derived.is_empty() {
            "auto-generated after name".to_owned()
        } else {
            derived
        }
    }

    pub(crate) fn field_specs(&self) -> Vec<ServiceFieldSpec> {
        let mut fields = vec![
            ServiceFieldSpec {
                field: ServiceFormField::Name,
                label: "Name",
                value: self.name.clone(),
                hint: "human-readable display name",
            },
            ServiceFieldSpec {
                field: ServiceFormField::Id,
                label: "Id",
                value: self.preview_id(),
                hint: "leave blank to derive from the name",
            },
            ServiceFieldSpec {
                field: ServiceFormField::ServiceKind,
                label: "Type",
                value: self.service_kind.label().to_owned(),
                hint: "Left/Right cycles service type",
            },
            ServiceFieldSpec {
                field: ServiceFormField::Environment,
                label: "Environment",
                value: self.environment.clone(),
                hint: "production, staging, development, or custom",
            },
            ServiceFieldSpec {
                field: ServiceFormField::Team,
                label: "Team",
                value: self.team.clone(),
                hint: "owning team label",
            },
        ];

        match self.service_kind {
            ServiceKind::DockerContainer => {
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::DockerNameOrLabel,
                    label: "Container",
                    value: self.docker_name_or_label.clone(),
                    hint: "docker container name or label",
                });
            }
            ServiceKind::Database => {
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::DatabaseKind,
                    label: "Database",
                    value: format!("{:?}", self.database_kind).to_ascii_lowercase(),
                    hint: "Left/Right cycles database type",
                });
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::DatabaseConnectionString,
                    label: "Connection",
                    value: self.database_connection_string.clone(),
                    hint: "full connection string",
                });
            }
            ServiceKind::Kubernetes => {
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::KubeconfigPath,
                    label: "Kubeconfig",
                    value: self.kubeconfig_path.clone(),
                    hint: "path to kubeconfig",
                });
            }
            ServiceKind::Pipeline => {
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::PipelineEndpoint,
                    label: "Endpoint",
                    value: self.pipeline_endpoint.clone(),
                    hint: "HTTP endpoint returning 2xx on success",
                });
            }
            ServiceKind::WebServer => {
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::WebUrl,
                    label: "URL",
                    value: self.web_url.clone(),
                    hint: "http:// or https:// URL",
                });
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::WebExpectedStatus,
                    label: "Expected",
                    value: self.web_expected_status.clone(),
                    hint: "HTTP status code",
                });
            }
            ServiceKind::TcpEndpoint => {
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::TcpHost,
                    label: "Host",
                    value: self.tcp_host.clone(),
                    hint: "hostname or IP",
                });
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::TcpPort,
                    label: "Port",
                    value: self.tcp_port.clone(),
                    hint: "TCP port",
                });
            }
            ServiceKind::HttpEndpoint => {
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::HttpUrl,
                    label: "URL",
                    value: self.http_url.clone(),
                    hint: "HTTP health endpoint",
                });
                fields.push(ServiceFieldSpec {
                    field: ServiceFormField::HttpExpectedStatus,
                    label: "Expected",
                    value: self.http_expected_status.clone(),
                    hint: "blank means any 2xx status",
                });
            }
        }

        fields.push(ServiceFieldSpec {
            field: ServiceFormField::Submit,
            label: "Save",
            value: "register service".to_owned(),
            hint: "press Enter to persist",
        });
        fields
    }

    pub(crate) fn text_field_mut(&mut self, field: ServiceFormField) -> Option<&mut String> {
        match field {
            ServiceFormField::Name => Some(&mut self.name),
            ServiceFormField::Id => Some(&mut self.id),
            ServiceFormField::Environment => Some(&mut self.environment),
            ServiceFormField::Team => Some(&mut self.team),
            ServiceFormField::DockerNameOrLabel => Some(&mut self.docker_name_or_label),
            ServiceFormField::DatabaseConnectionString => {
                Some(&mut self.database_connection_string)
            }
            ServiceFormField::KubeconfigPath => Some(&mut self.kubeconfig_path),
            ServiceFormField::PipelineEndpoint => Some(&mut self.pipeline_endpoint),
            ServiceFormField::WebUrl => Some(&mut self.web_url),
            ServiceFormField::WebExpectedStatus => Some(&mut self.web_expected_status),
            ServiceFormField::TcpHost => Some(&mut self.tcp_host),
            ServiceFormField::TcpPort => Some(&mut self.tcp_port),
            ServiceFormField::HttpUrl => Some(&mut self.http_url),
            ServiceFormField::HttpExpectedStatus => Some(&mut self.http_expected_status),
            ServiceFormField::ServiceKind
            | ServiceFormField::DatabaseKind
            | ServiceFormField::Submit => None,
        }
    }

    pub(crate) fn cycle_field(&mut self, field: ServiceFormField, delta: i32) -> bool {
        match field {
            ServiceFormField::ServiceKind => {
                let all = ServiceKind::all();
                let current = all
                    .iter()
                    .position(|kind| *kind == self.service_kind)
                    .unwrap_or(0) as i32;
                let next = (current + delta).rem_euclid(all.len() as i32) as usize;
                self.service_kind = all[next];
                true
            }
            ServiceFormField::DatabaseKind => {
                let all = [
                    DbKind::PostgreSQL,
                    DbKind::MySQL,
                    DbKind::Redis,
                    DbKind::MongoDB,
                ];
                let current = all
                    .iter()
                    .position(|kind| *kind == self.database_kind)
                    .unwrap_or(0) as i32;
                let next = (current + delta).rem_euclid(all.len() as i32) as usize;
                self.database_kind = all[next];
                true
            }
            _ => false,
        }
    }

    pub(crate) fn build_service(&self) -> Result<RegisteredService, String> {
        let id = if self.id.trim().is_empty() {
            let derived = slugify_service_id(&self.name);
            if derived.is_empty() {
                let random = Uuid::new_v4().simple().to_string();
                format!("service-{}", &random[..8])
            } else {
                derived
            }
        } else {
            self.id.trim().to_owned()
        };

        let service_type = match self.service_kind {
            ServiceKind::DockerContainer => ServiceType::DockerContainer {
                name_or_label: self.docker_name_or_label.trim().to_owned(),
            },
            ServiceKind::Database => ServiceType::Database {
                kind: self.database_kind,
                connection_string: self.database_connection_string.trim().to_owned(),
            },
            ServiceKind::Kubernetes => ServiceType::Kubernetes {
                kubeconfig_path: self.kubeconfig_path.trim().into(),
            },
            ServiceKind::Pipeline => ServiceType::Pipeline {
                endpoint: self.pipeline_endpoint.trim().to_owned(),
            },
            ServiceKind::WebServer => ServiceType::WebServer {
                url: self.web_url.trim().to_owned(),
                expected_status: parse_required_status(&self.web_expected_status)?,
            },
            ServiceKind::TcpEndpoint => ServiceType::TcpEndpoint {
                host: self.tcp_host.trim().to_owned(),
                port: parse_port(&self.tcp_port)?,
            },
            ServiceKind::HttpEndpoint => ServiceType::HttpEndpoint {
                url: self.http_url.trim().to_owned(),
                expected_status: parse_optional_status(&self.http_expected_status)?,
            },
        };

        Ok(RegisteredService {
            id,
            name: self.name.trim().to_owned(),
            service_type,
            environment: self.environment.trim().to_owned(),
            team: self.team.trim().to_owned(),
            consecutive_failures: 0,
            last_healthy: None,
        })
    }
}

fn parse_required_status(raw: &str) -> Result<u16, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("expected HTTP status must not be empty".to_owned());
    }
    trimmed
        .parse::<u16>()
        .map_err(|_| format!("invalid HTTP status `{trimmed}`"))
}

fn parse_optional_status(raw: &str) -> Result<Option<u16>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<u16>()
        .map(Some)
        .map_err(|_| format!("invalid HTTP status `{trimmed}`"))
}

fn parse_port(raw: &str) -> Result<u16, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("TCP port must not be empty".to_owned());
    }
    let port = trimmed
        .parse::<u16>()
        .map_err(|_| format!("invalid TCP port `{trimmed}`"))?;
    if port == 0 {
        return Err("TCP port must be greater than 0".to_owned());
    }
    Ok(port)
}

fn slugify_service_id(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_slug_id_from_name() {
        let draft = ServiceRegistrationDraft {
            name: "Payments API Prod".to_owned(),
            ..ServiceRegistrationDraft::default()
        };
        assert_eq!(draft.preview_id(), "payments-api-prod");
    }

    #[test]
    fn builds_http_endpoint_service() {
        let draft = ServiceRegistrationDraft {
            name: "API".to_owned(),
            environment: "production".to_owned(),
            team: "platform".to_owned(),
            service_kind: ServiceKind::HttpEndpoint,
            http_url: "https://example.com/health".to_owned(),
            http_expected_status: "204".to_owned(),
            ..ServiceRegistrationDraft::default()
        };

        let service = draft.build_service().unwrap();
        assert_eq!(service.id, "api");
        match service.service_type {
            ServiceType::HttpEndpoint {
                url,
                expected_status,
            } => {
                assert_eq!(url, "https://example.com/health");
                assert_eq!(expected_status, Some(204));
            }
            other => panic!("unexpected service type: {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_port_values() {
        let draft = ServiceRegistrationDraft {
            name: "TCP".to_owned(),
            environment: "production".to_owned(),
            team: "ops".to_owned(),
            service_kind: ServiceKind::TcpEndpoint,
            tcp_host: "127.0.0.1".to_owned(),
            tcp_port: "nope".to_owned(),
            ..ServiceRegistrationDraft::default()
        };

        let error = draft.build_service().unwrap_err();
        assert!(error.contains("invalid TCP port"));
    }
}
