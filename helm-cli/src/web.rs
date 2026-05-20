//! HELMOPS web dashboard — production HTTP/WebSocket server.
//! Started via `helm web [--port PORT]`.
//!
//! Architecture:
//!   Background collector (tokio::spawn) → broadcast::channel → WebSocket → browser
//!   The HTML dashboard connects via WebSocket and renders live data.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::{get, post},
};
use helm_monitor::{MonitorProfile, Severity};
use serde::Serialize;
use tokio::sync::broadcast;

// ── Application State ──────────────────────────────────────────────────────

struct WebState {
    db_path: PathBuf,
    snapshot_tx: broadcast::Sender<String>,
}

// ── Live Snapshot (pushed to browser every N seconds) ──────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LiveSnapshot {
    pub hostname: String,
    pub collected_at: String,
    pub profile: String,
    pub cpu_percent: f64,
    pub load_1m: f64,
    pub load_5m: f64,
    pub load_15m: f64,
    pub memory_used_pct: f64,
    pub memory_total_gb: f64,
    pub swap_used_pct: f64,
    pub disk_entries: Vec<DiskEntry>,
    pub total_services: usize,
    pub failed_services: usize,
    pub total_containers: usize,
    pub running_containers: usize,
    pub listening_ports: usize,
    pub port_listeners: Vec<PortListener>,
    pub journal_errors: u64,
    pub log_entries: Vec<String>,
    pub backup_count: usize,
    pub stale_backups: usize,
    pub findings: Vec<FindingEntry>,
    pub services: Vec<ServiceEntry>,
    pub containers: Vec<ContainerEntry>,
    pub processes: Vec<ProcessEntry>,
    pub collector_health: Vec<CollectorHealth>,
    pub fleet_hosts: Vec<FleetHost>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiskEntry {
    pub mount: String,
    pub used_pct: f64,
    pub total_gb: f64,
    pub available_gb: f64,
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
pub struct CollectorHealth {
    pub domain: String,
    pub healthy: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServiceEntry {
    pub name: String,
    pub state: String,
    pub description: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ContainerEntry {
    pub name: String,
    pub image: String,
    pub status: String,
    pub ports: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PortListener {
    pub port: u16,
    pub protocol: String,
    pub process: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProcessEntry {
    pub name: String,
    pub pid: Option<u32>,
    pub ports: Vec<u16>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FleetHost {
    pub hostname: String,
    pub status: String,
    pub findings_count: usize,
    pub last_seen: String,
}

// ── Server Entry Point ─────────────────────────────────────────────────────

/// Start the HELMOPS web dashboard server.
pub async fn serve(db_path: PathBuf, port: u16) -> Result<()> {
    let (tx, _) = broadcast::channel::<String>(32);
    let state = Arc::new(WebState {
        db_path: db_path.clone(),
        snapshot_tx: tx.clone(),
    });

    // Background collector: run Quick profile every 5 seconds, broadcast to clients
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
        .route("/api/plan", post(api_generate_plan))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("╔══════════════════════════════════════════════╗");
    println!("║   HELMOPS Web Dashboard                      ║");
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

// ── Route Handlers ─────────────────────────────────────────────────────────

/// Serve the dashboard HTML (embedded at compile time).
async fn serve_dashboard() -> impl IntoResponse {
    axum::response::Html(include_str!("web/dashboard.html"))
}

/// WebSocket upgrade — each connected browser receives live snapshot stream.
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<WebState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<WebState>) {
    let mut rx = state.snapshot_tx.subscribe();
    // Send initial snapshot immediately
    let initial = collect_live_snapshot(&state.db_path).await;
    if socket.send(Message::Text(initial.into())).await.is_err() {
        return;
    }
    // Stream subsequent updates
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

/// REST endpoint: latest snapshot as JSON.
async fn api_snapshot(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let json = collect_live_snapshot(&state.db_path).await;
    Json(serde_json::from_str::<serde_json::Value>(&json).unwrap_or_default())
}

/// POST /api/plan — generate a troubleshooting plan for a finding.
#[derive(serde::Deserialize)]
struct PlanRequest {
    #[serde(rename = "findingId")]
    finding_id: String,
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
            Json(serde_json::json!({
                "finding_id": req.finding_id,
                "title": finding.title,
                "severity": format!("{:?}", finding.severity),
                "impact": finding.impact,
                "evidence": finding.evidence.first().map(|e| &e.value),
                "read_only_checks": finding.read_only_checks,
                "fix_plan": finding.fix_plan,
                "plan_note": "AI-powered plan generation: use 'helm troubleshoot <issue>' from CLI, or configure an LLM provider with 'helm init --provider ollama'"
            }))
        }
        None => {
            Json(serde_json::json!({
                "error": "Finding not found. Run a collection first.",
                "plan": null
            }))
        }
    }
}

// ── Data Collection ────────────────────────────────────────────────────────

async fn collect_live_snapshot(db_path: &std::path::Path) -> String {
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return r#"{"error":"cannot open database"}"#.to_string(),
    };

    let prev = crate::load_previous_snapshot(&conn);
    let report = crate::run_monitor_cycle(MonitorProfile::Quick, None, prev).await;

    let snapshot = &report.snapshot;
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

    // Disk entries
    let disk_entries: Vec<DiskEntry> = snapshot
        .domains
        .disks
        .filesystems
        .iter()
        .map(|fs| DiskEntry {
            mount: fs.mount_point.clone(),
            used_pct: if fs.total_bytes > 0 {
                (fs.used_bytes as f64 / fs.total_bytes as f64) * 100.0
            } else {
                0.0
            },
            total_gb: fs.total_bytes as f64 / 1_073_741_824.0,
            available_gb: fs.available_bytes as f64 / 1_073_741_824.0,
        })
        .collect();

    // Findings
    let findings: Vec<FindingEntry> = report
        .findings
        .iter()
        .map(|f| FindingEntry {
            id: f.id.to_string(),
            fingerprint: f.fingerprint.clone(),
            severity: severity_str(&f.severity),
            confidence: format!("{:?}", f.confidence).to_lowercase(),
            category: format!("{:?}", f.category).to_lowercase(),
            title: f.title.clone(),
            affected_resource: f.affected_resource.clone(),
            evidence_summary: f.evidence.first().map(|e| e.value.clone()).unwrap_or_default(),
            impact: f.impact.clone(),
            read_only_checks: f.read_only_checks.clone(),
            fix_plan: f.fix_plan.clone(),
            risk: "medium".to_string(),
            lifecycle_state: format!("{:?}", f.lifecycle).to_lowercase(),
            first_seen: String::new(),  // Populated from memory store
            last_seen: String::new(),
            occurrence_count: 1,
            detector_id: f.detector_id.clone(),
            correlated_ids: Vec::new(),
        })
        .collect();

    // Services
    let units = &snapshot.domains.services.units;
    let total_services = units.len();
    let failed_services = units.iter().filter(|u| u.active == "failed").count();
    let services: Vec<ServiceEntry> = units.iter().map(|u| ServiceEntry {
        name: u.name.clone(),
        state: u.active.clone(),
        description: u.description.clone(),
    }).collect();

    // Containers
    let containers_raw = &snapshot.domains.containers.containers;
    let total_containers = containers_raw.len();
    let running_containers = containers_raw.iter().filter(|c| c.status == "running").count();
    let containers: Vec<ContainerEntry> = containers_raw.iter().map(|c| ContainerEntry {
        name: c.name.clone(),
        image: c.image.clone(),
        status: c.status.clone(),
        ports: c.ports.clone(),
    }).collect();

    // Port listeners
    let port_listeners: Vec<PortListener> = snapshot.domains.ports.listeners.iter().map(|l| PortListener {
        port: l.local_port,
        protocol: l.protocol.clone(),
        process: l.process_name.clone().unwrap_or_default(),
    }).collect();

    // Processes — derived from port listeners (unique process names)
    let mut process_map: std::collections::HashMap<String, (Option<u32>, Vec<u16>)> = std::collections::HashMap::new();
    for l in &snapshot.domains.ports.listeners {
        if let Some(ref name) = l.process_name {
            let entry = process_map.entry(name.clone()).or_insert((l.pid, Vec::new()));
            entry.1.push(l.local_port);
        }
    }
    let processes: Vec<ProcessEntry> = process_map.into_iter().map(|(name, (pid, ports))| ProcessEntry {
        name,
        pid,
        ports,
    }).collect();

    // Log entries
    let mut log_entries: Vec<String> = Vec::new();
    log_entries.extend(snapshot.domains.logs.kernel_errors.iter().cloned());
    log_entries.extend(snapshot.domains.logs.auth_failures.iter().cloned());
    if log_entries.is_empty() {
        log_entries.push("No recent errors detected".to_string());
    }

    // Backups
    let backup_count = snapshot.domains.backups.tools_detected.len();
    let stale_backups = 0; // Future: detect stale backups

    // Collector health — all domains, marking unhealthy ones from errors
    let unhealthy: std::collections::HashSet<String> = snapshot
        .collector_errors
        .iter()
        .map(|e| format!("{:?}", e.domain).to_lowercase())
        .collect();
    let collector_health: Vec<CollectorHealth> = helm_monitor::SnapshotDomains::domain_names()
        .iter()
        .map(|&d| {
            let name = d.to_string();
            let is_healthy = !unhealthy.contains(&name);
            let error = if is_healthy { None } else {
                snapshot.collector_errors.iter()
                    .find(|e| format!("{:?}", e.domain).to_lowercase() == name)
                    .map(|e| e.message.clone())
            };
            CollectorHealth { domain: name, healthy: is_healthy, error }
        })
        .collect();

    let cpu_percent = if load.cpu_logical_count > 0 {
        ((load.load_average.one / load.cpu_logical_count as f64) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let live = LiveSnapshot {
        hostname: snapshot.host.hostname.clone(),
        collected_at: chrono::Utc::now().format("%H:%M:%S UTC").to_string(),
        profile: "quick".to_string(),
        cpu_percent,
        load_1m: load.load_average.one,
        load_5m: load.load_average.five,
        load_15m: load.load_average.fifteen,
        memory_used_pct: mem_pct,
        memory_total_gb: mem_total as f64 / 1_073_741_824.0,
        swap_used_pct: swap_pct,
        disk_entries,
        total_services,
        failed_services,
        total_containers,
        running_containers,
        listening_ports: snapshot.domains.ports.listeners.len(),
        port_listeners,
        journal_errors: snapshot.domains.logs.journal_errors_last_hour,
        log_entries,
        backup_count,
        stale_backups,
        findings,
        services,
        containers,
        processes,
        collector_health,
        fleet_hosts: {
            crate::remote::RemoteRegistry::load()
                .map(|r| r.remotes.iter().map(|remote| FleetHost {
                    hostname: remote.host.clone(),
                    status: "unknown".to_string(),
                    findings_count: 0,
                    last_seen: String::new(),
                }).collect())
                .unwrap_or_default()
        },
    };

    serde_json::to_string(&live).unwrap_or_default()
}

fn severity_str(s: &Severity) -> String {
    match s {
        Severity::Critical => "critical".to_string(),
        Severity::Warning => "warning".to_string(),
        Severity::Info => "info".to_string(),
    }
}
