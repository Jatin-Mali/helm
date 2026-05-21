//! HELMOPS web dashboard — production HTTP/WebSocket server.
//! Started via `helmops web [--port PORT]`.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::{get, post},
};
use helm_memory::{FindingStateStore, SnapshotStore};
use helm_monitor::{
    Finding, HostIdentity, MonitorProfile, Severity, SnapshotDomains, SystemSnapshot,
    collectors::{
        Collector, db_health::DbHealthCollector, db_health::DbInstance,
        web_servers::WebServerCollector, web_servers::WebSite,
    },
};
use serde::Serialize;
use tokio::sync::broadcast;

struct WebState {
    db_path: PathBuf,
    snapshot_tx: broadcast::Sender<String>,
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
    pub log_entries: Vec<String>,
    pub inventory: SnapshotDomains,
    pub databases: Vec<DbInstance>,
    pub websites: Vec<WebSite>,
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

pub async fn serve(db_path: PathBuf, port: u16) -> Result<()> {
    let (tx, _) = broadcast::channel::<String>(32);
    let state = Arc::new(WebState {
        db_path: db_path.clone(),
        snapshot_tx: tx.clone(),
    });

    let bg_db = db_path.clone();
    let bg_tx = tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            let json = collect_live_snapshot(&bg_db).await;
            let _ = bg_tx.send(json);
        }
    });

    let app = Router::new()
        .route("/", get(serve_dashboard))
        .route("/ws", get(ws_handler))
        .route("/api/snapshot", get(api_snapshot))
        .route("/api/refresh", post(api_refresh))
        .route("/api/plan", post(api_generate_plan))
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
    axum::response::Html(include_str!("web/dashboard.html"))
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<WebState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<WebState>) {
    let mut rx = state.snapshot_tx.subscribe();
    let initial = collect_live_snapshot(&state.db_path).await;
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
    let json = collect_live_snapshot(&state.db_path).await;
    Json(serde_json::from_str::<serde_json::Value>(&json).unwrap_or_default())
}

async fn api_refresh(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let daemon_requested = crate::request_daemon_refresh().unwrap_or(false);
    let json = collect_live_snapshot(&state.db_path).await;
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
        Some(finding) => Json(serde_json::json!({
            "finding_id": req.finding_id,
            "title": finding.title,
            "severity": finding.severity.as_str(),
            "impact": finding.impact,
            "evidence": finding.evidence.first().map(|e| &e.value),
            "read_only_checks": finding.read_only_checks,
            "fix_plan": finding.fix_plan,
            "plan_note": "AI-backed plans remain available through `helmops troubleshoot <issue>` when you want a reviewed remediation path."
        })),
        None => Json(serde_json::json!({
            "error": "Finding not found. Run a collection first.",
            "plan": null
        })),
    }
}

async fn collect_live_snapshot(db_path: &std::path::Path) -> String {
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
    let (provider, ai_state) = provider_context();

    let db_future = DbHealthCollector.collect(snapshot.profile);
    let web_future = WebServerCollector.collect(snapshot.profile);
    let fleet_future = collect_fleet_hosts();
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
        log_entries,
        inventory: snapshot.domains.clone(),
        databases,
        websites,
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
        correlated_ids: Vec::new(),
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

async fn collect_fleet_hosts() -> Vec<FleetHost> {
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
                                ((load.load_average.one / load.cpu_logical_count as f64) * 100.0)
                                    .clamp(0.0, 100.0)
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
                Err(_) => {
                    let entry = by_id.get(&host_id);
                    FleetHost {
                        hostname: entry
                            .map(|remote| remote.name.clone())
                            .unwrap_or_else(|| host_id.to_string()),
                        transport: entry
                            .map(|remote| remote.summary_label())
                            .unwrap_or_else(|| "ssh".to_owned()),
                        status: "offline".to_owned(),
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
            })
            .collect(),
        Err(_) => entries
            .into_iter()
            .map(|entry| FleetHost {
                hostname: entry.name.clone(),
                transport: entry.summary_label(),
                status: "configured".to_owned(),
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
