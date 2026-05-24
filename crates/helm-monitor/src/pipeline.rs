//! Pipeline monitoring and webhook-backed CI/CD history.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::task::JoinSet;
use tracing::warn;

use crate::{
    custom_service::{RegisteredService, ServiceRegistry, ServiceType},
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::{CollectorError, MonitorProfile},
};

const MAX_RUNS_PER_PIPELINE: usize = 50;
const FETCH_TIMEOUT_SECS: u64 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PipelineProvider {
    GitHubActions,
    GitLabCi,
    Jenkins,
    #[default]
    Generic,
}

impl PipelineProvider {
    fn from_hint(endpoint: &str) -> Self {
        let hint = endpoint.to_ascii_lowercase();
        if hint.contains("github") || hint.contains("workflow") || hint.contains("actions") {
            Self::GitHubActions
        } else if hint.contains("gitlab") || hint.contains("/pipelines") {
            Self::GitLabCi
        } else if hint.contains("jenkins") {
            Self::Jenkins
        } else {
            Self::Generic
        }
    }

    fn from_webhook_provider(provider: &str) -> Self {
        match provider.to_ascii_lowercase().as_str() {
            "github" | "github_actions" | "github-actions" => Self::GitHubActions,
            "gitlab" | "gitlab_ci" | "gitlab-ci" => Self::GitLabCi,
            "jenkins" => Self::Jenkins,
            _ => Self::Generic,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::GitHubActions => "github_actions",
            Self::GitLabCi => "gitlab_ci",
            Self::Jenkins => "jenkins",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PipelineSnapshot {
    #[serde(default)]
    pub pipelines: Vec<PipelineStatus>,
    #[serde(default)]
    pub deployment_frequency_per_day: f64,
    #[serde(default)]
    pub change_failure_rate: f64,
    #[serde(default)]
    pub running_builds: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineStatus {
    pub service_id: String,
    pub name: String,
    pub environment: String,
    pub team: String,
    pub provider: PipelineProvider,
    pub endpoint: String,
    pub webhook_path: String,
    pub last_status: String,
    pub last_duration_secs: Option<u64>,
    #[serde(default)]
    pub duration_trend_secs: Vec<u64>,
    pub failure_rate: f64,
    pub running_builds: u32,
    pub last_stage: Option<String>,
    pub log_reference: Option<String>,
    pub last_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineRun {
    pub run_id: String,
    pub status: String,
    pub stage: Option<String>,
    pub duration_secs: Option<u64>,
    pub log_reference: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub source: String,
    pub is_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PipelineHistoryFile {
    #[serde(default)]
    pipeline: Vec<PipelineHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PipelineHistoryEntry {
    service_id: String,
    name: String,
    endpoint: String,
    provider: PipelineProvider,
    #[serde(default)]
    runs: Vec<PipelineRun>,
}

#[derive(Debug, Clone)]
struct PipelineProbe {
    provider: PipelineProvider,
    runs: Vec<PipelineRun>,
}

#[derive(Debug, Error)]
pub enum PipelineWebhookError {
    #[error("service registry is unavailable: {0}")]
    Registry(String),
    #[error("pipeline service `{0}` was not found")]
    NotFound(String),
    #[error("service `{0}` is not a pipeline")]
    NotPipeline(String),
    #[error("webhook payload could not be parsed for `{0}`")]
    Parse(String),
    #[error("failed to persist pipeline history: {0}")]
    Persist(String),
}

pub struct PipelineCollector;

impl PipelineCollector {
    pub async fn collect_default(
        _profile: MonitorProfile,
    ) -> Result<PipelineSnapshot, CollectorError> {
        let registry = ServiceRegistry::load_default()
            .map_err(|error| collector_error(format!("failed to load services.toml: {error}")))?;
        Self::collect_with_history_path(registry.services(), &default_pipeline_history_path()).await
    }

    async fn collect_with_history_path(
        services: &[RegisteredService],
        history_path: &Path,
    ) -> Result<PipelineSnapshot, CollectorError> {
        let pipeline_services = services
            .iter()
            .filter_map(|service| match &service.service_type {
                ServiceType::Pipeline { endpoint } => Some((service.clone(), endpoint.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();

        if pipeline_services.is_empty() {
            return Ok(PipelineSnapshot::default());
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
            .build()
            .map_err(|error| collector_error(format!("reqwest client init failed: {error}")))?;

        let mut history = load_history(history_path);
        let mut join_set = JoinSet::new();
        for (service, endpoint) in &pipeline_services {
            let client = client.clone();
            let service = service.clone();
            let endpoint = endpoint.clone();
            join_set.spawn(async move {
                let probe = fetch_pipeline_probe(&client, &service, &endpoint).await;
                (service, probe)
            });
        }

        while let Some(joined) = join_set.join_next().await {
            match joined {
                Ok((service, Ok(probe))) => {
                    merge_probe(&mut history, &service, probe);
                }
                Ok((service, Err(error))) => {
                    warn!(service_id = %service.id, error = %error, "pipeline probe failed");
                }
                Err(error) => warn!(error = %error, "pipeline probe join failed"),
            }
        }

        persist_history(history_path, &history).map_err(|error| {
            collector_error(format!("failed to persist pipeline history: {error}"))
        })?;

        Ok(build_snapshot(&pipeline_services, &history))
    }
}

pub fn default_pipeline_history_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("helm")
        .join("pipeline-history.json")
}

pub fn record_pipeline_webhook_event(
    provider: &str,
    service_id: &str,
    payload: &Value,
) -> Result<(), PipelineWebhookError> {
    let registry = ServiceRegistry::load_default()
        .map_err(|error| PipelineWebhookError::Registry(error.to_string()))?;
    let service = registry
        .services()
        .iter()
        .find(|service| service.id == service_id)
        .cloned()
        .ok_or_else(|| PipelineWebhookError::NotFound(service_id.to_owned()))?;

    let ServiceType::Pipeline { endpoint } = &service.service_type else {
        return Err(PipelineWebhookError::NotPipeline(service_id.to_owned()));
    };

    let provider = PipelineProvider::from_webhook_provider(provider);
    let runs = parse_webhook_runs(payload, &provider, endpoint);
    if runs.is_empty() {
        return Err(PipelineWebhookError::Parse(service_id.to_owned()));
    }

    let mut history = load_history(&default_pipeline_history_path());
    merge_probe(&mut history, &service, PipelineProbe { provider, runs });
    persist_history(&default_pipeline_history_path(), &history)
        .map_err(|error| PipelineWebhookError::Persist(error.to_string()))
}

pub fn pipeline_failure_findings(
    snapshot: &PipelineSnapshot,
    snapshot_id: &str,
    collected_at: DateTime<Utc>,
) -> Vec<Finding> {
    snapshot
        .pipelines
        .iter()
        .filter(|pipeline| is_failure_status(&pipeline.last_status))
        .map(|pipeline| {
            let display_name = pipeline_display_name(&pipeline.name, &pipeline.service_id);
            let stage = pipeline
                .last_stage
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "unknown".to_owned());
            let logs = pipeline
                .log_reference
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| pipeline.endpoint.clone());
            Finding::new(
                snapshot_id,
                "pipeline-failure",
                &pipeline.service_id,
                &format!("Pipeline {} failed in {}", display_name, stage),
                Severity::Critical,
                Confidence::High,
                MonitorDomain::Services,
            )
            .with_evidence(
                &format!("pipelines.{}.last_status", pipeline.service_id),
                &pipeline.last_status,
                "latest pipeline execution status",
            )
            .with_evidence(
                &format!("pipelines.{}.failure_stage", pipeline.service_id),
                &stage,
                "pipeline stage reported as failing",
            )
            .with_evidence(
                &format!("pipelines.{}.logs", pipeline.service_id),
                &logs,
                "pipeline log reference",
            )
            .with_impact(format!(
                "Pipeline {} in {} is failing at stage {} for team {}.",
                pipeline.name, pipeline.environment, stage, pipeline.team
            ))
            .with_read_only_check(format!("curl -fsS {}", pipeline.endpoint))
            .normalize_for_emission("pipeline-failure", collected_at)
        })
        .collect()
}

fn pipeline_display_name(name: &str, service_id: &str) -> String {
    let cleaned = name
        .replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        service_id.to_owned()
    } else {
        cleaned
    }
}

fn collector_error(message: String) -> CollectorError {
    CollectorError {
        domain: "pipelines".to_owned(),
        message,
        is_timeout: false,
    }
}

fn build_snapshot(
    services: &[(RegisteredService, String)],
    history: &HashMap<String, PipelineHistoryEntry>,
) -> PipelineSnapshot {
    let mut pipelines = Vec::with_capacity(services.len());
    let mut running_builds = 0usize;
    let mut successful_last_day = 0usize;
    let mut failed_terminal = 0usize;
    let mut total_terminal = 0usize;
    let day_ago = Utc::now() - chrono::Duration::hours(24);

    for (service, endpoint) in services {
        let provider = PipelineProvider::from_hint(endpoint);
        let entry = history.get(&service.id);
        let runs = entry.map(|item| item.runs.as_slice()).unwrap_or(&[]);
        let latest = runs.first();
        let duration_trend_secs = runs
            .iter()
            .filter_map(|run| run.duration_secs)
            .take(10)
            .collect::<Vec<_>>();
        let pipeline_running = runs.iter().filter(|run| run.is_running).count() as u32;
        let recent_terminal = runs
            .iter()
            .filter(|run| !run.is_running)
            .take(20)
            .collect::<Vec<_>>();
        let pipeline_failed = recent_terminal
            .iter()
            .filter(|run| is_failure_status(&run.status))
            .count();
        let pipeline_failure_rate = if recent_terminal.is_empty() {
            0.0
        } else {
            (pipeline_failed as f64 / recent_terminal.len() as f64) * 100.0
        };

        running_builds += pipeline_running as usize;

        for run in runs {
            if !run.is_running {
                total_terminal += 1;
                if is_failure_status(&run.status) {
                    failed_terminal += 1;
                }
                if is_success_status(&run.status)
                    && run.updated_at.is_some_and(|timestamp| timestamp >= day_ago)
                {
                    successful_last_day += 1;
                }
            }
        }

        let webhook_path = format!(
            "/api/pipelines/webhook/{}/{}",
            provider.as_str(),
            service.id
        );
        pipelines.push(PipelineStatus {
            service_id: service.id.clone(),
            name: service.name.clone(),
            environment: service.environment.clone(),
            team: service.team.clone(),
            provider,
            endpoint: endpoint.clone(),
            webhook_path,
            last_status: latest
                .map(|run| run.status.clone())
                .unwrap_or_else(|| "unknown".to_owned()),
            last_duration_secs: latest.and_then(|run| run.duration_secs),
            duration_trend_secs,
            failure_rate: pipeline_failure_rate,
            running_builds: pipeline_running,
            last_stage: latest.and_then(|run| run.stage.clone()),
            log_reference: latest.and_then(|run| run.log_reference.clone()),
            last_updated_at: latest.and_then(|run| run.updated_at),
        });
    }

    pipelines.sort_by(|left, right| {
        right
            .last_updated_at
            .cmp(&left.last_updated_at)
            .then(left.name.cmp(&right.name))
    });

    PipelineSnapshot {
        pipelines,
        deployment_frequency_per_day: successful_last_day as f64,
        change_failure_rate: if total_terminal == 0 {
            0.0
        } else {
            (failed_terminal as f64 / total_terminal as f64) * 100.0
        },
        running_builds,
    }
}

fn merge_probe(
    history: &mut HashMap<String, PipelineHistoryEntry>,
    service: &RegisteredService,
    probe: PipelineProbe,
) {
    let entry = history
        .entry(service.id.clone())
        .or_insert_with(|| PipelineHistoryEntry {
            service_id: service.id.clone(),
            name: service.name.clone(),
            endpoint: match &service.service_type {
                ServiceType::Pipeline { endpoint } => endpoint.clone(),
                _ => String::new(),
            },
            provider: probe.provider.clone(),
            runs: Vec::new(),
        });
    entry.provider = probe.provider;
    entry.name = service.name.clone();
    if let ServiceType::Pipeline { endpoint } = &service.service_type {
        entry.endpoint = endpoint.clone();
    }

    for run in probe.runs {
        if let Some(existing) = entry.runs.iter_mut().find(|item| item.run_id == run.run_id) {
            *existing = merge_runs(existing.clone(), run);
        } else {
            entry.runs.push(run);
        }
    }

    entry.runs.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then(right.started_at.cmp(&left.started_at))
    });
    entry.runs.truncate(MAX_RUNS_PER_PIPELINE);
}

fn merge_runs(existing: PipelineRun, fresh: PipelineRun) -> PipelineRun {
    PipelineRun {
        run_id: existing.run_id,
        status: if fresh.status.is_empty() {
            existing.status
        } else {
            fresh.status
        },
        stage: fresh.stage.or(existing.stage),
        duration_secs: fresh.duration_secs.or(existing.duration_secs),
        log_reference: fresh.log_reference.or(existing.log_reference),
        started_at: fresh.started_at.or(existing.started_at),
        updated_at: fresh.updated_at.or(existing.updated_at),
        source: if fresh.source.is_empty() {
            existing.source
        } else {
            fresh.source
        },
        is_running: fresh.is_running,
    }
}

async fn fetch_pipeline_probe(
    client: &Client,
    service: &RegisteredService,
    endpoint: &str,
) -> Result<PipelineProbe, String> {
    let provider = PipelineProvider::from_hint(endpoint);
    let response = client
        .get(endpoint)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{} returned HTTP {}", endpoint, response.status()));
    }

    let body = response.text().await.map_err(|error| error.to_string())?;
    let runs = serde_json::from_str::<Value>(&body)
        .ok()
        .map(|value| parse_pull_runs(&value, &provider, endpoint))
        .unwrap_or_default();

    if runs.is_empty() {
        let fallback = PipelineRun {
            run_id: stable_run_id(service, endpoint, "reachable", None),
            status: "reachable".to_owned(),
            stage: Some("api".to_owned()),
            duration_secs: None,
            log_reference: Some(endpoint.to_owned()),
            started_at: None,
            updated_at: Some(Utc::now()),
            source: "pull".to_owned(),
            is_running: false,
        };
        Ok(PipelineProbe {
            provider,
            runs: vec![fallback],
        })
    } else {
        Ok(PipelineProbe { provider, runs })
    }
}

fn parse_pull_runs(value: &Value, provider: &PipelineProvider, endpoint: &str) -> Vec<PipelineRun> {
    match provider {
        PipelineProvider::GitHubActions => parse_github_runs(value),
        PipelineProvider::GitLabCi => parse_gitlab_runs(value),
        PipelineProvider::Jenkins => parse_jenkins_runs(value),
        PipelineProvider::Generic => parse_generic_runs(value, endpoint),
    }
}

fn parse_webhook_runs(
    value: &Value,
    provider: &PipelineProvider,
    endpoint: &str,
) -> Vec<PipelineRun> {
    match provider {
        PipelineProvider::GitHubActions => value
            .get("workflow_run")
            .map(parse_github_run)
            .into_iter()
            .flatten()
            .collect(),
        PipelineProvider::GitLabCi => value
            .get("object_attributes")
            .map(parse_gitlab_run)
            .into_iter()
            .flatten()
            .collect(),
        PipelineProvider::Jenkins => parse_jenkins_runs(value),
        PipelineProvider::Generic => parse_generic_runs(value, endpoint),
    }
}

fn parse_github_runs(value: &Value) -> Vec<PipelineRun> {
    if let Some(runs) = value.get("workflow_runs").and_then(Value::as_array) {
        runs.iter().filter_map(parse_github_run).collect()
    } else {
        parse_github_run(value).into_iter().collect()
    }
}

fn parse_github_run(value: &Value) -> Option<PipelineRun> {
    let status = normalize_status(
        value
            .get("conclusion")
            .and_then(Value::as_str)
            .or_else(|| value.get("status").and_then(Value::as_str))
            .unwrap_or("unknown"),
    );
    let updated_at = parse_ts(
        value
            .get("updated_at")
            .and_then(Value::as_str)
            .or_else(|| value.get("created_at").and_then(Value::as_str)),
    );
    let started_at = parse_ts(value.get("run_started_at").and_then(Value::as_str));
    let duration_secs = started_at
        .zip(updated_at)
        .map(|(start, end)| (end - start).num_seconds().max(0) as u64);
    let run_id = value
        .get("id")
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
        .unwrap_or_else(|| stable_hash_hex(value.to_string().as_bytes()));
    Some(PipelineRun {
        run_id,
        status: status.clone(),
        stage: value
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                value
                    .get("display_title")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }),
        duration_secs,
        log_reference: value
            .get("html_url")
            .and_then(Value::as_str)
            .map(str::to_owned),
        started_at,
        updated_at,
        source: "github".to_owned(),
        is_running: is_running_status(&status),
    })
}

fn parse_gitlab_runs(value: &Value) -> Vec<PipelineRun> {
    if let Some(runs) = value.as_array() {
        runs.iter().filter_map(parse_gitlab_run).collect()
    } else {
        parse_gitlab_run(value).into_iter().collect()
    }
}

fn parse_gitlab_run(value: &Value) -> Option<PipelineRun> {
    let status = normalize_status(
        value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
    );
    Some(PipelineRun {
        run_id: value
            .get("id")
            .and_then(Value::as_i64)
            .map(|id| id.to_string())
            .unwrap_or_else(|| stable_hash_hex(value.to_string().as_bytes())),
        status: status.clone(),
        stage: value
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| value.get("ref").and_then(Value::as_str).map(str::to_owned)),
        duration_secs: value
            .get("duration")
            .and_then(Value::as_f64)
            .map(|secs| secs.max(0.0) as u64),
        log_reference: value
            .get("web_url")
            .and_then(Value::as_str)
            .map(str::to_owned),
        started_at: parse_ts(
            value
                .get("started_at")
                .and_then(Value::as_str)
                .or_else(|| value.get("created_at").and_then(Value::as_str)),
        ),
        updated_at: parse_ts(
            value
                .get("updated_at")
                .and_then(Value::as_str)
                .or_else(|| value.get("finished_at").and_then(Value::as_str))
                .or_else(|| value.get("created_at").and_then(Value::as_str)),
        ),
        source: "gitlab".to_owned(),
        is_running: is_running_status(&status),
    })
}

fn parse_jenkins_runs(value: &Value) -> Vec<PipelineRun> {
    if let Some(builds) = value.get("builds").and_then(Value::as_array) {
        builds.iter().filter_map(parse_jenkins_run).collect()
    } else if let Some(last_build) = value.get("lastBuild") {
        parse_jenkins_run(last_build).into_iter().collect()
    } else {
        parse_jenkins_run(value).into_iter().collect()
    }
}

fn parse_jenkins_run(value: &Value) -> Option<PipelineRun> {
    let raw_status = value
        .get("result")
        .and_then(Value::as_str)
        .or_else(|| value.get("status").and_then(Value::as_str))
        .unwrap_or_else(|| {
            if value
                .get("building")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "running"
            } else {
                "unknown"
            }
        });
    let status = normalize_status(raw_status);
    Some(PipelineRun {
        run_id: value
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                value
                    .get("number")
                    .and_then(Value::as_i64)
                    .map(|number| number.to_string())
            })
            .unwrap_or_else(|| stable_hash_hex(value.to_string().as_bytes())),
        status: status.clone(),
        stage: value
            .get("fullDisplayName")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                value
                    .get("displayName")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .or_else(|| value.get("name").and_then(Value::as_str).map(str::to_owned)),
        duration_secs: value
            .get("duration")
            .and_then(Value::as_u64)
            .map(|millis| millis / 1000),
        log_reference: value.get("url").and_then(Value::as_str).map(str::to_owned),
        started_at: None,
        updated_at: Some(Utc::now()),
        source: "jenkins".to_owned(),
        is_running: value
            .get("building")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || is_running_status(&status),
    })
}

fn parse_generic_runs(value: &Value, endpoint: &str) -> Vec<PipelineRun> {
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .filter_map(|item| parse_generic_run(item, endpoint))
            .collect();
    }
    parse_generic_run(value, endpoint).into_iter().collect()
}

fn parse_generic_run(value: &Value, endpoint: &str) -> Option<PipelineRun> {
    let raw_status = value
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| value.get("state").and_then(Value::as_str))
        .or_else(|| value.get("result").and_then(Value::as_str))
        .unwrap_or("unknown");
    let status = normalize_status(raw_status);
    Some(PipelineRun {
        run_id: value
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                value
                    .get("id")
                    .and_then(Value::as_i64)
                    .map(|id| id.to_string())
            })
            .unwrap_or_else(|| stable_hash_hex(format!("{endpoint}:{value}").as_bytes())),
        status: status.clone(),
        stage: value
            .get("stage")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| value.get("name").and_then(Value::as_str).map(str::to_owned)),
        duration_secs: value
            .get("duration_secs")
            .and_then(Value::as_u64)
            .or_else(|| value.get("duration").and_then(Value::as_u64)),
        log_reference: value
            .get("logs_url")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| value.get("url").and_then(Value::as_str).map(str::to_owned))
            .or_else(|| Some(endpoint.to_owned())),
        started_at: parse_ts(value.get("started_at").and_then(Value::as_str)),
        updated_at: parse_ts(
            value
                .get("updated_at")
                .and_then(Value::as_str)
                .or_else(|| value.get("finished_at").and_then(Value::as_str))
                .or_else(|| value.get("created_at").and_then(Value::as_str)),
        )
        .or_else(|| Some(Utc::now())),
        source: "generic".to_owned(),
        is_running: is_running_status(&status),
    })
}

fn load_history(path: &Path) -> HashMap<String, PipelineHistoryEntry> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    let parsed = toml_or_json_pipeline_history(&text);
    parsed
        .pipeline
        .into_iter()
        .map(|entry| (entry.service_id.clone(), entry))
        .collect()
}

fn toml_or_json_pipeline_history(text: &str) -> PipelineHistoryFile {
    serde_json::from_str(text)
        .or_else(|_| toml::from_str(text))
        .unwrap_or_default()
}

fn persist_history(
    path: &Path,
    history: &HashMap<String, PipelineHistoryEntry>,
) -> Result<(), std::io::Error> {
    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&parent, fs::Permissions::from_mode(0o700));
    }

    let mut entries = history.values().cloned().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.service_id.cmp(&right.service_id));
    let payload = serde_json::to_string_pretty(&PipelineHistoryFile { pipeline: entries })?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, payload)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn normalize_status(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "success" | "passed" | "completed" | "ok" | "succeeded" => "success".to_owned(),
        "failed" | "failure" | "error" | "errored" | "broken" => "failed".to_owned(),
        "cancelled" | "canceled" | "aborted" => "cancelled".to_owned(),
        "queued" | "pending" | "running" | "in_progress" | "building" => "running".to_owned(),
        other => other.to_owned(),
    }
}

fn is_failure_status(status: &str) -> bool {
    matches!(status, "failed" | "error" | "errored")
}

fn is_success_status(status: &str) -> bool {
    status == "success"
}

fn is_running_status(status: &str) -> bool {
    matches!(status, "running" | "queued" | "pending" | "building")
}

fn parse_ts(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn stable_run_id(
    service: &RegisteredService,
    endpoint: &str,
    status: &str,
    log_reference: Option<&str>,
) -> String {
    stable_hash_hex(
        format!(
            "{}:{}:{}:{}",
            service.id,
            endpoint,
            status,
            log_reference.unwrap_or_default()
        )
        .as_bytes(),
    )
}

fn stable_hash_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_service::ServiceType;
    use proptest::prelude::*;

    fn pipeline_service(id: &str, endpoint: &str) -> RegisteredService {
        RegisteredService {
            id: id.to_owned(),
            name: "Deploy Pipeline".to_owned(),
            service_type: ServiceType::Pipeline {
                endpoint: endpoint.to_owned(),
            },
            environment: "production".to_owned(),
            team: "devops".to_owned(),
            consecutive_failures: 0,
            last_healthy: None,
        }
    }

    #[test]
    fn parses_github_workflow_runs() {
        let value = serde_json::json!({
            "workflow_runs": [{
                "id": 42,
                "name": "deploy",
                "status": "completed",
                "conclusion": "failure",
                "html_url": "https://github.test/runs/42",
                "run_started_at": "2026-05-24T09:00:00Z",
                "updated_at": "2026-05-24T09:02:30Z"
            }]
        });
        let runs = parse_github_runs(&value);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "failed");
        assert_eq!(runs[0].duration_secs, Some(150));
    }

    proptest! {
        #[test]
        fn property_33_pipeline_failure_findings_include_required_fields(
            name in "[A-Za-z0-9 _-]{1,32}",
            stage in "[A-Za-z0-9 _-]{1,24}",
            log_ref in "https://[A-Za-z0-9./_-]{5,48}"
        ) {
            let snapshot = PipelineSnapshot {
                pipelines: vec![PipelineStatus {
                    service_id: "deploy".to_owned(),
                    name: name.clone(),
                    environment: "production".to_owned(),
                    team: "platform".to_owned(),
                    provider: PipelineProvider::Generic,
                    endpoint: "https://ci.example.test/api".to_owned(),
                    webhook_path: "/api/pipelines/webhook/generic/deploy".to_owned(),
                    last_status: "failed".to_owned(),
                    last_duration_secs: Some(60),
                    duration_trend_secs: vec![60, 58, 61],
                    failure_rate: 50.0,
                    running_builds: 0,
                    last_stage: Some(stage.clone()),
                    log_reference: Some(log_ref.clone()),
                    last_updated_at: Some(Utc::now()),
                }],
                deployment_frequency_per_day: 4.0,
                change_failure_rate: 25.0,
                running_builds: 0,
            };

            let findings = pipeline_failure_findings(&snapshot, "snap-1", Utc::now());
            prop_assert_eq!(findings.len(), 1);
            let finding = &findings[0];
            let expected_name = pipeline_display_name(&name, "deploy");
            let expected_stage = if stage.trim().is_empty() {
                "unknown".to_owned()
            } else {
                stage
                    .replace('_', " ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let raw_stage = if stage.trim().is_empty() {
                "unknown".to_owned()
            } else {
                stage.split_whitespace().collect::<Vec<_>>().join(" ")
            };
            prop_assert_eq!(finding.severity, Severity::Critical);
            prop_assert!(finding.title.contains(&expected_name));
            prop_assert!(finding.title.contains(&expected_stage));
            let has_stage_ref = finding.evidence.iter().any(|item| {
                item.value.contains(&expected_stage) || item.value.contains(&raw_stage)
            });
            prop_assert!(has_stage_ref);
            prop_assert!(finding.evidence.iter().any(|item| item.value.contains(&log_ref)));
        }
    }

    #[tokio::test]
    async fn collector_builds_snapshot_from_history() {
        let service = pipeline_service("deploy", "https://ci.example.test/pipelines");
        let mut history = HashMap::new();
        merge_probe(
            &mut history,
            &service,
            PipelineProbe {
                provider: PipelineProvider::GitLabCi,
                runs: vec![PipelineRun {
                    run_id: "42".to_owned(),
                    status: "success".to_owned(),
                    stage: Some("deploy".to_owned()),
                    duration_secs: Some(90),
                    log_reference: Some("https://ci.example.test/pipelines/42".to_owned()),
                    started_at: None,
                    updated_at: Some(Utc::now()),
                    source: "test".to_owned(),
                    is_running: false,
                }],
            },
        );
        let snapshot = build_snapshot(
            &[(service, "https://ci.example.test/pipelines".to_owned())],
            &history,
        );
        assert_eq!(snapshot.pipelines.len(), 1);
        assert_eq!(snapshot.pipelines[0].last_status, "success");
    }
}
