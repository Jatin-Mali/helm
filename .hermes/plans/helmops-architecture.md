# HELMOPS Web Dashboard — Architecture Design
> Canonical revision: 2026-05-21

## Architecture Decision: Embedded Single-Page Application

**Why no framework:** No React/Vue/Angular. No build step. No node_modules.
**What we use:** Single HTML file + vanilla JavaScript + WebSocket + CSS custom properties.
**Why:** Zero dependencies, instant load, no compilation, works on any browser, easy to audit.

```
┌─────────────────────────────────────────────────────────┐
│                    Browser (Client)                      │
│  ┌───────────────────────────────────────────────────┐  │
│  │              index.html (embedded JS)              │  │
│  │  ┌─────────┐  ┌──────────┐  ┌──────────────────┐  │  │
│  │  │ Header  │  │ Status   │  │  Right Metrics   │  │  │
│  │  │ Bar     │  │ Strip    │  │  (load/mem/cpu)   │  │  │
│  │  └─────────┘  └──────────┘  └──────────────────┘  │  │
│  │  ┌──────────────────────────────────────────────┐  │  │
│  │  │              Tab Bar (1-8)                    │  │  │
│  │  └──────────────────────────────────────────────┘  │  │
│  │  ┌──────────┐  ┌────────────────────────────────┐  │  │
│  │  │ Finding  │  │        Main Content              │  │  │
│  │  │ Queue    │  │  (tab-dependent)                 │  │  │
│  │  │ (240px)  │  │                                  │  │  │
│  │  │          │  │                                  │  │  │
│  │  └──────────┘  └────────────────────────────────┘  │  │
│  │  ┌──────────────────────────────────────────────┐  │  │
│  │  │              Footer (keybindings)             │  │  │
│  │  └──────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────┘  │
│           │  WebSocket (ws://localhost:9120/ws)         │
└───────────┼───────────────────────────────────────────┘
            │
┌───────────┼───────────────────────────────────────────┐
│           │         Rust Backend (helm web)            │
│  ┌────────┴────────────────────────────────────────┐  │
│  │              Axum HTTP Server                     │  │
│  │  GET  /              → serve index.html           │  │
│  │  GET  /ws            → WebSocket upgrade          │  │
│  │  GET  /api/snapshot   → JSON snapshot             │  │
│  │  GET  /api/findings   → JSON findings list        │  │
│  │  POST /api/plan       → generate troubleshooting  │  │
│  │  POST /api/apply      → execute fix step          │  │
│  │  GET  /api/history    → historical data           │  │
│  └──────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────┐  │
│  │         Background Collector (tokio::spawn)       │  │
│  │  Every N seconds:                                 │  │
│  │    1. Run MonitorProfile::Quick                  │  │
│  │    2. Serialize snapshot + findings to JSON       │  │
│  │    3. Broadcast to all WebSocket clients          │  │
│  │    4. Persist to SQLite                           │  │
│  └──────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────┐  │
│  │         Existing HELM Core (reused)               │  │
│  │  helm-monitor: collectors, detectors, snapshots   │  │
│  │  helm-memory:  SQLite persistence                │  │
│  │  helm-core:    types, taint, validation           │  │
│  │  helm-providers: LLM backends                     │  │
│  └──────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────┘
```

## Data Flow

```
User opens http://localhost:9120
  → Browser loads index.html
  → JS opens WebSocket to ws://localhost:9120/ws
  → Server sends initial LiveSnapshot JSON
  → JS renders header, status strip, finding queue
  → Every 5 seconds: server pushes new snapshot
  → JS diffs and updates only changed elements

User clicks a finding in queue
  → JS renders detail panel (10 workspace sections)
  → Fetches /api/findings/:id for full data
  → Shows WHAT BROKE → IMPACT → EVIDENCE → FIX PLAN

User presses Alt+G (or clicks "Generate Plan")
  → POST /api/plan with finding_id
  → Backend calls LLM with system context
  → Returns structured troubleshooting plan
  → JS renders hypothesis tree + command previews

User clicks "Apply" on a fix step
  → POST /api/apply with step_id
  → Backend shows approval gate
  → After approval, executes command
  → Returns verification results
```

## LiveSnapshot JSON Schema

```json
{
  "hostname": "string",
  "collected_at": "ISO8601",
  "profile": "quick|standard|deep",
  "cpu_percent": 0.0,
  "load_1m": 0.0, "load_5m": 0.0, "load_15m": 0.0,
  "memory_used_pct": 0.0,
  "memory_total_gb": 0.0,
  "swap_used_pct": 0.0,
  "disk_entries": [{"mount": "/", "used_pct": 0.0, "total_gb": 0.0}],
  "total_services": 0, "failed_services": 0,
  "total_containers": 0, "running_containers": 0,
  "listening_ports": 0,
  "journal_errors": 0,
  "backup_count": 0, "stale_backups": 0,
  "findings": [
    {
      "id": "string",
      "severity": "critical|warning|info",
      "confidence": "high|medium|low",
      "category": "disks|services|containers|...",
      "title": "string",
      "affected_resource": "string",
      "evidence_summary": "string",
      "impact": "string",
      "read_only_checks": ["string"],
      "fix_plan": "string|null",
      "risk": "low|medium|high",
      "lifecycle_state": "new|recurring|suppressed|resolved",
      "first_seen": "ISO8601",
      "last_seen": "ISO8601",
      "occurrence_count": 0
    }
  ],
  "collector_health": [
    {"domain": "string", "healthy": true, "error": "string|null"}
  ],
  "fleet_hosts": [
    {"hostname": "string", "status": "online|offline|error",
     "findings_count": 0, "last_seen": "ISO8601"}
  ]
}
```

## Tech Stack

| Layer | Technology | Why |
|-------|-----------|-----|
| HTTP Server | axum 0.8 | Rust-native, async, WebSocket support built-in |
| WebSocket | axum::extract::ws | Same crate, no extra dependency |
| Frontend | Single HTML + vanilla JS + CSS | Zero build step, instant load |
| Styling | CSS custom properties + dark theme | Themable, no preprocessor needed |
| Data | JSON over WebSocket | Simple, human-readable, debuggable |
| Persistence | SQLite (existing helm-memory) | Already integrated, zero config |
| LLM | Existing helm-providers | Already supports 7 backends |
| Templating | None (JSON → DOM via JS) | No server-side rendering needed |
