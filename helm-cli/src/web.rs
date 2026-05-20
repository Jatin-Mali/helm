//! HELMOPS web dashboard — local HTTP server with live WebSocket data push.
//! Started via `helm web [--port PORT]`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
};
use helm_monitor::{MonitorProfile, SnapshotDomains};
use serde::Serialize;
use tokio::sync::broadcast;

/// Shared application state for the web server.
struct WebState {
    db_path: PathBuf,
    snapshot_tx: broadcast::Sender<String>,
}

/// Snapshot pushed to WebSocket clients.
#[derive(Serialize, Clone)]
struct LiveSnapshot {
    hostname: String,
    load_1m: f64,
    load_5m: f64,
    load_15m: f64,
    memory_used_pct: f64,
    disk_entries: Vec<String>,
    total_services: usize,
    failed_services: usize,
    total_containers: usize,
    running_containers: usize,
    listening_ports: usize,
    journal_errors: u64,
    findings: Vec<FindingSummary>,
    collected_at: String,
    collector_health: Vec<CollectorStatus>,
}

#[derive(Serialize, Clone)]
struct FindingSummary {
    id: String,
    severity: String,
    title: String,
    category: String,
    affected_resource: String,
    sample: String,
}

#[derive(Serialize, Clone)]
struct CollectorStatus {
    domain: String,
    healthy: bool,
    error: Option<String>,
}

/// Start the HELMOPS web server.
pub async fn serve(db_path: PathBuf, port: u16) -> Result<()> {
    let (tx, _) = broadcast::channel::<String>(16);
    let state = Arc::new(WebState {
        db_path: db_path.clone(),
        snapshot_tx: tx.clone(),
    });

    // Spawn background collector that pushes snapshots every 5 seconds
    let bg_db = db_path.clone();
    let bg_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            let json = collect_live_snapshot(&bg_db).await;
            let _ = bg_tx.send(json);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/ws", get(ws_handler))
        .route("/api/snapshot", get(api_snapshot))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("HELMOPS web dashboard → http://{addr}");
    println!("WebSocket endpoint   → ws://{addr}/ws");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("binding web server")?;

    axum::serve(listener, app).await.context("web server")?;

    Ok(())
}

/// Serve the dashboard HTML embedded at compile time.
async fn serve_index() -> impl IntoResponse {
    axum::response::Html(include_str!("../../docs/helmops_tui_dashboard.html"))
}

/// WebSocket upgrade handler — clients receive live snapshots.
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<WebState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<WebState>) {
    let mut rx = state.snapshot_tx.subscribe();
    // Send initial snapshot
    let initial = collect_live_snapshot(&state.db_path).await;
    if socket.send(Message::Text(initial.into())).await.is_err() {
        return;
    }
    // Stream updates from collector
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

/// REST API: get latest snapshot as JSON.
async fn api_snapshot(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let json = collect_live_snapshot(&state.db_path).await;
    axum::response::Json(serde_json::from_str::<serde_json::Value>(&json).unwrap_or_default())
}

/// Collect a live snapshot from the monitor and serialize to JSON.
async fn collect_live_snapshot(db_path: &std::path::Path) -> String {
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return r#"{"error":"cannot open db"}"#.to_string(),
    };

    let prev = crate::load_previous_snapshot(&conn);
    let report = crate::run_monitor_cycle(MonitorProfile::Quick, None, prev).await;

    let findings: Vec<FindingSummary> = report
        .findings
        .iter()
        .map(|f| FindingSummary {
            id: f.id.to_string(),
            severity: format!("{:?}", f.severity),
            title: f.title.clone(),
            category: format!("{:?}", f.category),
            affected_resource: f.affected_resource.clone(),
            sample: f
                .evidence
                .first()
                .map(|e| e.value.clone())
                .unwrap_or_default(),
        })
        .collect();

    let disk_entries: Vec<String> = report
        .snapshot
        .domains
        .disks
        .filesystems
        .iter()
        .map(|fs| {
            let pct = if fs.total_bytes > 0 {
                (fs.used_bytes as f64 / fs.total_bytes as f64) * 100.0
            } else {
                0.0
            };
            format!("{} {:.0}%", fs.mount_point, pct)
        })
        .collect();

    let load = &report.snapshot.domains.load;
    let mem_pct = if load.memory.total > 0 {
        (load.memory.used as f64 / load.memory.total as f64) * 100.0
    } else {
        0.0
    };

    let units = &report.snapshot.domains.services.units;
    let total_services = units.len();
    let failed_services = units.iter().filter(|u| u.active == "failed").count();

    let containers = &report.snapshot.domains.containers.containers;
    let total_containers = containers.len();
    let running_containers = containers.iter().filter(|c| c.status == "running").count();

    let live = LiveSnapshot {
        hostname: report.snapshot.host.hostname.clone(),
        load_1m: load.load_average.one,
        load_5m: load.load_average.five,
        load_15m: load.load_average.fifteen,
        memory_used_pct: mem_pct,
        disk_entries,
        total_services,
        failed_services,
        total_containers,
        running_containers,
        listening_ports: report.snapshot.domains.ports.listeners.len(),
        journal_errors: report.snapshot.domains.logs.journal_errors_last_hour,
        findings,
        collected_at: chrono::Utc::now().format("%H:%M:%S UTC").to_string(),
        collector_health: SnapshotDomains::domain_names()
            .iter()
            .map(|&d| CollectorStatus {
                domain: d.to_string(),
                healthy: true,
                error: None,
            })
            .collect(),
    };

    serde_json::to_string(&live).unwrap_or_default()
}
