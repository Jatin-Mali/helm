# Phase 4: M4 — Kubernetes / VM / Compose — Context & Decisions

**Phase Goal:** Cover the real infrastructure surface by adding three new infrastructure collectors (Kubernetes, libvirt, Docker Compose) with capability gating, integration tests, and finding detectors.

**Roadmap Requirements:**
- REQ-kubernetes-collector
- REQ-libvirt-collector
- REQ-compose-collector

**Phase Scope:**
1. Kubernetes collector: kubectl wrapper exposing pod events, restarts, OOMKills, PVC pressure with KubectlRead capability gate
2. libvirt/QEMU collector: virsh wrapper exposing domain state, snapshot age, host load
3. Docker Compose collector: project-level aggregation over Docker data, grouped by com.docker.compose.project label
4. Engine integration: wire all three collectors into engine.rs + SnapshotDomains
5. Finding detectors: pod_restart, oom_kill, pvc_pressure, domain_state, snapshot_age, compose_health
6. Thresholds: configurable via ~/.config/helm/thresholds.toml

---

## Key Design Decisions

### D-01: Capability Gate for Kubernetes
**Decision:** KubectlRead capability gate blocks kubectl operations when not authorized.
**Rationale:** kubectl access can expose cluster secrets and workload details; gating is security-critical.
**Implementation:** Check capability in KubernetesCollector::collect() before calling run_timed("kubectl", ...).
**Status:** Locked

### D-02: Graceful Degradation on Missing Binaries
**Decision:** All three collectors return default (available=false) if their binary is missing, rather than failing.
**Rationale:** Many deployments don't have all three tools installed; collectors should be optional.
**Implementation:** Check bin_exists() first; return Ok(Snapshot::default()) if missing.
**Status:** Locked

### D-03: Multi-Step Collection Pattern for libvirt
**Decision:** libvirt collector uses multi-step pattern (primary domstats + optional snapshot enrichment).
**Rationale:** Some virsh commands may fail on restricted hosts; secondary commands should not fail the entire collector.
**Implementation:** Use if let Ok(...) for optional snapshot commands; only fail on primary command errors.
**Status:** Locked

### D-04: Compose Grouping by Project Label
**Decision:** Compose collector groups containers by com.docker.compose.project label (not by directory).
**Rationale:** Same project may run from different directories or machines; label is the canonical identifier.
**Implementation:** Filter docker ps output by label, group by label value, calculate per-project aggregates.
**Status:** Locked

### D-05: Detector Severity Mapping
**Decision:** OOMKill always CRIT; pod restarts WARN (>10) / INFO (≤10); domain down CRIT / paused WARN; snapshot age WARN; compose down CRIT / degraded WARN.
**Rationale:** Escalate severity for conditions requiring immediate action (OOMKill, VM down, Compose down); info for informational state.
**Implementation:** Hardcoded in detector detect() methods; can be overridden via thresholds.toml if needed.
**Status:** Locked

### D-06: Thresholds Configuration Location
**Decision:** thresholds.toml at ~/.config/helm/thresholds.toml with TOML format.
**Rationale:** Consistent with existing threshold configuration; user-modifiable without code changes.
**Implementation:** Load at engine startup; detectors query via profile.get_threshold(...) calls.
**Status:** Locked

### D-07: Wave Structure
**Decision:** Plans 04-01, 04-02, 04-03 (collectors) run in parallel (Wave 1). Plans 04-04 (engine integration) and 04-05 (detectors) depend on all three collectors (Wave 2).
**Rationale:** Collectors are independent; engine and detectors require all collectors to be defined.
**Implementation:** Set depends_on=[04-01, 04-02, 04-03] for 04-04 and 04-05.
**Status:** Locked

---

## Cross-Cutting Constraints

### Security Invariants (Per CLAUDE.md)
1. **Capability gate:** KubectlRead checked before kubectl execution
2. **Command injection:** All commands use fixed args arrays (no shell interpolation)
3. **Graceful degradation:** Missing tools don't cause failures

### Testing Requirements
- **Unit tests:** Each collector and detector has unit tests with mocked/default data
- **Integration test:** engine.rs has test_collect_snapshot that verifies all three collectors are called
- **Workspace gate:** Every commit must pass `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets`
- **RC gate (post-phase):** `cargo test deterministic_100_run -- --ignored` must pass

### File Patterns
- **Collectors:** Follow `crates/helm-monitor/src/collectors/containers.rs` pattern (struct + Collector impl)
- **Snapshots:** Follow `snapshot.rs` line 231–264 pattern (Serialize/Deserialize derives)
- **Detectors:** Follow `detectors/container_restart.rs` pattern (struct + Detector impl)
- **Error handling:** Use `err(domain, message)` helper from `collectors/mod.rs` line 91

### No Out-of-Scope Work
- ❌ UI changes (dashboard display of K8s/VM/Compose data — deferred to M5+)
- ❌ Alerting integration (finding routing — deferred to M5)
- ❌ Multi-host (fleet) handling (agent/fleet coordination — belongs to M3)
- ❌ Auto-plan generation (finding → fix — deferred to M6)

---

## Dependency Map

```
Phase 3 (M3 — Fleet) [completed]
  ↓
Phase 4 (M4 — K8s / VM / Compose)
  ├─ 04-01: Kubernetes collector (Wave 1)
  ├─ 04-02: libvirt collector (Wave 1)
  ├─ 04-03: Compose collector (Wave 1)
  ├─ 04-04: Engine integration (Wave 2, depends on 01-03)
  └─ 04-05: Detectors + thresholds (Wave 2, depends on 01-03)
  ↓
Phase 5 (M5 — Alerting) [pending]
```

---

## Success Criteria Checklist

- [ ] All three collectors implement Collector trait correctly
- [ ] KubectlRead capability gate present and functional
- [ ] All collectors return gracefully if binary missing
- [ ] SnapshotDomains includes kubernetes, libvirt, compose fields
- [ ] engine.rs calls all three collectors in tokio::join!()
- [ ] Six detectors (pod_restart, oom_kill, pvc_pressure, domain_state, snapshot_age, compose_health) registered
- [ ] thresholds.toml configured with K8s/VM/Compose sections
- [ ] All unit tests pass
- [ ] Full workspace gate passes (fmt + clippy + test)
- [ ] Integration test confirms engine collects from all three domains

---

## Open Questions / Not Yet Decided

None — all critical design decisions locked above.

---

## Plan Summaries (to be filled in as each plan executes)

### 04-01: Kubernetes Collector
*Status: Pending*

### 04-02: libvirt Collector
*Status: Pending*

### 04-03: Docker Compose Collector
*Status: Pending*

### 04-04: Engine Integration
*Status: Pending*

### 04-05: Detectors + Thresholds
*Status: Pending*

---

**Phase Created:** 2026-05-18
**Last Updated:** 2026-05-18
