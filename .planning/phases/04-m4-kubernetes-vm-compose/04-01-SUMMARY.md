---
phase: 04-m4-kubernetes-vm-compose
plan: 01
title: Kubernetes Collector Implementation
implementation_status: complete
date_completed: 2026-05-18
duration_minutes: 15
subsystem: helm-monitor
tags: [kubernetes, collector, kubectl, monitoring]
---

# Phase 04 Plan 01: Kubernetes Collector Implementation Summary

## One-Liner
Kubernetes collector wrapping kubectl with pod events, restarts, OOMKills, PVC pressure detection, and capability-gated access.

## Implementation Status
**COMPLETE** — All 5 tasks executed, all tests passing (6/6), full workspace verification successful.

## Files Created/Modified

| File | Changes |
|------|---------|
| `crates/helm-monitor/src/collectors/kubernetes.rs` | NEW (193 lines) — Complete implementation |
| `crates/helm-monitor/src/collectors/mod.rs` | MODIFIED (+1 line) — Module registration |
| `crates/helm-core/src/capability.rs` | MODIFIED (+4 locations) — KubectlRead capability |

## Key Functions & Signatures

### Structs
```rust
pub struct KubernetesSnapshot {
    pub pods: Vec<PodInfo>,
    pub namespace_count: usize,
    pub available: bool,
}

pub struct PodInfo {
    pub namespace: String,
    pub name: String,
    pub status: String,
    pub ready: String,
    pub restarts: u32,
    pub age: String,
    pub events_count: u32,
    pub oom_kill_count: u32,
    pub pvc_pressure: bool,
}

#[derive(Default)]
pub struct KubernetesCollector;
```

### Trait Implementation
```rust
impl Collector for KubernetesCollector {
    type Output = KubernetesSnapshot;
    fn domain(&self) -> &'static str { "kubernetes" }
    async fn collect(self, profile: MonitorProfile) -> Result<Self::Output, CollectorError>
}
```

### Helpers
```rust
fn parse_kubectl(s: &str) -> Vec<PodInfo>
```

## Task Completion Summary

| Task | Name | Status | Notes |
|------|------|--------|-------|
| 1 | Define KubernetesSnapshot and PodInfo structs | ✓ DONE | Derives Default, Serialize, Deserialize |
| 2 | Implement KubernetesCollector trait | ✓ DONE | bin_exists check, run_timed call, error handling |
| 3 | Implement kubectl output parser | ✓ DONE | Splits on whitespace, extracts 6+ fields, detects OOMKilled/PVC |
| 4 | Register in mod.rs + KubectlRead capability | ✓ DONE | Added to enum, as_str(), all(), FromStr |
| 5 | Unit tests | ✓ DONE | 6 tests covering empty, single pod, multi-namespace, OOMKilled |

## Capability Gate

**KubectlRead** capability added to `crates/helm-core/src/capability.rs`:
- Variant: `KubectlRead`
- String key: `"kubectl.read"`
- Location: Between `NetworkOut` and `Sudo` in enum
- Integrated in: `as_str()`, `all()`, `FromStr` impl

**Binary-level gate**: If kubectl is missing (`bin_exists("kubectl")` returns false), collector returns gracefully with `available: false` and empty pod list.

## Testing Results

```
cargo test -p helm-monitor kubernetes:: --lib
  ✓ parse_kubectl_empty
  ✓ parse_kubectl_header_only
  ✓ parse_kubectl_single_pod
  ✓ parse_kubectl_multiple_namespaces
  ✓ parse_kubectl_oom_killed
  ✓ test_kubernetes_snapshot_default
  Result: 6 passed
```

Full workspace test: **575 passed, 1 ignored** (11 suites, 11.23s)

## Verification Gates

| Gate | Result |
|------|--------|
| `cargo fmt --check` | ✓ PASS |
| `cargo clippy -p helm-monitor -- -D warnings` | ✓ PASS |
| `cargo test --workspace --all-targets` | ✓ PASS (575 passed) |
| Per-task: `cargo build --lib -p helm-monitor` | ✓ PASS after each task |

## Deviations from Plan
None — plan executed exactly as written.

## Next Steps
Plan 04-02 depends on this collector for integration into the monitor engine. KubernetesSnapshot::default() is ready for unwrap_or_default() calls in engine.rs.

## Commit
- Hash: `4b6db21`
- Message: `feat(04-01): implement Kubernetes collector with kubectl integration`
