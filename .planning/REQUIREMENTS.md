# HELMOPS v1 — Requirements (M1–M6)

**Version:** 1.0  
**Status:** M1–M2 complete, M3–M6 in planning  
**Total Requirements:** 25  

---

## M1 — UX Foundation + Agent Strip

### REQ-ux-dashboard-5sec
**Priority:** Critical  
**Milestone:** M1  
**Description:** Operator opens HELMOPS dashboard and sees the top problem in ≤5 seconds with clear visual hierarchy.

**Acceptance Criteria:**
- Dashboard loads in <500ms
- Top CRIT finding visible without scrolling
- Severity-grouped left pane with CRIT/WARN/INFO blocks
- Right pane shows WHEN/WHERE/WHAT/WHY/IMPACT/FIX PLAN in exact order
- Selection remains sticky across live refresh (fingerprint-pinned)

**Verified:** ✓ M1 Complete

---

### REQ-tui-selection-sticky
**Priority:** Critical  
**Milestone:** M1  
**Description:** Selected finding remains visible and selected across live refresh cycles.

**Acceptance Criteria:**
- Replace `selected_finding: usize` with `selected_fingerprint: Option<u64>`
- Capture fingerprint pre-refresh in `refresh_dashboard`
- Restore index post-refresh by fingerprint lookup
- Fall back to clamp if finding is gone
- Unit test: build Vec [A,B,C], select B, replace with [A,C,B,D], assert selection still on B

**Verified:** ✓ M1 Complete

---

### REQ-tui-why-pane-dedup
**Priority:** High  
**Milestone:** M1  
**Description:** WHY pane collapses duplicate correlated findings into single entries with count.

**Acceptance Criteria:**
- Group correlated findings by title via `HashMap<String, (Severity, usize)>`
- Render `● {sev} {title} (×{count})` per group
- Snapshot test: 17 correlations render as single `(×17)` line
- Manual inspection on nginx-5xx scenario confirms single line

**Verified:** ✓ M1 Complete

---

### REQ-tui-tabs-5-layout
**Priority:** Critical  
**Milestone:** M1  
**Description:** TUI collapses to 5 primary tabs with severity-grouped left pane and sparklines.

**Acceptance Criteria:**
- Reduce `OpsTab` enum to: Alerts, Services, Resources, Logs, Changes
- Move Processes/Network/Storage under Resources via sub-section toggle
- Single-line status bar: host, fleet count, uptime, clock, tab list, hotkeys
- Severity-group left pane: CRIT block, WARN block, INFO block
- Add sparklines beneath severity groups (cpu/mem/disk/net/load)
- Snapshot test at 120×40 terminal size
- Visual diff against roadmap mockup

**Verified:** ✓ M1 Complete

---

### REQ-agent-strip
**Priority:** Critical  
**Milestone:** M1  
**Description:** Remove ReAct agent, chat modes, and associated infrastructure; reduce binary size by ≥30%.

**Acceptance Criteria:**
- Delete: helm-cli/src/{serve,agent_remote,hooks,ndjson_sink,snapshot_sink}.rs
- Delete: crates/helm-agent/ entire crate
- Remove from workspace Cargo.toml
- Remove TuiMode::{Chat,Plan,Diagnose,Auto}
- Remove CLI commands: episodes, skills, sessions, mcp, replay, memory, profile, undo, redo, export, stats, serve, chat, plan, diagnose, auto
- Default subcommand → `watch`
- Workspace builds clean
- Binary size drop ≥30% pre/post
- apply-plan integration test green (verify execute.rs uses only std::process)

**Verified:** ✓ M1 Complete

---

### REQ-helm-watch-default
**Priority:** Critical  
**Milestone:** M1  
**Description:** `helm watch` is the default zero-arg entry point; `helm` launches directly to dashboard.

**Acceptance Criteria:**
- Invoking `helm` with no arguments → `helm watch`
- No argument parsing or mode selection
- Opens dashboard TUI in <500ms

**Verified:** ✓ M1 Complete

---

### REQ-secrets-policy
**Priority:** Critical  
**Milestone:** M1  
**Description:** API key storage with strict file permissions and no silent env import.

**Acceptance Criteria:**
- Storage: $XDG_CONFIG_HOME/helm/secrets.toml (or ~/.config/helm/secrets.toml)
- File mode: 0600 (user only)
- Parent directory mode: 0700
- Atomic write: temp-file + rename
- Never world-readable; refuse load if wider than 0600
- Keys in Secret newtype; suppress debug output
- Resolution order: CLI flag → secrets store → env var
- No silent env auto-import by TUI
- Security boundary: 0600 file mode + 0700 parent

**Verified:** ✓ M1 Complete

---

### REQ-audit-hmac-chain
**Priority:** Critical  
**Milestone:** M1  
**Description:** HMAC-chained append-only audit log that verifies after every episode.

**Acceptance Criteria:**
- Hash chain: each record includes hash of previous record
- Append-only: no modification or deletion
- Verify called after every episode
- Verify must succeed for episode to be considered complete
- Cross-cutting invariant test: HMAC audit chain verifies after milestone's representative episode

**Verified:** ✓ M1 Complete

---

### REQ-capability-gate
**Priority:** Critical  
**Milestone:** M1  
**Description:** Capability gate checked before every tool call; `*.write` blocked on external-tainted input.

**Acceptance Criteria:**
- Every tool call checks capability before dispatch
- Taint level checked for privileged operations
- External-tainted requires fresh approval for `*.write` ops
- Regression test: external-tainted cannot execute destructive ops
- Read-only diagnose mode blocks all write tools

**Verified:** ✓ M1 Complete

---

### REQ-taint-propagation
**Priority:** Critical  
**Milestone:** M1  
**Description:** Taint level propagated through Tainted<T> newtype; external content tagged TaintLevel::External.

**Acceptance Criteria:**
- Tainted<T> newtype enforces taint semantics
- Browser, SSH, MCP output marked TaintLevel::External
- Taint level checked before privileged operations
- Never strip taint without explicit user action
- Cross-cutting invariant test included

**Verified:** ✓ M1 Complete

---

## M2 — Real-Time Trends

### REQ-sparkline-history
**Priority:** High  
**Milestone:** M2  
**Description:** Ring-buffer history per metric for sparkline rendering.

**Acceptance Criteria:**
- Add VecDeque<f64> per metric: cpu, mem, disk, net, loadavg
- Cap 60 points per buffer
- Push on each refresh
- Render beneath severity groups in left pane
- Render per-tab sparklines in Resources tab
- Performance budget ≤4ms render time at 60 findings + 5 sparklines
- Configurable refresh cadence + history depth via thresholds.toml

**Verified:** ✓ M2 Complete

---

## M3 — Fleet (Multi-Host)

### REQ-fleet-uuid-parallel
**Priority:** High  
**Milestone:** M3  
**Description:** Multi-host fleet support via parallel SSH with credential abstraction.

**Acceptance Criteria:**
- Extend RemoteEntry with host_id: Uuid
- Parallel SSH JoinSet in RemoteRegistry
- Collect findings from N hosts concurrently
- Benchmark at N=20, complete refresh in ≤2s
- Credential abstraction: enum Credential { SshAgent, KeyFile(PathBuf), Password(Secret) }
- No passwords in registry file
- Fleet panel shows `fleet: 12/12 up` in header
- Fleet tab (tab 6) with per-host CRIT counts
- HMAC chain verifies across multi-host episode

**Status:** Pending

---

## M4 — Kubernetes / VM / Compose

### REQ-kubernetes-collector
**Priority:** High  
**Milestone:** M4  
**Description:** Kubernetes collector via kubectl wrapper to surface pod events, restarts, OOMKills, and PVC pressure.

**Acceptance Criteria:**
- New file: crates/helm-monitor/src/collectors/kubernetes.rs
- Collects: events (Warning), pod restarts, OOMKills, PVC pressure
- Capability-gated: KubectlRead
- Integration test against kind cluster in CI matrix
- Detector tuning for new finding types

**Status:** Pending

---

### REQ-libvirt-collector
**Priority:** Medium  
**Milestone:** M4  
**Description:** libvirt/QEMU collector via virsh wrapper to surface domain state, snapshot age, and host load.

**Acceptance Criteria:**
- New file: crates/helm-monitor/src/collectors/libvirt.rs
- Collects: domain state, snapshot age, host load
- Integration test against libvirt test VM in CI matrix

**Status:** Pending

---

### REQ-compose-collector
**Priority:** Medium  
**Milestone:** M4  
**Description:** Docker Compose collector layered over existing Docker collector.

**Acceptance Criteria:**
- Groups containers by project label
- Integration test against compose fixture in CI matrix
- Detector tuning for new finding types

**Status:** Pending

---

## M5 — Alerting

### REQ-alerting-routing
**Priority:** High  
**Milestone:** M5  
**Description:** Alert routing module with severity threshold, dedup window, and rate limiting.

**Acceptance Criteria:**
- New module: crates/helm-monitor/src/alerting/mod.rs
- Severity threshold (gate CRIT/WARN/INFO separately)
- Dedup window per fingerprint
- Rate limiting per sink
- Three sinks: webhook, Slack, PagerDuty

**Status:** Pending

---

### REQ-webhook-sink
**Priority:** High  
**Milestone:** M5  
**Description:** Webhook alert sink with HTTP POST, JSON body, and retry backoff.

**Acceptance Criteria:**
- HTTP POST to configurable URL
- JSON body with finding details
- Retry with exponential backoff
- Configurable timeout

**Status:** Pending

---

### REQ-slack-sink
**Priority:** High  
**Milestone:** M5  
**Description:** Slack alert sink with incoming webhook URL, channel override, and severity color.

**Acceptance Criteria:**
- Slack incoming webhook integration
- Channel override per alert route
- Severity color coding (red=CRIT, yellow=WARN, blue=INFO)
- Formatted message with finding details

**Status:** Pending

---

### REQ-pagerduty-sink
**Priority:** High  
**Milestone:** M5  
**Description:** PagerDuty alert sink using Events API v2 with fingerprint-based dedup.

**Acceptance Criteria:**
- PagerDuty Events API v2 integration
- dedup_key = finding fingerprint
- Severity mapping (CRIT→critical, WARN→warning)
- Resolution event on finding lifecycle close
- End-to-end: trigger CRIT finding in mock → PD event fires within 5s → resolution event fires on lifecycle close

**Status:** Pending

---

## M6 — AI Excellence

### REQ-auto-plan-background
**Priority:** High  
**Milestone:** M6  
**Description:** Auto-plan generation via background LLM call with fingerprint-keyed cache.

**Acceptance Criteria:**
- Spawn background tokio::task on finding selection (fingerprint change)
- Cache result by fingerprint in Arc<DashMap<Fingerprint, PlanState>>
- Instant render on cache hit
- "generating…" spinner while waiting
- Cache hit ratio ≥80% on second open of same finding
- Plans capability-gated and taint-checked
- 24h trace test validates cache TTL

**Status:** Pending

---

### REQ-auto-plan-apply
**Priority:** High  
**Milestone:** M6  
**Description:** `[a]apply` hotkey wires to execute.rs apply-plan path with capability gate, taint check, and audit append.

**Acceptance Criteria:**
- Hotkey `[a]` bound to apply flow
- Routes to crates/helm-monitor/src/execute.rs
- Capability gate checked before execution
- Taint level verified (external-tainted cannot execute destructive ops)
- Audit log appends with full command trace
- HMAC chain verifies after apply episode
- Integration test: select finding → apply plan → audit log appends → HMAC verifies

**Status:** Pending

---

## Traceability

| Requirement | Milestone | Status |
|-------------|-----------|--------|
| REQ-ux-dashboard-5sec | M1 | Complete ✓ |
| REQ-tui-selection-sticky | M1 | Complete ✓ |
| REQ-tui-why-pane-dedup | M1 | Complete ✓ |
| REQ-tui-tabs-5-layout | M1 | Complete ✓ |
| REQ-agent-strip | M1 | Complete ✓ |
| REQ-helm-watch-default | M1 | Complete ✓ |
| REQ-secrets-policy | M1 | Complete ✓ |
| REQ-audit-hmac-chain | M1 | Complete ✓ |
| REQ-capability-gate | M1 | Complete ✓ |
| REQ-taint-propagation | M1 | Complete ✓ |
| REQ-sparkline-history | M2 | Complete ✓ |
| REQ-fleet-uuid-parallel | M3 | Pending |
| REQ-kubernetes-collector | M4 | Pending |
| REQ-libvirt-collector | M4 | Pending |
| REQ-compose-collector | M4 | Pending |
| REQ-alerting-routing | M5 | Pending |
| REQ-webhook-sink | M5 | Pending |
| REQ-slack-sink | M5 | Pending |
| REQ-pagerduty-sink | M5 | Pending |
| REQ-auto-plan-background | M6 | Pending |
| REQ-auto-plan-apply | M6 | Pending |

**Total Mapped:** 21/21 requirements  
**Coverage:** 100% ✓
