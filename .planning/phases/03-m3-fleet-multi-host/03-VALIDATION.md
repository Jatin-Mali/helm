---
phase: 03
slug: m3-fleet-multi-host
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-05-18
---

# Phase 03 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | cargo test (Rust built-in) |
| **Config file** | Cargo.toml (workspace) |
| **Quick run command** | `cargo test --workspace --all-targets 2>&1 \| tail -20` |
| **Full suite command** | `cargo test --workspace --all-targets && cargo clippy --workspace --all-targets -- -D warnings` |
| **Estimated runtime** | ~30 seconds |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --workspace --all-targets 2>&1 | tail -20`
- **After every plan wave:** Run full suite + clippy
- **Before `/gsd:verify-work`:** Full suite must be green
- **Max feedback latency:** 30 seconds

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|--------|
| 03-01-01 | 01 | 1 | REQ-fleet-uuid-parallel | — | No plaintext password in RemoteEntry | unit | `cargo test -p helm-cli credential` | ⬜ pending |
| 03-02-01 | 02 | 1 | REQ-fleet-uuid-parallel | — | JoinSet bounded to 20 concurrent tasks | unit | `cargo test -p helm-cli parallel_collect` | ⬜ pending |
| 03-02-02 | 02 | 1 | REQ-fleet-uuid-parallel | — | 20-host benchmark ≤2s p99 | bench | `cargo test -p helm-cli bench -- --ignored` | ⬜ pending |
| 03-03-01 | 03 | 2 | REQ-fleet-uuid-parallel | — | fleet_hosts updated on tick | unit | `cargo test -p helm-cli fleet_dashboard` | ⬜ pending |
| 03-03-02 | 03 | 2 | REQ-fleet-uuid-parallel | — | HMAC chain appended after fleet episode | unit | `cargo test -p helm-audit hmac_fleet` | ⬜ pending |
| 03-04-01 | 04 | 3 | REQ-fleet-uuid-parallel | — | Fleet tab renders in terminal | snapshot | `cargo test -p helm-cli fleet_tab_snapshot` | ⬜ pending |
| 03-05-01 | 05 | 2 | REQ-fleet-uuid-parallel | — | host_id stored and queryable | unit | `cargo test -p helm-memory host_id` | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Wave 0 Requirements

- Existing cargo test infrastructure covers all phase requirements — no new test harness needed.

*Existing infrastructure covers all phase requirements.*

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| Fleet of 12 hosts shows "fleet: 12/12 up" in TUI header | REQ-fleet-uuid-parallel | Requires live SSH hosts or mock server | Run `helm tui` with 12 remotes configured; observe header |
| 20-host parallel refresh ≤2s wall-clock | REQ-fleet-uuid-parallel | Depends on network/SSH timing | Run `helm fleet --benchmark` against 20 mock SSH targets |
| Selection sticky after background fleet refresh | REQ-fleet-uuid-parallel | TUI interaction hard to automate | Navigate to fleet tab, trigger refresh, verify row stays highlighted |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 30s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
