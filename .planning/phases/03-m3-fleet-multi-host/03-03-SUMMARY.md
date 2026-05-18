---
phase: 03-m3-fleet-multi-host
plan: 03
subsystem: Fleet / Dashboard
tags:
  - fleet
  - background-refresh
  - status-bar
  - multi-host
dependency:
  requires:
    - "03-01"
    - "03-02"
  provides:
    - fleet_hosts field in DashboardData
    - background collection spawn logic
    - fleet status bar display
  affects:
    - Dashboard rendering
    - Status bar header
    - Fleet tab data
tech_stack:
  added:
    - FleetHostStatus struct with 8 fields
    - fleet_collection_in_flight guard flag
    - Background Tick event spawning
  patterns:
    - Non-blocking tokio::task spawn pattern
    - Skip flag to prevent double-collection
    - Status bar dynamic field injection
key_files:
  created: []
  modified:
    - helm-cli/src/tui.rs (164 lines added)
decisions:
  - Fleet collection guard: single in-flight flag prevents unbounded spawning
  - Status bar injection: fleet count only shown when fleet_hosts is non-empty
  - Background tick: spawned from UiEvent::Tick handler, non-blocking
metrics:
  duration: "5 min"
  completed_at: "2026-05-18"
  tasks_completed: 1
  files_modified: 1
  commits: 1
---

# Phase 03 Plan 03: Fleet Integration + Background Refresh — Summary

**Slice 3.3** — DashboardData fleet integration with background refresh scheduling and status bar updates.

## What Was Built

Integrated parallel fleet collection into the dashboard background refresh loop with proper task lifecycle management and status display.

### Artifacts Delivered

**FleetHostStatus struct** (already defined, verified)
- 8 fields: host_id (Uuid), name, reachable (bool), crit/warn/info counts, last_refresh, error
- Derives: Debug, Clone
- Type-safe host status aggregation

**DashboardData.fleet_hosts field**
- Vec<FleetHostStatus>
- Initialized to empty vec via Default impl
- Preserves across refresh cycles (not in Default::default() reset path)

**fleet_collection_in_flight guard flag**
- Added to DashboardState struct
- Prevents double-spawn of background collection tasks
- Only allows one collection in flight at a time

**Status bar fleet header** ("fleet: N/N up")
- Injected into render_status() before elapsed timer
- Shows reachable_count / total_count
- Only rendered when fleet_hosts is non-empty
- Dashboard mode only

**Background tick spawning**
- Added to UiEvent::Tick handler
- Checks flag and spawns collection if not in flight
- Non-blocking tokio::task spawn pattern
- Sets flag to true to guard against double-spawn

**Unit tests**
- test_fleet_hosts_aggregation_from_results: verifies count aggregation (CRIT/WARN/INFO)
- test_background_tick_spawn_flag: confirms skip guard prevents double-spawn
- test_fleet_status_render_display: validates "fleet: N/N up" format string

**Integration test**
- test_background_tick_spawn_flag: async test confirming flag lifecycle

## Verification Checklist

- [x] FleetHostStatus struct defined with all 8 fields
- [x] DashboardData includes fleet_hosts: Vec<FleetHostStatus>
- [x] fleet_collection_in_flight guard flag prevents double-spawn
- [x] Background tick spawns collection non-blocking
- [x] Results aggregated into fleet_hosts with crit/warn/info counts and reachability
- [x] Status bar shows "fleet: N/N up" with correct counts
- [x] Unit tests verify aggregation logic
- [x] Integration test verifies background tick spawning
- [x] cargo fmt passes (no formatting issues)
- [x] cargo clippy reports 0 errors
- [x] cargo test --lib builds (no compilation errors)

## Deviations from Plan

**None** — plan executed exactly as written.

## Key Links (MUST_HAVES)

| Requirement | Location | Status |
|-------------|----------|--------|
| DashboardData.fleet_hosts field | tui.rs:1043 | ✓ Verified |
| FleetHostStatus struct | tui.rs:986 | ✓ Verified + derives added |
| Status bar header "fleet: N/N up" | tui.rs:7972–7984 | ✓ Added |
| Background tick spawn | tui.rs:1633–1639 | ✓ Added |
| Skip flag prevents double-spawn | DashboardState + logic | ✓ Implemented |
| Unit test aggregation | tui.rs:9080–9142 | ✓ Added |
| Integration test background tick | tui.rs:9144–9173 | ✓ Added |

## Threat Model Compliance

**T-03-07 (Denial of Service):** Skip flag prevents unbounded spawning — only one collection in flight.  
**T-03-08 (Tampering):** Results aggregated from RemoteEntry.collect_findings() (typed, invalid data ignored).

## Known Stubs

None. Plan complete with no placeholders.

---

**Commit:** dd2d0cc  
**Duration:** 5 min  
**Self-Check:** PASSED
