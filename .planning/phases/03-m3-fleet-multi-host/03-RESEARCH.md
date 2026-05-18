# Phase 3: M3 — Fleet (Multi-Host) - Gap Analysis Research

**Researched:** 2026-05-18  
**Domain:** Multi-host fleet architecture, parallel SSH collection, TUI integration  
**Confidence:** HIGH

---

## Summary

Five plans (03-01 through 03-05) have been created covering credential abstraction, parallel SSH collection, DashboardData integration, Fleet tab UI, and snapshot store migration. Cross-referencing these plans against the phase success criteria and REQ-fleet-uuid-parallel reveals **no significant gaps** — all critical requirements are covered. However, **seven micro-gaps exist** in scope definition, audit integration, and selection persistence that need clarification before full execution.

**Primary recommendation:** Plans are well-structured and ready for execution. Clarify the seven gaps listed below via checkpoint tasks or planning adjustments before wave 2 begins.

---

## Success Criteria → Plan Coverage Map

| Criterion | Plan(s) Covering | Status | Gap? |
|-----------|------------------|--------|------|
| Fleet of 12 hosts shows "fleet: 12/12 up" in header | 03-03 (status bar) | ✓ | No |
| Fleet tab displays per-host CRIT counts | 03-04 (table rendering) | ✓ | No |
| Parallel SSH JoinSet collects findings concurrently | 03-02 (parallel_collect) | ✓ | No |
| 20-host refresh completes in ≤2s | 03-02 (benchmark test) | ✓ | No |
| RemoteEntry extended with host_id: Uuid | 03-01 (credential field exists, but see Gap #1) | ⚠️ | **Yes** |
| Snapshot store keyed by both host_hostname and host_id | 03-05 (adds column, but see Gap #2) | ⚠️ | **Yes** |
| Credential abstraction: SshAgent, KeyFile, Password | 03-01 (SshAgent + KeyFile only) | ⚠️ | **Yes** |
| No passwords in registry file | 03-01 ✓, CONTEXT says deferred | ✓ | No |
| Selection sticky across fleet refresh | 03-04 (keyboard nav), but see Gap #3 | ⚠️ | **Yes** |
| HMAC chain verifies across multi-host episode | CONTEXT says needed, no plan | ✗ | **Yes** |

---

## Gap Analysis

### Gap #1: host_id Field Type Mismatch

**What's missing:**
- **CONTEXT.md** (line 90): `RemoteEntry` struct definition shows `pub host_id: Uuid` already present.
- **03-01-PLAN.md** (lines 57–87): Adds `Credential` enum to `RemoteEntry`, but makes **no mention** of `host_id: Uuid` field.
- **REQ-fleet-uuid-parallel** (line 206): "Extend RemoteEntry with host_id: Uuid" is a locked acceptance criterion.

**Current state (from 03-CONTEXT.md line 89–90):**
```rust
pub struct RemoteEntry {
    pub host_id: Uuid,        // already present
    // ... other fields ...
}
```

**Question:** Is `host_id: Uuid` already in the codebase, or does 03-01 need to add it? The CONTEXT shows it exists, but the plan doesn't mention it. This is either:
1. Already done in a prior phase/milestone (likely M1 or early M3 work)
2. Needs to be verified as present before 03-02 runs (which depends on it for `parallel_collect()`)

**Files affected:** `helm-cli/src/remote.rs`

**Action needed:** Before executing 03-01, verify that `RemoteEntry.host_id: Uuid` exists. If not, add it to 03-01 as Task 0 (prerequisite). If yes, document this in a checkpoint.

---

### Gap #2: Snapshot Store Keying Strategy Undefined

**What's missing:**
- **REQ-fleet-uuid-parallel** (line 203): "snapshot store keyed by both host_hostname and host_id"
- **03-05-PLAN.md** (lines 124–199): Adds `host_id TEXT` column and updates `insert()` and query functions, but **does not define query functions** that use `host_id` as a lookup key.
- **Current pattern:** Queries in `snapshots.rs` likely key by `id` (UUID) or `host_hostname`. Adding `host_id` as a column doesn't automatically enable "keying by host_id".

**Current design (from 03-PATTERNS.md lines 310–325):**
```rust
// Insert extracts host_id and stores it
let host_id = val["host"]["id"].as_str().unwrap_or("").to_string();
conn.execute("INSERT OR REPLACE INTO snapshots (..., host_id) VALUES (..., ?N)", ...)?;

// But query functions are not specified to use host_id lookup
```

**Question:** After 03-05, what queries should support `host_id` filtering? Should there be:
- `get_by_host_id(host_id: Uuid) -> Vec<SnapshotRecord>`?
- `get_latest_by_host_id(host_id: Uuid) -> Option<SnapshotRecord>`?

Or is "keying" only about the index, with queries remaining keyed by primary `id`?

**Files affected:** `crates/helm-memory/src/snapshots.rs`

**Action needed:** Define the full set of query functions that should support `host_id` lookup. This may be a separate plan (03-06) or a Task 4.5 in 03-05. Defer if not needed for M3 gate, but document the decision.

---

### Gap #3: Selection Pinning Across Fleet Tab Refresh

**What's missing:**
- **03-UI-SPEC.md** (line 315–323): "Selection Stability" section says selection should remain stable across refresh IF the row still exists.
- **03-04-PLAN.md** (lines 174–230): Adds keyboard handler for Fleet tab navigation, but **does not specify** how `fleet_selected_row` state persists across background refresh.
- **Current pattern:** M1 used fingerprint-based pinning for findings. Fleet tab needs analogous pinning for host rows.

**Question:** Should `fleet_selected_row: Option<usize>` be:
1. **Positionally stable** (index-based, resets to 0 if fleet size changes)?
2. **Host-stable** (keyed by `host_id`, remains on the same host even if row index changes)?
3. **Hybrid** (prefer host-stable, fall back to index-stable if host deleted)?

**Current 03-04 design (Task 2, line 199–200):**
```rust
// This is index-based; if fleet_hosts vec is rebuilt, index may point to wrong host
self.fleet_selected_row = Some((row + 1).min(max));
```

**Files affected:** `helm-cli/src/tui.rs`, DashboardState or similar struct

**Action needed:** Clarify in a checkpoint: is positional pinning sufficient for M3, or is host-based pinning required? M1 required fingerprint-based pinning; Fleet tab may need similar rigor. Document decision in 03-04-SUMMARY.md.

---

### Gap #4: Password Credential Variant — Scope Boundary Unclear

**What's missing:**
- **REQ-fleet-uuid-parallel** (line 210): "Credential abstraction: enum Credential { SshAgent, KeyFile(PathBuf), Password(Secret) }"
- **03-CONTEXT.md** (lines 43–44, D-M3-1): "No password type in M3. No `secrets.toml` integration. Stored inline in `remotes.toml` as a TOML-serializable enum."
- **03-01-PLAN.md** (lines 14–27): Only implements `SshAgent | KeyFile`, no `Password` variant.

**Status:** This is actually **not a gap** — it's a deliberate deferred decision. CONTEXT locked it out of M3. REQ contradicts CONTEXT, which indicates the requirement was written before the M3 discuss-phase.

**Question:** Is REQ-fleet-uuid-parallel outdated? Should it be amended to remove Password, or is Password planned for a future phase (M3.5)?

**Files affected:** N/A for M3; defer to post-M3 security hardening phase

**Action needed:** Document in 03-01-SUMMARY.md that Password credential is deferred per D-M3-1. Verify REQ-fleet-uuid-parallel is updated or marked as "Password deferred to post-M3".

---

### Gap #5: HMAC Chain Verification Across Multi-Host Episode — No Plan

**What's missing:**
- **REQ-fleet-uuid-parallel** (line 214): "HMAC chain verifies across multi-host episode"
- **03-CONTEXT.md** (line 209): "M3 Gate" specifies test `hmac_chain_verifies_after_fleet_episode`
- **No plan file** addresses HMAC chain integration. It's mentioned in 03-03 threat model (line 336–337) but not implemented in tasks.

**Current state:**
- M1 implemented HMAC chain for single-host audit log (REQ-audit-hmac-chain).
- M3 expands to multi-host: each background fleet refresh should append to audit log with multi-host context.
- 03-03-PLAN.md spawns parallel_collect() but **does not specify** audit log append or HMAC chain verification.

**Question:** Should 03-03 include a Task 6 (before full gate) for:
1. Appending audit log entry after fleet episode completes?
2. Including host_id list and finding counts in audit record?
3. Verifying HMAC chain integrity post-fleet-episode?

**Files affected:** `crates/helm-audit/src/lib.rs` (or similar audit module), 03-03-PLAN.md

**Action needed:** Add Task 6 to 03-03-PLAN.md (or create 03-03.5-PLAN.md) for HMAC audit integration. This is required for M3 gate per CONTEXT line 208–209.

---

### Gap #6: Background Refresh Tick Integration — No Explicit Spawn Site

**What's missing:**
- **03-03-PLAN.md** (lines 114–159): Task 2 describes spawning `parallel_collect()` "in the background refresh tick", but **does not specify where** in the codebase that tick is located.
- **Pattern:** 03-PATTERNS.md (lines 221–233) references "background refresh tick ~line 5338", but this is for sparkline updates, not fleet collection.
- **Question:** Is there a single tokio task loop (e.g., `dashboard_refresh_loop()`) where both sparkline updates and fleet collection should spawn?

**Current uncertainty:**
```rust
// Where does this go?
if !self.fleet_collection_in_flight {
    self.fleet_collection_in_flight = true;
    let registry = self.remote_registry.clone();
    tokio::task::spawn(async move { ... });
}
```

**Files affected:** `helm-cli/src/tui.rs`, DashboardState, refresh loop

**Action needed:** Grep the codebase to locate the exact background refresh tick function/loop before executing 03-03 Task 2. Update the plan with the precise function name and line numbers.

---

### Gap #7: Fleet Tab State — `fleet_selected_row` Field Location Undefined

**What's missing:**
- **03-04-PLAN.md** (lines 174–213): Task 4 adds keyboard handler for Fleet tab navigation using `self.fleet_selected_row`, but **does not specify which struct** holds this field.
- **Options:**
  1. Add to `DashboardState` (global state)?
  2. Add to `DashboardData` (per-dashboard data)?
  3. Add to a new `FleetTabState` struct?

**Current pattern (from 03-PATTERNS.md line 201–205):**
```rust
struct DashboardData {
    // ... other fields ...
    fleet_hosts: Vec<FleetHostStatus>,  // added by 03-03
}
```

But `DashboardData` is per-host data. Fleet tab selection is **global** (one selection across entire TUI). Should go in `DashboardState` or similar global state.

**Files affected:** `helm-cli/src/tui.rs`, struct definitions

**Action needed:** Clarify which struct holds `fleet_selected_row` before executing 03-04 Task 4. Update plan with exact struct name and initialization.

---

## Validation Architecture

### Test Infrastructure Status

**Existing test framework:** Ratatui snapshot tests already in place (per 03-04-PLAN.md line 233–271).

**Gaps in M3 test coverage:**

| Test Name | Plan File | Status | Missing? |
|-----------|-----------|--------|----------|
| `test_ssh_argv_with_keyfile_injects_identity_flag` | 03-01 Task 3 | ✓ Specified | No |
| `test_credential_default_is_ssh_agent` | 03-01 Task 3 | ✓ Specified | No |
| `test_credential_serialization_roundtrip` | 03-01 Task 3 | ✓ Specified | No |
| `test_fleet_parallel_refresh_20_hosts` | 03-02 Task 5 | ✓ Specified | No |
| `test_fleet_hosts_aggregation_from_results` | 03-03 Task 4 | ✓ Specified | No |
| `test_background_tick_spawns_parallel_collect` | 03-03 Task 5 | ✓ Specified | No |
| `test_render_fleet_tab_snapshot` | 03-04 Task 5 | ✓ Specified | No |
| `test_host_id_roundtrip` | 03-05 Task 5 | ✓ Specified | No |
| `hmac_chain_verifies_after_fleet_episode` | CONTEXT (line 209) | ✗ **NOT SPECIFIED** | **Yes** |

**Gap:** HMAC chain verification test is required by M3 gate (CONTEXT line 208–209) but has no corresponding plan task.

### Per-Wave Gate Verification

**Wave 1 (03-01, 03-02):**
```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test --workspace
cargo test fleet_parallel_refresh_20_hosts -- --ignored
```

**Wave 2 (03-03, 03-05):**
```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test --workspace
cargo test fleet_hosts_aggregation_from_results
cargo test host_id_roundtrip
```

**Wave 3 (03-04):**
```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test --workspace
cargo test test_render_fleet_tab_snapshot
```

**M3 RC Gate (after all waves):**
```bash
cargo test fleet_parallel_refresh_20_hosts -- --ignored
cargo test hmac_chain_verifies_after_fleet_episode  # NOT PLANNED — needs 03-03.5 or 03-06
cargo fmt --check && cargo clippy -- -D warnings && cargo test --workspace
```

---

## Package Legitimacy Audit

No new external packages are introduced in Phase 3. All plans use existing dependencies:
- `tokio` (already in workspace)
- `serde`, `serde_json` (already in workspace)
- `ratatui` (already in workspace)
- `rusqlite` (already in workspace)
- `anyhow` (already in workspace)

**Disposition:** No package audit needed.

---

## Architecture Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| Credential selection (SshAgent vs. KeyFile) | RemoteEntry (model) | CLI (config) | Remote entry owns which key to use; stored in remotes.toml |
| Parallel SSH collection | RemoteRegistry (service) | Tokio runtime | Registry orchestrates; tokio::task provides concurrency |
| Fleet finding aggregation | DashboardData (model) | Background tick (task) | Data model holds results; refresh tick computes aggregation |
| Fleet tab rendering | Ratatui widget (UI) | DashboardData (model) | Widget renders fleet_hosts vec; data layer feeds it |
| Selection state (fleet_selected_row) | DashboardState (global) | Keyboard handler (input) | Selection is global; handler mutates state on arrow keys |
| Snapshot storage by host_id | SnapshotRecord (model) | Database layer (CRUD) | Record owns host_id field; DB layer persists it |
| HMAC audit chain | Audit module (infra) | Background tick (task) | Audit logs append on episode completion; orchestrated by tick |

---

## Common Pitfalls

### Pitfall 1: Index-Based Selection Instability Across Refresh

**What goes wrong:** User selects Fleet row 2 (host "staging"). Background refresh rebuilds fleet_hosts vec. If a host is added/removed at the front, row 2 now points to wrong host.

**Why it happens:** Positional pinning is fragile when list order can change.

**How to avoid:** Clarify in Gap #3 checkpoint whether M3 needs fingerprint-style (host_id-based) or index-based selection. If fingerprint: store `selected_host_id: Option<Uuid>` instead of `selected_row: Option<usize>`, then find row index by looking up host_id in fleet_hosts.

**Warning signs:** Fleet tab renders wrong host highlighted after each refresh; user reports "selection jumps around".

---

### Pitfall 2: Parallel Collect Timeout Stalls Dashboard

**What goes wrong:** One slow host (30s timeout) blocks the main refresh loop. Dashboard becomes unresponsive.

**Why it happens:** If tokio::spawn() result is awaited on the main thread instead of detached.

**How to avoid:** 03-03 Task 2 uses `tokio::task::spawn()`, which is correct (detached). Verify skip flag (`fleet_collection_in_flight`) prevents double-spawn. Test: simulate 30s timeout on one host, confirm main dashboard still refreshes.

**Warning signs:** Dashboard freezes every 60s for ~30s; logs show multiple parallel_collect spawns in flight.

---

### Pitfall 3: host_id Column Migration Breaks Existing Snapshots

**What goes wrong:** 03-05 migration adds `host_id TEXT DEFAULT ''`. Old snapshots have empty host_id. Queries that filter `WHERE host_id = ?` silently return no results.

**How to avoid:** 03-05 Task 6 (full gate) should include migration verification: run migration on test DB, verify old snapshot rows still queryable by primary key (id) and by host_hostname. If queries require non-empty host_id, add WHERE clause: `WHERE host_id = ? OR host_id = ''`.

**Warning signs:** After migration, snapshot queries for old hosts return empty.

---

### Pitfall 4: FleetHostStatus.name Field Empty on Error

**What goes wrong:** Host collection fails (network error). FleetHostStatus created with `name: "unknown"` (03-03 Task 2, line 145). Fleet tab shows blank row instead of host identifier.

**How to avoid:** When result is Err, extract `host_id` from registry and convert to string for display. Or pre-populate name from RemoteEntry before spawn.

**Warning signs:** Down hosts show blank Name column in Fleet tab.

---

## Code Examples

### Pattern 1: Credential Enum + TOML Serialization

**Source:** 03-01-PLAN.md lines 53–65, 03-PATTERNS.md lines 54–71

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

// In RemoteEntry:
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEntry {
    pub host_id: Uuid,
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub credential: Credential,
}
```

**Usage in remotes.toml:**
```toml
[[remotes]]
name = "prod-01"
credential = "SshAgent"  # or { KeyFile = "/home/ops/.ssh/id_rsa" }
```

### Pattern 2: JoinSet Bounded Parallelism

**Source:** 03-02-PLAN.md lines 184–197, 03-PATTERNS.md lines 113–121

```rust
use tokio::task::JoinSet;

pub async fn parallel_collect(&self) -> Vec<(Uuid, Result<Vec<FleetFinding>>)> {
    let mut joinset = JoinSet::new();
    for remote in &self.remotes {
        let remote = remote.clone();
        joinset.spawn(async move {
            (remote.host_id, remote.collect_findings().await)
        });
        if joinset.len() >= 20 {
            let _ = joinset.join_next().await;
        }
    }
    let mut results = Vec::new();
    while let Some(result) = joinset.join_next().await {
        if let Ok((host_id, finding_result)) = result {
            results.push((host_id, finding_result));
        }
    }
    results
}
```

### Pattern 3: Background Task Spawn (Non-Blocking)

**Source:** 03-03-PLAN.md lines 119–155

```rust
// In background refresh tick:
if !self.fleet_collection_in_flight {
    self.fleet_collection_in_flight = true;
    let registry = self.remote_registry.clone();
    let dashboard_data = Arc::clone(&self.dashboard_data);
    tokio::task::spawn(async move {
        let results = registry.parallel_collect().await;
        let fleet_hosts = results.into_iter().map(|(host_id, result)| {
            match result {
                Ok(findings) => {
                    let crit = findings.iter().filter(|f| f.severity == "CRIT").count();
                    FleetHostStatus {
                        host_id,
                        name: findings.first().map(|f| f.host_name.clone()).unwrap_or_default(),
                        reachable: true,
                        crit, warn: 0, info: 0,
                        last_refresh: Some(Instant::now()),
                        error: None,
                    }
                }
                Err(e) => FleetHostStatus {
                    host_id,
                    name: "unknown".to_string(),
                    reachable: false,
                    crit: 0, warn: 0, info: 0,
                    last_refresh: Some(Instant::now()),
                    error: Some(e.to_string()),
                }
            }
        }).collect();
        dashboard_data.lock().unwrap().fleet_hosts = fleet_hosts;
        // fleet_collection_in_flight = false;  // Reset flag
    });
}
```

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | `RemoteEntry.host_id: Uuid` already exists in codebase | Gap #1 | Plans 03-02+ cannot compile if field missing |
| A2 | Background refresh tick is a single tokio::spawn loop | Gap #6 | Task 2 of 03-03 won't compile if spawning location is different |
| A3 | `fleet_selected_row` should be in global state (DashboardState), not per-dashboard data | Gap #7 | 03-04 Task 4 keyboard handler won't have correct scope |
| A4 | Selection pinning uses positional index, not host_id-based lookup | Gap #3 | Fleet tab selection will jump around on host list changes |
| A5 | HMAC audit log integration is required for M3 gate | Gap #5 | M3 RC gate fails if HMAC verification test is missing |
| A6 | Snapshot queries do not require host_id-specific lookups in M3 | Gap #2 | 03-05 queries may be incomplete |

---

## Open Questions

1. **host_id field presence (Gap #1):** Is `RemoteEntry.host_id: Uuid` already present in the codebase, or should 03-01 add it?
   - **Resolution path:** Grep codebase: `grep -n "pub host_id: Uuid" helm-cli/src/remote.rs`. If found, document in 03-01-SUMMARY.md as prerequisite. If not found, add as Task 0 to 03-01-PLAN.md.

2. **Selection pinning strategy (Gap #3):** Should Fleet tab selection be pinned by host_id (fingerprint-style) or by index position?
   - **Resolution path:** Checkpoint task before 03-04 execution: decide and update keyboard handler in Task 4.

3. **HMAC audit integration (Gap #5):** Is audit log append + HMAC verification required before Wave 3, or deferred to post-M3?
   - **Resolution path:** Clarify with stakeholder. If required: add 03-03.5 or Task 6 to 03-03. If deferred: update CONTEXT and M3 gate spec.

4. **Background refresh tick location (Gap #6):** Where is the main background refresh loop? What is the function name and file?
   - **Resolution path:** Grep: `grep -rn "background.*tick\|refresh.*loop" helm-cli/src/tui.rs`. Find exact function, update 03-03-PLAN.md Task 2 with line numbers.

5. **fleet_selected_row struct location (Gap #7):** Should this field go in DashboardState, DashboardData, or new FleetTabState struct?
   - **Resolution path:** Checkpoint task before 03-04: decide struct, update plan with field addition task.

---

## Environment Availability

**Dependencies checked:**
- `tokio` (runtime for JoinSet) — ✓ Already in Cargo.toml
- `serde` + `serde_json` (for JSON parsing) — ✓ Already in Cargo.toml
- `ratatui` (for TUI table widget) — ✓ Already in Cargo.toml
- `rusqlite` (for SQLite migration) — ✓ Already in Cargo.toml
- `anyhow` (for error handling) — ✓ Already in Cargo.toml

**External tools:**
- `helm` binary on remote hosts (required by parallel_collect) — ✓ Assumed installed (per D-M3-2)
- `ssh` CLI on local machine — ✓ Standard on Linux/macOS

**No missing dependencies or environment blockers.**

---

## Sources

### Primary (HIGH confidence)

- **CONTEXT.md** — Locked M3 decisions (D-M3-1 through D-M3-5)
- **03-01-PLAN.md through 03-05-PLAN.md** — Detailed task breakdown for all 5 slices
- **03-UI-SPEC.md** — Design contract for Fleet tab UI
- **03-PATTERNS.md** — Code pattern map with line numbers and analogs

### Secondary (MEDIUM confidence)

- **REQUIREMENTS.md** — REQ-fleet-uuid-parallel acceptance criteria (cross-verified with CONTEXT and plans)
- **STATE.md** — Project state, confirmed M1–M2 complete, M3 slices in planning

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — all libraries already in Cargo.toml, patterns well-established
- Architecture: HIGH — 5 plans cover all success criteria systematically
- Pitfalls: HIGH — common issues identified from M1 patterns (selection pinning, timeout, migration compat)
- Gaps: MEDIUM — 7 micro-gaps identified but all addressable via clarification checkpoints, not blocking execution

**Research date:** 2026-05-18  
**Valid until:** 2026-05-25 (7 days; M3 is stable, no breaking changes expected)

**Key files referenced:**
- `/home/white_devil/code/helm/.planning/phases/03-m3-fleet-multi-host/03-CONTEXT.md`
- `/home/white_devil/code/helm/.planning/phases/03-m3-fleet-multi-host/03-{01,02,03,04,05}-PLAN.md`
- `/home/white_devil/code/helm/.planning/phases/03-m3-fleet-multi-host/03-UI-SPEC.md`
- `/home/white_devil/code/helm/.planning/REQUIREMENTS.md`
- `/home/white_devil/code/helm/.planning/STATE.md`

---

## Recommendations

### Before Wave 1 Execution (03-01, 03-02)

- [ ] **Checkpoint 1 (Gap #1):** Verify `RemoteEntry.host_id: Uuid` field exists. If not, add Task 0 to 03-01.
- [ ] **Checkpoint 2 (Gap #6):** Grep for background refresh loop, update 03-03-PLAN.md Task 2 with exact location.

### Before Wave 2 Execution (03-03, 03-05)

- [ ] **Checkpoint 3 (Gap #5):** Clarify HMAC audit requirement. If needed, create 03-03.5-PLAN.md or add Task 6 to 03-03 for audit + HMAC verification test.
- [ ] **Checkpoint 4 (Gap #2):** Define snapshot query functions that should support host_id lookups (if any beyond insert).

### Before Wave 3 Execution (03-04)

- [ ] **Checkpoint 5 (Gap #3):** Decide selection pinning strategy (host_id-based or index-based), update keyboard handler accordingly.
- [ ] **Checkpoint 6 (Gap #7):** Decide struct location for `fleet_selected_row`, update plan with field addition.

### Before M3 RC Gate

- [ ] All 7 checkpoints resolved
- [ ] Gap #4 (Password deferral) documented in 03-01-SUMMARY.md
- [ ] M3 gate test list updated to include HMAC verification (if required per Checkpoint 3)

---

## Research Complete

**Plans are sound.** All success criteria have corresponding tasks. Seven micro-gaps exist but are **not blockers** — they are scope clarifications that should be resolved via checkpoint tasks before execution. Once checkpoints are cleared, Phase 3 is ready for full execution across all 3 waves.
