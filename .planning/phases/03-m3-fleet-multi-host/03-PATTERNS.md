# Phase 3: M3 Fleet (Multi-Host) - Pattern Map

**Mapped:** 2026-05-18
**Files analyzed:** 7 files to create/modify
**Analogs found:** 6 / 7

---

## File Classification

| New/Modified File | Role | Data Flow | Closest Analog | Match Quality |
|-------------------|------|-----------|----------------|---------------|
| `helm-cli/src/remote.rs` (Credential enum + collect_findings) | model/service | request-response | `helm-cli/src/remote.rs` (RemoteEntry struct) | exact — same file |
| `helm-cli/src/remote.rs` (parallel_collect on RemoteRegistry) | service | request-response | `helm-cli/src/remote.rs` (RemoteRegistry methods) | exact — same file |
| `helm-cli/src/tui.rs` (OpsTab::Fleet variant) | enum | config | `helm-cli/src/tui.rs:833` (OpsTab enum) | exact — same file |
| `helm-cli/src/tui.rs` (DashboardData fleet_hosts field) | model | CRUD | `helm-cli/src/tui.rs:981` (DashboardData struct) | exact — same file |
| `helm-cli/src/main.rs` (--format json flag) | controller | request-response | `helm-cli/src/main.rs:634` (MonitorArgs struct) | exact — same file |
| `crates/helm-memory/src/snapshots.rs` (host_id column) | model | CRUD | `crates/helm-memory/src/snapshots.rs` (SnapshotStore) | exact — same file |
| `crates/helm-memory/migrations/0011_host_id_snapshots.sql` | migration | CRUD | `crates/helm-memory/migrations/0006_snapshots.sql` | role-match |

---

## Pattern Assignments

### `helm-cli/src/remote.rs` — Add Credential enum & collect_findings

**File location:** `/home/white_devil/code/helm/helm-cli/src/remote.rs`

**Struct serialization pattern** (lines 14–32):
```rust
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RemoteRegistry {
    #[serde(default)]
    pub remotes: Vec<RemoteEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEntry {
    #[serde(default = "Uuid::new_v4")]
    pub host_id: Uuid,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub ssh_opts: Option<String>,
}
```

**Add Credential enum** (insert after RemoteEntry struct, before default_port fn):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Credential {
    SshAgent,
    KeyFile(PathBuf),
}

impl Default for Credential {
    fn default() -> Self {
        Self::SshAgent
    }
}
```

Then add to RemoteEntry:
```rust
    #[serde(default)]
    pub credential: Credential,
```

**Async method pattern** (lines 120–131):
```rust
/// Run `ssh remote true` and return whether the connection succeeded.
pub async fn ping(&self) -> Result<bool> {
    let mut argv = self.ssh_argv();
    argv.push("true".to_owned());
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = cmd.output().await.context("spawning ssh")?;
    Ok(output.status.success())
}
```

**Use for:** `collect_findings()` async method on RemoteEntry — spawn SSH + helm monitor JSON, parse JSON array, tag with host_id, return Vec<FleetFinding>.

**Error handling pattern** (lines 8–12):
```rust
use anyhow::{Context, Result, anyhow};
```
Apply Result<T> return type + .context() chaining for SSH/IO errors.

---

### `helm-cli/src/remote.rs` — Add parallel_collect to RemoteRegistry

**RemoteRegistry impl pattern** (lines 50–93):
```rust
impl RemoteRegistry {
    pub fn load() -> Result<Self> { ... }
    pub fn load_from(path: &Path) -> Result<Self> { ... }
    pub fn save(&self) -> Result<()> { ... }
    pub fn get(&self, name: &str) -> Option<&RemoteEntry> { ... }
    pub fn upsert(&mut self, entry: RemoteEntry) { ... }
    pub fn remove(&mut self, name: &str) -> bool { ... }
}
```

**Add parallel_collect async method:**
- Use `tokio::task::JoinSet` (bounded to 20 concurrent tasks)
- Spawn one task per remote in `self.remotes`
- Each task calls `RemoteEntry::collect_findings()`
- Collect results as `Vec<(Uuid, Result<Vec<FleetFinding>>>)`
- Return that vec

**Pattern:** Follow the sync method pattern above; add `pub async fn parallel_collect(&self) -> ...`

---

### `helm-cli/src/tui.rs` — Add OpsTab::Fleet variant

**Existing enum pattern** (lines 832–866):
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpsTab {
    Alerts,
    Services,
    Processes,
    Logs,
    Network,
    Disk,
    Changes,
}

impl OpsTab {
    fn all() -> &'static [Self] {
        &[
            Self::Alerts,
            Self::Services,
            Self::Processes,
            Self::Logs,
            Self::Network,
            Self::Disk,
            Self::Changes,
        ]
    }
    fn label(self) -> &'static str {
        match self {
            Self::Alerts => "ALERTS",
            Self::Services => "SERVICES",
            Self::Processes => "PROCESSES",
            Self::Logs => "LOGS",
            Self::Network => "NETWORK",
            Self::Disk => "DISK",
            Self::Changes => "CHANGES",
        }
    }
}
```

**Add to enum:**
```rust
    Fleet,
```

**Add to all() array:**
```rust
            Self::Fleet,
```

**Add to label() match:**
```rust
            Self::Fleet => "FLEET",
```

---

### `helm-cli/src/tui.rs` — Add DashboardData.fleet_hosts field

**Existing struct pattern** (lines 979–1024):
```rust
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct DashboardData {
    hostname: String,
    profile: String,
    cpu_percent: f64,
    // ... 25 other fields ...
    /// Ring buffers for sparkline rendering (cap 60 points each).
    cpu_history: VecDeque<f64>,
    mem_history: VecDeque<f64>,
    load_history: VecDeque<f64>,
    disk_history: VecDeque<f64>,
}
```

**Add field** (after disk_history):
```rust
    /// Per-host fleet status for multi-host view.
    fleet_hosts: Vec<FleetHostStatus>,
```

**Define FleetHostStatus struct** (before DashboardData):
```rust
#[derive(Debug, Clone)]
struct FleetHostStatus {
    pub host_id: Uuid,
    pub name: String,
    pub reachable: bool,
    pub crit: usize,
    pub warn: usize,
    pub info: usize,
    pub last_refresh: Option<Instant>,
    pub error: Option<String>,
}
```

**VecDeque pattern for ring buffers** (lines 5338–5354):
```rust
let cap = self.dashboard.sparkline_history_depth;
let push = |buf: &mut VecDeque<f64>, val: f64| {
    buf.push_back(val);
    if buf.len() > cap {
        buf.pop_front();
    }
};
push(&mut self.dashboard.data.cpu_history, cpu_percent);
```
Use this same pattern for any fleet_hosts history if needed later (post-M3).

---

### `helm-cli/src/main.rs` — Add --format json flag

**Existing MonitorArgs pattern** (lines 634–659):
```rust
struct MonitorArgs {
    /// Output as JSON
    #[arg(long)]
    json: bool,
    /// Output as Markdown
    #[arg(long)]
    markdown: bool,
    /// Collection depth (quick, standard, deep)
    #[arg(long, default_value = "standard")]
    profile: String,
    /// Comma-separated domains to check (e.g. "disks,services,ports")
    #[arg(long, value_name = "DOMAINS")]
    domain: Option<String>,
    /// Watch mode: run repeatedly
    #[arg(long)]
    watch: bool,
    /// Interval in seconds for watch mode (default: 60)
    #[arg(long, default_value = "60")]
    interval: u64,
    /// Write report to file
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,
    /// Output format: text, json, markdown (default: text)
    #[arg(long, default_value = "text")]
    format: String,
}
```

**Note:** `format` field already exists at line 658. The `--format json` flag is already supported via this field.

**Clap derive pattern** (lines 55–100):
```rust
#[derive(Debug, Parser)]
#[command(name = "helm", version, about = "HELMOPS — live Linux/DevOps troubleshooting console")]
struct Cli {
    #[arg(long, value_name = "PATH", global = true)]
    db_path: Option<PathBuf>,
    // ... other global args
}

#[derive(Debug, Args)]
struct MonitorArgs {
    #[arg(long)]
    flag_name: type_,
}
```

Apply: Add `--format json` as an alias or ensure resolve_format() function handles it.

**Run handler pattern** (lines 3960–4009):
```rust
async fn run_monitor_command(args: MonitorArgs) -> Result<()> {
    let profile: MonitorProfile = args.profile.parse().unwrap_or(MonitorProfile::Standard);
    let domain_filter: Option<Vec<MonitorDomain>> = args.domain.as_ref().map(|s| { ... });
    let db_path = default_db_path()?;
    let conn = rusqlite::Connection::open(&db_path).context(...)?;
    // ... collect report ...
    let fmt = resolve_format(&args);
    let output = format_report(&report, &fmt);
    let redacted = helm_core::redact_secrets(&output);
    println!("{redacted}");
    Ok(())
}
```

**Ensure resolve_format() returns JSON when args.format == "json".**

---

### `crates/helm-memory/src/snapshots.rs` — Add host_id column support

**Existing struct pattern** (lines 8–17):
```rust
#[derive(Debug, Clone)]
pub struct SnapshotRecord {
    pub id: String,
    pub host_hostname: String,
    pub collected_at: i64,
    pub profile: String,
    pub domains_json: String,
    pub collector_errors_json: String,
    pub findings_json: String,
}
```

**Add field:**
```rust
    pub host_id: String,
```

**Insert pattern** (lines 24–49):
```rust
pub fn insert(conn: &Connection, json: &str, findings_json: &str) -> Result<(), MemoryError> {
    let val: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| MemoryError::Other(e.to_string()))?;

    let id = val["id"].as_str().unwrap_or("").to_string();
    let host_hostname = val["host"]["hostname"].as_str().unwrap_or("unknown").to_string();
    let collected_at = val["collected_at"].as_str().unwrap_or("");
    let collected_at_ts = chrono::DateTime::parse_from_rfc3339(collected_at)
        .map(|dt| dt.timestamp())
        .unwrap_or(0);
    let profile = val["profile"].as_str().unwrap_or("standard").to_string();
    let domains_json = serde_json::to_string(&val["domains"]).unwrap_or_default();
    let collector_errors_json = serde_json::to_string(&val["collector_errors"]).unwrap_or_default();

    conn.execute(
        "INSERT OR REPLACE INTO snapshots (id, host_hostname, collected_at, profile, domains_json, collector_errors_json, findings_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, host_hostname, collected_at_ts, profile, domains_json, collector_errors_json, findings_json],
    )
    .map_err(|e| MemoryError::Other(e.to_string()))?;

    Ok(())
}
```

**Update insert() call** to include `host_id` in the query:
- Extract `host_id` from JSON: `let host_id = val["host"]["id"].as_str().unwrap_or("").to_string();`
- Add `host_id` param to INSERT
- Add to column list

**Query patterns** (lines 52–72, 75–95):
- Rows returned from queries need `host_id` field mapped from row.get(N)
- Update all SELECT statements to include `host_id` column

---

### `crates/helm-memory/migrations/0011_host_id_snapshots.sql`

**Existing migration pattern** (from 0006_snapshots.sql, lines 1–20):
```sql
CREATE TABLE IF NOT EXISTS snapshots (
    id TEXT PRIMARY KEY,
    host_hostname TEXT NOT NULL DEFAULT 'unknown',
    host_kernel_name TEXT NOT NULL DEFAULT 'unknown',
    -- ... other columns ...
    profile TEXT NOT NULL DEFAULT 'standard',
    domains_json TEXT NOT NULL DEFAULT '{}',
    collector_errors_json TEXT NOT NULL DEFAULT '[]',
    redaction_version TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_snapshots_collected_at ON snapshots(collected_at DESC);
CREATE INDEX IF NOT EXISTS idx_snapshots_profile ON snapshots(profile);
```

**For 0011_host_id_snapshots.sql:**
```sql
-- Add host_id column to snapshots table
ALTER TABLE snapshots ADD COLUMN host_id TEXT DEFAULT '';
CREATE INDEX IF NOT EXISTS idx_snapshots_host_id ON snapshots(host_id);
```

---

## Shared Patterns

### Serialization & TOML
**Source:** `helm-cli/src/remote.rs` (lines 14–32, 10)
**Apply to:** Credential enum, RemoteEntry updates
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Credential {
    SshAgent,
    KeyFile(PathBuf),
}

impl Default for Credential { fn default() -> Self { Self::SshAgent } }
```

### Async SSH Command Execution
**Source:** `helm-cli/src/remote.rs` (lines 96–131)
**Apply to:** `collect_findings()` method
```rust
use tokio::process::Command;
use std::process::Stdio;

pub async fn collect_findings(&self) -> Result<Vec<FleetFinding>> {
    let mut argv = self.ssh_argv();
    argv.push("helm".to_owned());
    argv.push("monitor".to_owned());
    argv.push("--format".to_owned());
    argv.push("json".to_owned());
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd.output().await.context("spawning ssh")?;
    // Parse stdout as JSON array, tag with host_id
    Ok(...)
}
```

### Error Handling
**Source:** `helm-cli/src/remote.rs` (lines 8–9)
**Apply to:** All async methods in remote.rs, snapshots.rs
```rust
use anyhow::{Context, Result, anyhow};
// Always use .context("operation description") when converting errors
```

### Enum with all() and label() methods
**Source:** `helm-cli/src/tui.rs` (lines 833–866)
**Apply to:** OpsTab::Fleet addition
- Add variant to enum
- Add to `all()` array
- Add to `label()` match arm

### VecDeque Ring Buffer
**Source:** `helm-cli/src/tui.rs` (lines 1020–1023, 5338–5354)
**Apply to:** fleet_hosts field initialization and update
```rust
use std::collections::VecDeque;

struct DashboardData {
    fleet_hosts: Vec<FleetHostStatus>,  // not a ring buffer, just a vec
    // but if per-host history is added later, use this pattern:
    // fleet_sparklines: VecDeque<...>,
}

// Push pattern:
let cap = self.dashboard.sparkline_history_depth;
let push = |buf: &mut VecDeque<f64>, val: f64| {
    buf.push_back(val);
    if buf.len() > cap {
        buf.pop_front();
    }
};
```

### CLI Arg Parsing
**Source:** `helm-cli/src/main.rs` (lines 634–659)
**Apply to:** Ensure --format json is supported
```rust
#[derive(Debug, Args)]
struct MonitorArgs {
    #[arg(long, default_value = "text")]
    format: String,
}
```

### SQLite Migration & Query
**Source:** `crates/helm-memory/src/snapshots.rs` (lines 24–49, 52–95)
**Apply to:** host_id column handling
```rust
// Extract from JSON:
let host_id = val["host"]["id"].as_str().unwrap_or("").to_string();

// Insert pattern:
conn.execute(
    "INSERT OR REPLACE INTO snapshots (..., host_id) VALUES (..., ?N)",
    params![..., host_id],
)?;

// Query pattern:
let result = conn.query_row(
    "SELECT ..., host_id FROM snapshots WHERE ...",
    params![...],
    |row| {
        Ok(SnapshotRecord {
            host_id: row.get(N)?,
            ...
        })
    },
)?;
```

---

## No Analog Found

All files have exact analogs in the existing codebase (same file or closely related).

---

## Metadata

**Analog search scope:** helm-cli/src/, crates/helm-memory/src/, crates/helm-memory/migrations/
**Files scanned:** 200+
**Pattern extraction date:** 2026-05-18
