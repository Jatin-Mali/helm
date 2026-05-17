# Phase 3 — M3: Fleet (Multi-Host)

## Goal

One HELMOPS dashboard covering N remote hosts in parallel. Operator sees `fleet: 12/12 up`
in the header, switches to the Fleet tab for per-host CRIT/WARN/INFO counts, and drills into
any host's finding queue with a single keystroke.

---

<domain>

## Scope

### In scope

- `RemoteEntry` credential field (`SshAgent | KeyFile(PathBuf)`) in `remotes.toml`
- Parallel SSH `JoinSet` in `RemoteRegistry` — collect findings from N hosts concurrently
- `FleetHostStatus` struct aggregating per-host finding counts + reachability
- `DashboardData.fleet_hosts: Vec<FleetHostStatus>` ring-buffered on each background tick
- `OpsTab::Fleet` as new tab variant; Fleet tab renders per-host summary table
- Status-bar header: `fleet: N/N up` (reachable / total)
- DB migration `0011_host_id_snapshots.sql` — add `host_id TEXT` column to snapshots table
- `helm monitor --format json` output flag for remote collection

### Out of scope (deferred)

- Password credential type (never stored in remotes.toml; deferred to future hardening)
- `secrets.toml` integration for credentials (key paths are not secrets)
- Tab collapse from 7 → 5 variants (M1 Slice 1.3 deferred; Fleet adds as variant #8 for now)
- K8s / libvirt / Compose collectors (M4)
- Per-host sparklines in Fleet tab (post-M3)
- Fleet-wide finding merge / cross-host correlation (post-M3)

</domain>

---

<decisions>

## Locked Decisions (from M3 smart discuss, 2026-05-18)

**D-M3-1 — Credential model:** `enum Credential { SshAgent, KeyFile(PathBuf) }` only.
No password type in M3. No `secrets.toml` integration. Stored inline in `remotes.toml`
as a TOML-serializable enum. `ssh_argv()` extended to inject `-i <path>` for KeyFile.

**D-M3-2 — Remote collection protocol:** SSH + `helm monitor --format json` on remote.
Requires `helm` binary installed on each remote host (same model as old agent-remote, minus
the NDJSON event loop). Add `--format json` (or `--json`) flag to `helm monitor` CLI.
Each host returns a JSON array of `FindingSummary`-compatible objects. Local fleet refresh
deserializes, tags with `host_id`, and stores in `fleet_hosts`.

**D-M3-3 — Fleet tab position:** `OpsTab::Fleet` added as new variant. Tab collapse
(Processes/Network/Disk → Resources) is deferred; Fleet is the 8th tab in render order
for now. The key UX win (fleet header count + drilldown) is independent of tab count.

**D-M3-4 — Fleet aggregation model:** Fleet tab = per-host summary view. Main finding
queue (Alerts tab) stays single-host, driven by `active_remote`. Fleet tab rows show
`| Name | Status | CRIT | WARN | INFO | Last refresh |`. Selecting a row switches
`active_remote` and triggers a dashboard refresh for that host.

**D-M3-5 — Parallel refresh:** `tokio::task::JoinSet<(Uuid, Result<Vec<FleetFinding>>)>`
spawned inside the existing background refresh tick. Results stored in
`DashboardData.fleet_hosts`. Concurrency bounded to 20 (matches benchmark target).
Refresh interval: same as main dashboard interval from `thresholds.toml`.

</decisions>

---

<code_context>

## Key Files

| File | Relevance |
|------|-----------|
| `helm-cli/src/remote.rs` | Add `Credential` enum + field to `RemoteEntry`; add `collect_findings()` async method; add `parallel_collect()` on `RemoteRegistry` |
| `helm-cli/src/tui.rs:833` | `OpsTab` enum — add `Fleet` variant |
| `helm-cli/src/tui.rs:981` | `DashboardData` struct — add `fleet_hosts: Vec<FleetHostStatus>` |
| `helm-cli/src/tui.rs:7714` | Status bar render — add `fleet: N/N up` |
| `helm-cli/src/main.rs` | `helm monitor` CLI — add `--format json` flag |
| `crates/helm-memory/src/snapshots.rs` | Extend with `host_id` column (migration 0011) |
| `crates/helm-memory/migrations/` | Add `0011_host_id_snapshots.sql` |

## Existing Structures (current state)

```rust
// remote.rs — CURRENT
pub struct RemoteEntry {
    pub host_id: Uuid,        // already present
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub ssh_opts: Option<String>,
    // MISSING: credential field
}
```

```rust
// tui.rs:833 — CURRENT OpsTab (7 variants, pre-collapse)
pub enum OpsTab { Alerts, Services, Processes, Logs, Network, Disk, Changes }
// TARGET: add Fleet variant
```

```rust
// tui.rs:981 — DashboardData (current, abbreviated)
struct DashboardData {
    hostname: String,
    findings: Vec<FindingSummary>,
    cpu_history: VecDeque<f64>,   // M2 sparklines already present
    // MISSING: fleet_hosts
}
```

## New Structures to Add

```rust
// remote.rs — Credential enum
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Credential {
    SshAgent,
    KeyFile(PathBuf),
}
impl Default for Credential { fn default() -> Self { Self::SshAgent } }

// Add to RemoteEntry:
#[serde(default)]
pub credential: Credential,
```

```rust
// new: FleetHostStatus in remote.rs or tui.rs
#[derive(Debug, Clone)]
pub struct FleetHostStatus {
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

```rust
// DashboardData addition
fleet_hosts: Vec<FleetHostStatus>,
```

</code_context>

---

<specifics>

## Slice Breakdown

### Slice 3.1 — Credential abstraction in RemoteEntry

- Add `Credential` enum to `remote.rs`
- Add `credential: Credential` field to `RemoteEntry` (default: SshAgent)
- Extend `ssh_argv()` to inject `-i <path>` when `Credential::KeyFile`
- Unit test: `ssh_argv_with_keyfile_injects_identity_flag`
- No migration needed (TOML file; old entries get SshAgent default via `#[serde(default)]`)

### Slice 3.2 — Parallel SSH JoinSet + findings collection

- Add `async fn collect_findings(&self) -> Result<Vec<FleetFinding>>` to `RemoteEntry`
  - SSH + `helm monitor --format json` via `ssh_argv()`
  - Parse stdout as `Vec<serde_json::Value>`, map to `FleetFinding { host_id, .. }`
  - Timeout: 30s per host
- Add `async fn parallel_collect(&self) -> Vec<(Uuid, Result<Vec<FleetFinding>>)>` to `RemoteRegistry`
  - `JoinSet` bounded to 20 concurrent tasks
- Add `--format json` flag to `helm monitor` CLI (prints JSON array, no color/formatting)
- Integration test: mock SSH responder returning canned JSON, assert parallel_collect returns findings with correct host_id tags
- Benchmark: 20-host mock fixture completes in ≤2s

### Slice 3.3 — DashboardData fleet integration + background refresh

- Add `fleet_hosts: Vec<FleetHostStatus>` to `DashboardData`
- In background refresh tick, spawn `parallel_collect()` as a separate `tokio::task`
  - Store results back into `fleet_hosts` on completion
  - Non-blocking: if still running from last tick, skip (don't spawn second one)
- Status bar: append `fleet: N/N up` where N = reachable count / total
- Unit test: fleet_hosts correctly derived from parallel_collect results

### Slice 3.4 — Fleet tab in TUI

- Add `OpsTab::Fleet` variant to `OpsTab` enum
- Implement `render_fleet_tab()`: table rows `| Name | Status | CRIT | WARN | INFO | Last |`
- Keyboard: selecting a Fleet row + Enter switches `active_remote` and triggers refresh
- Snapshot test: Fleet tab renders correctly with 3 mock `FleetHostStatus` entries

### Slice 3.5 — DB migration for host_id in snapshots

- Add `0011_host_id_snapshots.sql`: `ALTER TABLE snapshots ADD COLUMN host_id TEXT`
- Update `snapshots.rs` `store()` and query functions to populate/read `host_id`
- Keyed by both `host_hostname` (legacy) and `host_id` (new) for lookup

## M3 Gate

```bash
# 20-host mock SSH benchmark
cargo test fleet_parallel_refresh_20_hosts -- --ignored

# HMAC chain verifies after multi-host episode
cargo test hmac_chain_verifies_after_fleet_episode

# Full gate
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

Acceptance: 20-host mock completes in ≤2s p99; HMAC chain verifies; Fleet tab renders; header shows fleet count.

</specifics>

---

<deferred>

## Deferred Items

- Tab collapse (7 → 5 variants: Alerts, Services, Resources, Logs, Changes): block on M1 Slice 1.3
- Password credential type: deferred to post-M3 security hardening
- Cross-host finding correlation: post-M4 (needs K8s/infra context)
- Per-host sparklines in Fleet tab: post-M5 (after alerting infrastructure)

</deferred>
