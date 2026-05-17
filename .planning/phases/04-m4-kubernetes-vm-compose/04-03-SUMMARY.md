---
phase: 04-m4-kubernetes-vm-compose
plan: 03
title: Docker Compose Collector Implementation
status: completed
date: 2026-05-18
duration: 15m
commits:
  - hash: 76fb605
    message: "feat(04-m4-kubernetes-vm-compose): implement docker compose collector with project grouping"
---

# Phase 04 Plan 03: Docker Compose Collector Summary

Docker Compose collector implementation complete. Groups containers by `com.docker.compose.project` label and provides project-level health aggregates.

## Implementation Status

**COMPLETED** — All 5 tasks executed, all 6 unit tests pass, workspace clean.

## Key Functions

### ComposeCollector Struct
```rust
#[derive(Default)]
pub struct ComposeCollector;

impl Collector for ComposeCollector {
    type Output = ComposeSnapshot;
    fn domain(&self) -> &'static str { "compose" }
    async fn collect(self, profile: MonitorProfile) -> Result<Self::Output, CollectorError>
}
```

### ComposeSnapshot
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ComposeSnapshot {
    pub projects: Vec<ComposeProject>,
    pub total_container_count: usize,
    pub available: bool,
}
```

### ComposeProject
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeProject {
    pub name: String,
    pub status: String,           // "healthy", "degraded", "down"
    pub container_count: u32,
    pub running_count: u32,
    pub unhealthy_count: u32,
}
```

### parse_compose_projects()
- Parses `docker ps` JSON output line-by-line
- Groups containers by `com.docker.compose.project` label
- Calculates per-project aggregates (running count, unhealthy count)
- Derives status: "healthy" (all running + all healthy), "degraded" (some running), "down" (none running)
- Returns Vec<ComposeProject> sorted by name

## Files Created/Modified

| File | Action | Purpose |
|------|--------|---------|
| `crates/helm-monitor/src/collectors/compose.rs` | Created | Docker Compose collector implementation (250 LOC) |
| `crates/helm-monitor/src/collectors/mod.rs` | Modified | Added `pub mod compose;` registration (alphabetically) |

## Task Execution

| Task | Name | Status | Details |
|------|------|--------|---------|
| 1 | Define ComposeSnapshot and ComposeProject structs | ✓ | Structs defined with serde derives |
| 2 | Implement ComposeCollector with trait and docker integration | ✓ | Collector impl, bin_exists check, docker ps integration |
| 3 | Implement docker-compose project parser and grouping logic | ✓ | parse_compose_projects() with JSON parsing and label grouping |
| 4 | Register ComposeCollector in mod.rs | ✓ | Module registered and exported |
| 5 | Unit tests for ComposeCollector | ✓ | 6 tests: empty, single/multiple projects, status logic, missing label, all stopped |

## Verification Results

- Compilation: PASS (no errors or warnings)
- Unit tests: **6/6 PASS** (`cargo test -p helm-monitor compose::`)
- Formatting: PASS (`cargo fmt --check`)
- Linting: PASS (`cargo clippy -p helm-monitor --lib -- -D warnings`)
- Full workspace tests: **575/575 PASS** (1 ignored)

## Design Notes

**Docker Integration:**
- Command: `docker ps -a --filter "label=com.docker.compose.project" --format "{{json .}}"`
- Uses JSON format for reliable parsing over CSV
- Graceful degradation: returns default with `available=false` if docker not found

**Project Grouping:**
- Containers extracted from docker ps with Labels field
- Labels parsed as comma-separated key=value pairs
- Project name extracted from `com.docker.compose.project` label
- Containers without the label are skipped

**Status Calculation:**
- "healthy": All containers running AND unhealthy_count == 0
- "degraded": Some containers running (but not all or some unhealthy)
- "down": No containers running

**Default Derivation:**
- ComposeSnapshot derives Default (required for `unwrap_or_default()` in engine.rs)
- Returns `{ projects: [], total_container_count: 0, available: false }` when docker unavailable

## Deviations from Plan

None — plan executed exactly as written.

## Next Plan Dependency

Next plan: **04-04** — integrates ComposeCollector into engine, adds to registry, wires into dashboard.

## Threat Compliance

✓ **T-04-06** (Command Injection): Fixed args array, no shell interpolation
✓ **T-04-07** (Information Disclosure): User-controlled labels accepted per threat model
