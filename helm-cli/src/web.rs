//! HELMOPS web dashboard — production HTTP/WebSocket server.
//! Started via `helmops web [--port PORT]`.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::{get, post},
};
use helm_memory::{FindingStateStore, MemoryStore, SnapshotStore};
use helm_monitor::{
    Finding, HostIdentity, MetricHistory, MonitorProfile, PlanCache, Severity, SnapshotDomains,
    SystemSnapshot,
    collectors::{
        Collector, db_health::DbHealthCollector, db_health::DbInstance,
        web_servers::WebServerCollector, web_servers::WebSite,
    },
    log_pattern_ids_new_since_previous,
    pipeline::{PipelineSnapshot, record_pipeline_webhook_event},
};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::{ProviderSettings, llm_provider_usable, ops, secrets::SecretsStore};

const DASHBOARD_HTML: &str = include_str!("web/dashboard.html");

struct WebState {
    policy: helm_monitor::ExecutionPolicy,
    db_path: PathBuf,
    memory: Arc<MemoryStore>,
    snapshot_tx: broadcast::Sender<String>,
    plan_cache: PlanCache,
    provider_settings: ProviderSettings,
    secrets: SecretsStore,
    fleet_cache: Mutex<HashMap<uuid::Uuid, FleetHost>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LiveSnapshot {
    pub hostname: String,
    pub host: HostIdentity,
    pub collected_at: String,
    pub profile: String,
    pub snapshot_source: String,
    pub scope: String,
    pub provider: String,
    pub ai_state: String,
    pub cpu_percent: f64,
    pub load_1m: f64,
    pub load_5m: f64,
    pub load_15m: f64,
    pub memory_used_pct: f64,
    pub memory_total_gb: f64,
    pub swap_used_pct: f64,
    pub total_services: usize,
    pub failed_services: usize,
    pub total_containers: usize,
    pub running_containers: usize,
    pub listening_ports: usize,
    pub journal_errors: u64,
    pub backup_count: usize,
    pub stale_backups: usize,
    pub findings: Vec<FindingEntry>,
    pub collector_health: Vec<CollectorHealth>,
    pub fleet_hosts: Vec<FleetHost>,
    pub daemon: DaemonSummary,
    pub metric_history: MetricHistoryPayload,
    pub log_entries: Vec<String>,
    pub new_log_pattern_ids: Vec<String>,
    pub inventory: SnapshotDomains,
    pub databases: Vec<DbInstance>,
    pub websites: Vec<WebSite>,
    pub pipelines: PipelineSnapshot,
    pub change_sets: Vec<ops::ChangeSetView>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceActionRequest {
    unit: String,
    action: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeRollbackRequest {
    change_set_id: String,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetricHistoryPayload {
    pub cpu: Vec<f64>,
    pub memory: Vec<f64>,
    pub disk: Vec<f64>,
    pub network: Vec<f64>,
    pub load: Vec<f64>,
    pub swap: Vec<f64>,
    pub io_pressure: Vec<f64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FindingEntry {
    pub id: String,
    pub fingerprint: String,
    pub severity: String,
    pub confidence: String,
    pub category: String,
    pub title: String,
    pub affected_resource: String,
    pub evidence_summary: String,
    pub evidence: Vec<EvidenceEntry>,
    pub assumptions: Vec<String>,
    pub missing_data: Vec<String>,
    pub impact: String,
    pub read_only_checks: Vec<String>,
    pub fix_plan: Option<String>,
    pub risk: String,
    pub lifecycle_state: String,
    pub first_seen: String,
    pub last_seen: String,
    pub occurrence_count: u64,
    pub detector_id: String,
    pub correlation_id: Option<String>,
    pub root_cause_finding_id: Option<String>,
    pub correlated_ids: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceEntry {
    pub source: String,
    pub value: String,
    pub note: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CollectorHealth {
    pub domain: String,
    pub healthy: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FleetHost {
    pub hostname: String,
    pub transport: String,
    pub status: String,
    pub stale: bool,
    pub findings_count: usize,
    pub critical_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub last_seen: String,
    pub load_1m: Option<f64>,
    pub cpu_percent: Option<f64>,
    pub memory_used_pct: Option<f64>,
    pub failed_services: Option<usize>,
    pub running_containers: Option<usize>,
    pub total_containers: Option<usize>,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct DaemonSummary {
    pub running: bool,
    pub status: String,
    pub started_at: Option<String>,
    pub last_heartbeat: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub refresh_interval_secs: u64,
}

#[derive(Default)]
struct FindingHistory {
    first_seen_ts: i64,
    last_seen_ts: i64,
    count: u64,
}

#[derive(serde::Deserialize)]
struct PlanRequest {
    #[serde(rename = "findingId")]
    finding_id: String,
}

pub async fn serve(
    db_path: PathBuf,
    port: u16,
    provider_settings: ProviderSettings,
    secrets: SecretsStore,
    policy: helm_monitor::ExecutionPolicy,
) -> Result<()> {
    let (tx, _) = broadcast::channel::<String>(32);
    let memory = Arc::new(
        MemoryStore::open(&db_path)
            .await
            .with_context(|| format!("opening memory store at {}", db_path.display()))?,
    );
    let state = Arc::new(WebState {
        policy,
        db_path: db_path.clone(),
        memory,
        snapshot_tx: tx.clone(),
        plan_cache: PlanCache::default(),
        provider_settings,
        secrets,
        fleet_cache: Mutex::new(HashMap::new()),
    });
    let web_refresh_secs = helm_monitor::load_thresholds_config()
        .refresh_interval_secs
        .clamp(5, 300);

    let bg_state = Arc::clone(&state);
    let bg_tx = tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(web_refresh_secs));
        loop {
            interval.tick().await;
            let json = collect_live_snapshot(bg_state.as_ref()).await;
            let _ = bg_tx.send(json);
        }
    });

    let app = Router::new()
        .route("/", get(serve_dashboard))
        .route("/ws", get(ws_handler))
        .route("/api/snapshot", get(api_snapshot))
        .route("/api/refresh", post(api_refresh))
        .route("/api/plan", post(api_generate_plan))
        .route("/api/service-action", post(api_service_action))
        .route("/api/change-rollback", post(api_change_rollback))
        .route(
            "/api/pipelines/webhook/{provider}/{service_id}",
            post(api_pipeline_webhook),
        )
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("╔══════════════════════════════════════════════╗");
    println!("║   HELMOPS Web Console                       ║");
    println!("╠══════════════════════════════════════════════╣");
    println!("║   Dashboard  → http://{addr}        ║");
    println!("║   WebSocket  → ws://{addr}/ws          ║");
    println!("║   API        → http://{addr}/api/snapshot ║");
    println!("╚══════════════════════════════════════════════╝");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("binding web server address")?;
    axum::serve(listener, app).await.context("web server")?;
    Ok(())
}

async fn serve_dashboard() -> impl IntoResponse {
    axum::response::Html(DASHBOARD_HTML)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<WebState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<WebState>) {
    let mut rx = state.snapshot_tx.subscribe();
    let initial = collect_live_snapshot(state.as_ref()).await;
    if socket.send(Message::Text(initial.into())).await.is_err() {
        return;
    }
    loop {
        match rx.recv().await {
            Ok(json) => {
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

async fn api_snapshot(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let json = collect_live_snapshot(state.as_ref()).await;
    Json(serde_json::from_str::<serde_json::Value>(&json).unwrap_or_default())
}

async fn api_refresh(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let daemon_requested = crate::request_daemon_refresh().unwrap_or(false);
    let json = collect_live_snapshot(state.as_ref()).await;
    let _ = state.snapshot_tx.send(json.clone());
    let snapshot = serde_json::from_str::<serde_json::Value>(&json).unwrap_or_default();
    Json(serde_json::json!({
        "ok": true,
        "daemonRequested": daemon_requested,
        "snapshot": snapshot
    }))
}

async fn api_generate_plan(
    State(state): State<Arc<WebState>>,
    Json(req): Json<PlanRequest>,
) -> impl IntoResponse {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(e) => return Json(serde_json::json!({"error": format!("DB error: {e}")})),
    };

    match crate::find_finding_by_id(&conn, &req.finding_id) {
        Some(finding) => {
            if let Ok(Some(record)) = helm_memory::TroubleshootingPlanStore::latest_for_fingerprint(
                &conn,
                &finding.fingerprint,
            ) {
                return Json(serde_json::json!({
                    "finding_id": req.finding_id,
                    "title": finding.title,
                    "severity": finding.severity.as_str(),
                    "impact": finding.impact,
                    "evidence": finding.evidence.first().map(|e| &e.value),
                    "read_only_checks": finding.read_only_checks,
                    "fix_plan": web_plan_summary(&record),
                    "plan_status": record.dashboard_plan_status,
                }));
            }

            if llm_provider_usable(&state.provider_settings) {
                let runtime = crate::PlanGenerationRuntime {
                    provider_settings: state.provider_settings.clone(),
                    secrets: state.secrets.clone(),
                    cache: state.plan_cache.clone(),
                };
                if let Err(error) =
                    crate::queue_background_plan_generation(&runtime, &state.db_path, &finding)
                {
                    return Json(
                        serde_json::json!({"error": format!("Plan queue failed: {error}")}),
                    );
                }
                Json(serde_json::json!({
                    "finding_id": req.finding_id,
                    "title": finding.title,
                    "severity": finding.severity.as_str(),
                    "impact": finding.impact,
                    "evidence": finding.evidence.first().map(|e| &e.value),
                    "read_only_checks": finding.read_only_checks,
                    "fix_plan": "Generating plan... retry in a few seconds.",
                    "plan_status": "loading",
                }))
            } else {
                Json(serde_json::json!({
                    "finding_id": req.finding_id,
                    "title": finding.title,
                    "severity": finding.severity.as_str(),
                    "impact": finding.impact,
                    "evidence": finding.evidence.first().map(|e| &e.value),
                    "read_only_checks": finding.read_only_checks,
                    "fix_plan": finding.fix_plan,
                    "plan_status": "disabled",
                    "plan_note": "No LLM provider is configured for cached remediation plans yet."
                }))
            }
        }
        None => Json(serde_json::json!({
            "error": "Finding not found. Run a collection first.",
            "plan": null
        })),
    }
}

async fn api_service_action(
    State(state): State<Arc<WebState>>,
    Json(req): Json<ServiceActionRequest>,
) -> impl IntoResponse {
    match req.action.as_str() {
        "journal" => {
            if state
                .memory
                .active_capability_grant(helm_core::Capability::FsRead, true)
                .await
                .ok()
                .flatten()
                .is_none()
            {
                return Json(serde_json::json!({
                    "ok": false,
                    "error": "Missing fs.read grant. Run `helmops permissions grant fs.read --scope once` and retry.",
                }));
            }
            Json(serde_json::json!({
                "ok": true,
                "unit": req.unit,
                "lines": ops::read_service_journal_excerpt(&req.unit, 120).await,
            }))
        }
        "restart" | "stop" => {
            match state.policy.check_mutation() {
                Err(error) => return Json(serde_json::json!({"ok": false, "error": error})),
                Ok(false) => {
                    return Json(
                        serde_json::json!({"ok": true, "dryRun": true, "action": req.action, "unit": req.unit}),
                    );
                }
                Ok(true) => {}
            }
            if state
                .memory
                .active_capability_grant(helm_core::Capability::SystemService, true)
                .await
                .ok()
                .flatten()
                .is_none()
            {
                return Json(serde_json::json!({
                    "ok": false,
                    "error": "Missing system.service grant. Run `helmops permissions grant system.service --scope once` and retry.",
                }));
            }
            let mut command = tokio::process::Command::new("systemctl");
            command.args([&req.action, "--", &req.unit]);
            let output = helm_monitor::process_runner::run_command(
                command,
                Default::default(),
                &Default::default(),
            )
            .await;
            match output {
                Ok(out) => {
                    let ok = out.status.success();
                    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
                    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                    let json = collect_live_snapshot(state.as_ref()).await;
                    let _ = state.snapshot_tx.send(json);
                    Json(serde_json::json!({
                        "ok": ok,
                        "unit": req.unit,
                        "action": req.action,
                        "stdout": helm_core::redact_secrets(&stdout),
                        "stderr": helm_core::redact_secrets(&stderr),
                    }))
                }
                Err(error) => Json(serde_json::json!({
                    "ok": false,
                    "error": error.to_string(),
                })),
            }
        }
        _ => Json(serde_json::json!({
            "ok": false,
            "error": format!("unsupported service action: {}", req.action),
        })),
    }
}

async fn api_change_rollback(
    State(state): State<Arc<WebState>>,
    Json(req): Json<ChangeRollbackRequest>,
) -> impl IntoResponse {
    match state.policy.check_mutation() {
        Err(error) => return Json(serde_json::json!({"ok": false, "error": error})),
        Ok(false) => {
            return Json(
                serde_json::json!({"ok": true, "dryRun": true, "changeSetId": req.change_set_id}),
            );
        }
        Ok(true) => {}
    }
    if state
        .memory
        .active_capability_grant(helm_core::Capability::ShellShell, true)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return Json(serde_json::json!({
            "ok": false,
            "error": "Missing shell.shell grant. Run `helmops permissions grant shell.shell --scope once` and retry rollback.",
        }));
    }
    match ops::rollback_change_set(
        &state.db_path,
        &req.change_set_id,
        state.policy,
        &state.secrets,
    )
    .await
    {
        Ok(results) => {
            let json = collect_live_snapshot(state.as_ref()).await;
            let _ = state.snapshot_tx.send(json);
            Json(serde_json::json!({
                "ok": true,
                "changeSetId": req.change_set_id,
                "results": results,
            }))
        }
        Err(error) => Json(serde_json::json!({
            "ok": false,
            "error": error.to_string(),
        })),
    }
}

async fn api_pipeline_webhook(
    State(state): State<Arc<WebState>>,
    Path((provider, service_id)): Path<(String, String)>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    match record_pipeline_webhook_event(&provider, &service_id, &payload) {
        Ok(()) => {
            let json = collect_live_snapshot(state.as_ref()).await;
            let _ = state.snapshot_tx.send(json);
            Json(serde_json::json!({
                "ok": true,
                "provider": provider,
                "serviceId": service_id,
            }))
        }
        Err(error) => Json(serde_json::json!({
            "ok": false,
            "error": error.to_string(),
        })),
    }
}

fn web_plan_summary(record: &helm_memory::TroubleshootingPlanRecord) -> String {
    let mut lines = Vec::new();
    if !record.narrative_summary.trim().is_empty() {
        lines.push(record.narrative_summary.clone());
    }
    let steps = match helm_monitor::decode_plan_steps(&record.proposed_fix_steps_json) {
        Ok(steps) => steps
            .into_iter()
            .map(|step| serde_json::to_value(step).expect("typed plan serializes"))
            .collect::<Vec<_>>(),
        Err(error) => {
            lines.push(format!("Cannot load executable plan: {error}"));
            return lines.join("\n");
        }
    };
    for (index, step) in steps.iter().enumerate() {
        let command = step
            .get("command")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                step.get("command")
                    .and_then(|command| command.get("command_text"))
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or_default();
        let effect = step
            .get("expected_effect")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                step.get("command")
                    .and_then(|command| command.get("expected_effect"))
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or_default();
        if !command.is_empty() {
            lines.push(format!("{}. {} — {}", index + 1, command, effect));
        }
    }
    lines.join("\n")
}

async fn collect_live_snapshot(state: &WebState) -> String {
    let db_path = state.db_path.as_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return r#"{"error":"cannot open database"}"#.to_string(),
    };

    let daemon_running = crate::daemon_is_running();
    let (snapshot, findings, snapshot_source) =
        if daemon_running && let Some((snapshot, findings)) = latest_snapshot_and_findings(&conn) {
            (snapshot, findings, "daemon".to_owned())
        } else {
            let previous = crate::load_previous_snapshot(&conn);
            let report = crate::run_monitor_cycle(MonitorProfile::Quick, None, previous).await;
            (report.snapshot, report.findings, "live".to_owned())
        };

    let history = load_finding_history(&conn);
    let state_by_fingerprint = load_finding_states(&conn);
    let daemon = daemon_summary();
    let metric_history = load_metric_history(&conn);
    let (provider, ai_state) = provider_context();
    let change_sets = ops::load_change_set_views(&conn, 16);

    let db_future = DbHealthCollector.collect(snapshot.profile);
    let web_future = WebServerCollector.collect(snapshot.profile);
    let fleet_future = collect_fleet_hosts(state);
    let (db_result, web_result, fleet_hosts) = tokio::join!(db_future, web_future, fleet_future);

    let databases = db_result.map(|s| s.instances).unwrap_or_default();
    let websites = web_result.map(|s| s.sites).unwrap_or_default();

    let load = &snapshot.domains.load;
    let mem_total = load.memory.total;
    let mem_used = load.memory.used;
    let mem_pct = if mem_total > 0 {
        (mem_used as f64 / mem_total as f64) * 100.0
    } else {
        0.0
    };
    let swap_pct = if load.swap_total > 0 {
        (load.swap_used as f64 / load.swap_total as f64) * 100.0
    } else {
        0.0
    };
    let cpu_percent = if load.cpu_logical_count > 0 {
        ((load.load_average.one / load.cpu_logical_count as f64) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let services = &snapshot.domains.services.units;
    let total_services = services.len();
    let failed_services = services
        .iter()
        .filter(|unit| unit.active == "failed")
        .count();

    let containers = &snapshot.domains.containers.containers;
    let total_containers = containers.len();
    let running_containers = containers
        .iter()
        .filter(|container| container.status == "running")
        .count();

    let log_entries = log_entries(&snapshot);
    let new_log_pattern_ids = load_previous_log_patterns(&conn, &snapshot.id)
        .map(|previous| {
            let mut ids =
                log_pattern_ids_new_since_previous(&snapshot.domains.logs.patterns, &previous)
                    .into_iter()
                    .collect::<Vec<_>>();
            ids.sort();
            ids
        })
        .unwrap_or_else(|| {
            snapshot
                .domains
                .logs
                .patterns
                .iter()
                .map(|pattern| pattern.pattern_id.clone())
                .collect()
        });
    let collector_health = collector_health(&snapshot);
    let findings = findings
        .iter()
        .map(|finding| finding_entry(finding, &history, &state_by_fingerprint))
        .collect::<Vec<_>>();

    let live = LiveSnapshot {
        hostname: snapshot.host.hostname.clone(),
        host: snapshot.host.clone(),
        collected_at: snapshot.collected_at.to_rfc3339(),
        profile: monitor_profile_label(snapshot.profile),
        snapshot_source,
        scope: if fleet_hosts.is_empty() {
            "local".to_owned()
        } else {
            "fleet".to_owned()
        },
        provider,
        ai_state,
        cpu_percent,
        load_1m: load.load_average.one,
        load_5m: load.load_average.five,
        load_15m: load.load_average.fifteen,
        memory_used_pct: mem_pct,
        memory_total_gb: mem_total as f64 / 1_073_741_824.0,
        swap_used_pct: swap_pct,
        total_services,
        failed_services,
        total_containers,
        running_containers,
        listening_ports: snapshot.domains.ports.listeners.len(),
        journal_errors: snapshot.domains.logs.journal_errors_last_hour,
        backup_count: snapshot.domains.backups.tools_detected.len(),
        stale_backups: 0,
        findings,
        collector_health,
        fleet_hosts,
        daemon,
        metric_history,
        log_entries,
        new_log_pattern_ids,
        inventory: snapshot.domains.clone(),
        databases,
        websites,
        pipelines: snapshot.domains.pipelines.clone(),
        change_sets,
    };

    serde_json::to_string(&live).unwrap_or_default()
}

fn latest_snapshot_and_findings(
    conn: &rusqlite::Connection,
) -> Option<(SystemSnapshot, Vec<Finding>)> {
    let record = SnapshotStore::latest(conn).ok().flatten()?;
    let domains: SnapshotDomains = serde_json::from_str(&record.domains_json).ok()?;
    let findings = serde_json::from_str::<Vec<Finding>>(&record.findings_json).unwrap_or_default();
    Some((crate::reconstruct_snapshot(&record, domains), findings))
}

fn load_finding_history(conn: &rusqlite::Connection) -> HashMap<String, FindingHistory> {
    let mut history = HashMap::new();
    let Ok(records) = SnapshotStore::list(conn, 200) else {
        return history;
    };

    for record in records {
        let findings =
            serde_json::from_str::<Vec<Finding>>(&record.findings_json).unwrap_or_default();
        for finding in findings {
            let key = finding_key(&finding);
            let entry = history.entry(key).or_insert_with(|| FindingHistory {
                first_seen_ts: record.collected_at,
                last_seen_ts: record.collected_at,
                count: 0,
            });
            entry.first_seen_ts = entry.first_seen_ts.min(record.collected_at);
            entry.last_seen_ts = entry.last_seen_ts.max(record.collected_at);
            entry.count += 1;
        }
    }

    history
}

fn load_metric_history(conn: &rusqlite::Connection) -> MetricHistoryPayload {
    let depth = crate::read_daemon_state()
        .map(|state| state.metric_history.cpu.len().max(10))
        .unwrap_or_else(|| helm_monitor::load_thresholds_config().sparkline_history_depth);
    let mut history = MetricHistory::new(depth);
    let records = SnapshotStore::list(conn, depth as u32).unwrap_or_default();
    for record in records.into_iter().rev() {
        if let Ok(domains) = serde_json::from_str::<SnapshotDomains>(&record.domains_json) {
            let snapshot = crate::reconstruct_snapshot(&record, domains);
            history.push_snapshot(&snapshot);
        }
    }

    MetricHistoryPayload {
        cpu: history.cpu_values(),
        memory: history.memory_values(),
        disk: history.disk_values(),
        network: history.network_values(),
        load: history.load_values(),
        swap: history.swap_values(),
        io_pressure: history.io_pressure_values(),
    }
}

fn load_finding_states(
    conn: &rusqlite::Connection,
) -> HashMap<String, helm_memory::FindingStateRecord> {
    FindingStateStore::list(conn)
        .unwrap_or_default()
        .into_iter()
        .map(|record| (record.fingerprint.clone(), record))
        .collect()
}

fn finding_entry(
    finding: &Finding,
    history: &HashMap<String, FindingHistory>,
    states: &HashMap<String, helm_memory::FindingStateRecord>,
) -> FindingEntry {
    let key = finding_key(finding);
    let lifecycle_state = states
        .get(&finding.fingerprint)
        .map(|record| record.status.as_str().to_owned())
        .unwrap_or_else(|| format!("{:?}", finding.lifecycle).to_lowercase());
    let history = history.get(&key);

    FindingEntry {
        id: finding.id.clone(),
        fingerprint: finding.fingerprint.clone(),
        severity: finding.severity.as_str().to_owned(),
        confidence: finding.confidence.as_str().to_owned(),
        category: finding.category.as_str().to_owned(),
        title: finding.title.clone(),
        affected_resource: finding.affected_resource.clone(),
        evidence_summary: finding
            .evidence
            .first()
            .map(|evidence| evidence.value.clone())
            .unwrap_or_default(),
        evidence: finding
            .evidence
            .iter()
            .map(|evidence| EvidenceEntry {
                source: evidence.source.clone(),
                value: evidence.value.clone(),
                note: evidence.note.clone(),
            })
            .collect(),
        assumptions: finding.assumptions.clone(),
        missing_data: finding.missing_data.clone(),
        impact: finding.impact.clone(),
        read_only_checks: finding.read_only_checks.clone(),
        fix_plan: finding.fix_plan.clone(),
        risk: risk_label(finding.severity).to_owned(),
        lifecycle_state,
        first_seen: history
            .map(|h| timestamp_to_rfc3339(h.first_seen_ts))
            .unwrap_or_default(),
        last_seen: history
            .map(|h| timestamp_to_rfc3339(h.last_seen_ts))
            .unwrap_or_default(),
        occurrence_count: history.map(|h| h.count).unwrap_or(1),
        detector_id: finding.detector_id.clone(),
        correlation_id: finding.correlation_id.clone(),
        root_cause_finding_id: finding.root_cause_finding_id.clone(),
        correlated_ids: finding.correlated_finding_ids.clone(),
    }
}

fn finding_key(finding: &Finding) -> String {
    if finding.fingerprint.is_empty() {
        finding.id.clone()
    } else {
        finding.fingerprint.clone()
    }
}

fn risk_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "high",
        Severity::Warning => "medium",
        Severity::Info => "low",
    }
}

fn log_entries(snapshot: &SystemSnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    lines.extend(snapshot.domains.logs.kernel_errors.iter().cloned());
    lines.extend(snapshot.domains.logs.auth_failures.iter().cloned());
    if lines.is_empty() {
        lines.push("No recent kernel or auth errors detected.".to_owned());
    }
    lines
}

fn load_previous_log_patterns(
    conn: &rusqlite::Connection,
    current_snapshot_id: &str,
) -> Option<Vec<helm_monitor::JournalPattern>> {
    let records = SnapshotStore::list(conn, 2).ok()?;
    let record = records
        .into_iter()
        .find(|record| record.id != current_snapshot_id)?;
    let domains = serde_json::from_str::<SnapshotDomains>(&record.domains_json).ok()?;
    Some(domains.logs.patterns)
}

fn collector_health(snapshot: &SystemSnapshot) -> Vec<CollectorHealth> {
    let unhealthy: HashSet<String> = snapshot
        .collector_errors
        .iter()
        .map(|error| error.domain.to_lowercase())
        .collect();

    SnapshotDomains::domain_names()
        .iter()
        .map(|domain| {
            let name = (*domain).to_owned();
            let error = snapshot
                .collector_errors
                .iter()
                .find(|collector_error| collector_error.domain.eq_ignore_ascii_case(domain))
                .map(|collector_error| collector_error.message.clone());
            CollectorHealth {
                domain: name.clone(),
                healthy: !unhealthy.contains(&name),
                error,
            }
        })
        .collect()
}

fn fleet_host_from_report(transport: String, report: crate::remote::FleetHostReport) -> FleetHost {
    let critical_count = report
        .findings
        .iter()
        .filter(|finding| matches!(finding.severity, Severity::Critical))
        .count();
    let warning_count = report
        .findings
        .iter()
        .filter(|finding| matches!(finding.severity, Severity::Warning))
        .count();
    let info_count = report
        .findings
        .iter()
        .filter(|finding| matches!(finding.severity, Severity::Info))
        .count();
    let (
        last_seen,
        load_1m,
        cpu_percent,
        memory_used_pct,
        failed_services,
        running_containers,
        total_containers,
    ) = match report.snapshot.as_ref() {
        Some(snapshot) => {
            let load = &snapshot.domains.load;
            let cpu = if load.cpu_logical_count > 0 {
                ((load.load_average.one / load.cpu_logical_count as f64) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let mem = if load.memory.total > 0 {
                (load.memory.used as f64 / load.memory.total as f64) * 100.0
            } else {
                0.0
            };
            (
                snapshot.collected_at.to_rfc3339(),
                Some(load.load_average.one),
                Some(cpu),
                Some(mem),
                Some(
                    snapshot
                        .domains
                        .services
                        .units
                        .iter()
                        .filter(|unit| unit.active == "failed")
                        .count(),
                ),
                Some(
                    snapshot
                        .domains
                        .containers
                        .containers
                        .iter()
                        .filter(|container| container.status == "running")
                        .count(),
                ),
                Some(snapshot.domains.containers.containers.len()),
            )
        }
        None => (String::new(), None, None, None, None, None, None),
    };

    FleetHost {
        hostname: report.host_name,
        transport,
        status: if critical_count > 0 {
            "critical".to_owned()
        } else if warning_count > 0 {
            "warning".to_owned()
        } else {
            "online".to_owned()
        },
        stale: false,
        findings_count: report.findings.len(),
        critical_count,
        warning_count,
        info_count,
        last_seen,
        load_1m,
        cpu_percent,
        memory_used_pct,
        failed_services,
        running_containers,
        total_containers,
    }
}

fn stale_or_offline_fleet_host(
    cache: &Mutex<HashMap<uuid::Uuid, FleetHost>>,
    host_id: uuid::Uuid,
    entry: Option<&crate::remote::RemoteEntry>,
    default_status: &str,
) -> FleetHost {
    if let Ok(cache) = cache.lock()
        && let Some(previous) = cache.get(&host_id)
    {
        let mut stale = previous.clone();
        stale.hostname = entry
            .map(|remote| remote.name.clone())
            .unwrap_or_else(|| host_id.to_string());
        stale.transport = entry
            .map(|remote| remote.summary_label())
            .unwrap_or_else(|| "ssh".to_owned());
        stale.status = "stale".to_owned();
        stale.stale = true;
        return stale;
    }

    FleetHost {
        hostname: entry
            .map(|remote| remote.name.clone())
            .unwrap_or_else(|| host_id.to_string()),
        transport: entry
            .map(|remote| remote.summary_label())
            .unwrap_or_else(|| "ssh".to_owned()),
        status: default_status.to_owned(),
        stale: false,
        findings_count: 0,
        critical_count: 0,
        warning_count: 0,
        info_count: 0,
        last_seen: String::new(),
        load_1m: None,
        cpu_percent: None,
        memory_used_pct: None,
        failed_services: None,
        running_containers: None,
        total_containers: None,
    }
}

async fn collect_fleet_hosts(state: &WebState) -> Vec<FleetHost> {
    let Ok(registry) = crate::remote::RemoteRegistry::load() else {
        return Vec::new();
    };
    if registry.remotes.is_empty() {
        return Vec::new();
    }

    let entries = registry.remotes.clone();
    let by_id = entries
        .iter()
        .map(|entry| (entry.host_id, entry.clone()))
        .collect::<HashMap<_, _>>();

    let results =
        tokio::time::timeout(Duration::from_secs(12), registry.parallel_collect_reports()).await;

    match results {
        Ok(results) => results
            .into_iter()
            .map(|(host_id, result)| match result {
                Ok(report) => {
                    let transport = by_id
                        .get(&host_id)
                        .map(|entry| entry.summary_label())
                        .unwrap_or_else(|| "ssh".to_owned());
                    let host = fleet_host_from_report(transport, report);
                    if let Ok(mut cache) = state.fleet_cache.lock() {
                        cache.insert(host_id, host.clone());
                    }
                    host
                }
                Err(_) => stale_or_offline_fleet_host(
                    &state.fleet_cache,
                    host_id,
                    by_id.get(&host_id),
                    "offline",
                ),
            })
            .collect(),
        Err(_) => entries
            .into_iter()
            .map(|entry| {
                stale_or_offline_fleet_host(
                    &state.fleet_cache,
                    entry.host_id,
                    Some(&entry),
                    "configured",
                )
            })
            .collect(),
    }
}

fn daemon_summary() -> DaemonSummary {
    crate::read_daemon_state()
        .map(|state| DaemonSummary {
            running: crate::daemon_is_running(),
            status: state.status,
            started_at: state.started_at,
            last_heartbeat: state.last_heartbeat,
            last_success_at: state.last_success_at,
            last_error: state.last_error,
            refresh_interval_secs: state.refresh_interval_secs,
        })
        .unwrap_or_default()
}

fn provider_context() -> (String, String) {
    let config_path = match crate::default_config_path() {
        Ok(path) => path,
        Err(_) => return ("unconfigured".to_owned(), "off".to_owned()),
    };
    let config = match crate::load_config(&config_path) {
        Ok(config) => config,
        Err(_) => return ("unconfigured".to_owned(), "off".to_owned()),
    };
    let settings = match crate::resolve_provider_settings(config.as_ref(), None, None, None, None) {
        Ok(settings) => settings,
        Err(_) => return ("unconfigured".to_owned(), "off".to_owned()),
    };

    let provider = crate::provider_choice_name(settings.choice).to_owned();
    let ai_state = if matches!(settings.choice, crate::ProviderChoice::Ollama) {
        "local".to_owned()
    } else {
        "api".to_owned()
    };
    (provider, ai_state)
}

fn monitor_profile_label(profile: MonitorProfile) -> String {
    match profile {
        MonitorProfile::Quick => "quick".to_owned(),
        MonitorProfile::Standard => "standard".to_owned(),
        MonitorProfile::Deep => "deep".to_owned(),
    }
}

fn timestamp_to_rfc3339(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn web_modes_block_service_and_rollback_before_permission_consumption() {
        for policy in [
            helm_monitor::ExecutionPolicy {
                read_only: true,
                ..Default::default()
            },
            helm_monitor::ExecutionPolicy {
                dry_run: true,
                ..Default::default()
            },
            helm_monitor::ExecutionPolicy {
                sandbox_requested: true,
                ..Default::default()
            },
        ] {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("helm.db");
            let state = Arc::new(WebState {
                policy,
                memory: Arc::new(MemoryStore::open(&db_path).await.unwrap()),
                db_path,
                snapshot_tx: broadcast::channel(1).0,
                plan_cache: PlanCache::default(),
                provider_settings: crate::resolve_provider_settings(
                    None,
                    Some(crate::ProviderChoice::Ollama),
                    None,
                    None,
                    None,
                )
                .unwrap(),
                secrets: SecretsStore::open_at(dir.path().join("secrets.toml")).unwrap(),
                fleet_cache: Mutex::new(HashMap::new()),
            });
            let service = api_service_action(
                State(state.clone()),
                Json(ServiceActionRequest {
                    unit: "helm-test-never-target.service".into(),
                    action: "stop".into(),
                }),
            )
            .await
            .into_response();
            let rollback = api_change_rollback(
                State(state),
                Json(ChangeRollbackRequest {
                    change_set_id: "never-existed".into(),
                }),
            )
            .await
            .into_response();
            for response in [service, rollback] {
                let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
                if policy.dry_run {
                    assert_eq!(json["dryRun"], true);
                } else {
                    assert_eq!(json["ok"], false);
                    assert!(!json["error"].as_str().unwrap().contains("grant"));
                }
            }
        }
    }

    #[test]
    fn dashboard_html_exposes_mobile_and_live_console_hooks() {
        assert!(DASHBOARD_HTML.contains("width=device-width, initial-scale=1.0"));
        assert!(DASHBOARD_HTML.contains("new WebSocket"));
        assert!(DASHBOARD_HTML.contains("F5"));
        assert!(DASHBOARD_HTML.contains("Alt+G"));
        assert!(DASHBOARD_HTML.contains("1-0"));
        assert!(DASHBOARD_HTML.contains("renderLogs"));
        assert!(DASHBOARD_HTML.contains("renderChanges"));
    }
}
