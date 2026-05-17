# HELMOPS Roadmap — Milestones M1–M6

**Status:** M1–M2 Complete | M3–M6 Pending

---

## Phases

- [x] **Phase 1: M1 — UX Foundation + Agent Strip** - Fix selection jump, collapse tabs, deduplicate WHY pane, strip ReAct agent
- [x] **Phase 2: M2 — Real-Time Trends** - Ring-buffer metrics, sparkline rendering
- [ ] **Phase 3: M3 — Fleet (Multi-Host)** - Parallel SSH, credential abstraction, fleet panel
- [ ] **Phase 4: M4 — Kubernetes / VM / Compose** - K8s, libvirt, Compose collectors
- [ ] **Phase 5: M5 — Alerting** - Webhook, Slack, PagerDuty sinks
- [ ] **Phase 6: M6 — AI Excellence** - Auto-plan generation, fingerprint caching, apply hotkey

---

## Phase Details

### Phase 1: M1 — UX Foundation + Agent Strip

**Goal:** Operator opens HELMOPS, sees grouped findings in severity hierarchy, selects one without jump, and binary is half the size.

**Depends on:** None (foundation)

**Requirements:** REQ-ux-dashboard-5sec, REQ-tui-selection-sticky, REQ-tui-why-pane-dedup, REQ-tui-tabs-5-layout, REQ-agent-strip, REQ-helm-watch-default, REQ-secrets-policy, REQ-audit-hmac-chain, REQ-capability-gate, REQ-taint-propagation

**Success Criteria (what must be TRUE):**
1. Operator opens `helm` (no args) → dashboard loads in <500ms, CRIT findings visible at top
2. Selection remains on same finding across live refresh (fingerprint-pinned, not positional)
3. WHY pane shows collapsed correlations (e.g., "● WARN nginx 5xx (×17)" not 17 duplicate lines)
4. Five tabs visible (Alerts, Services, Resources, Logs, Changes) with sub-toggles for Processes/Network/Storage
5. Severity-grouped left pane (CRIT block, WARN block, INFO block) with sparklines beneath
6. Workspace builds clean; binary ≥30% smaller post-agent-strip; apply-plan integration test passes
7. Security invariants verified: HMAC chain, capability gate, taint propagation, secrets (0600 mode)

**Plans:** TBD

**Status:** Complete ✓

---

### Phase 2: M2 — Real-Time Trends

**Goal:** Operator sees the curve, not just the current value.

**Depends on:** Phase 1

**Requirements:** REQ-sparkline-history

**Success Criteria (what must be TRUE):**
1. Left pane shows sparklines beneath each severity block (cpu, mem, disk, net, load trends)
2. Resources tab renders large sparklines for top-N processes, disks, interfaces
3. Ring buffers cap at 60 points; push on each refresh; render latency ≤4ms
4. Dashboard refresh budget maintained at ≤16ms with sparklines active
5. Refresh cadence and history depth configurable via thresholds.toml

**Plans:** TBD

**Status:** Complete ✓

---

### Phase 3: M3 — Fleet (Multi-Host)

**Goal:** One dashboard, N hosts, parallel.

**Depends on:** Phase 1

**Requirements:** REQ-fleet-uuid-parallel

**Success Criteria (what must be TRUE):**
1. Fleet of 12 hosts shows "fleet: 12/12 up" in header; fleet tab displays per-host CRIT counts
2. Parallel SSH JoinSet collects findings from all hosts concurrently; 20-host refresh completes in ≤2s
3. RemoteEntry extended with host_id: Uuid; snapshot store keyed by both host_hostname and host_id
4. Credential abstraction implemented (SshAgent, KeyFile, Password); no passwords in registry file
5. Selection sticky across fleet refresh; HMAC chain verifies across multi-host episode

**Plans:** TBD

**Status:** Pending

---

### Phase 4: M4 — Kubernetes / VM / Compose

**Goal:** Cover the real infrastructure surface.

**Depends on:** Phase 3

**Requirements:** REQ-kubernetes-collector, REQ-libvirt-collector, REQ-compose-collector

**Success Criteria (what must be TRUE):**
1. Kubernetes collector (kubectl wrapper) surfaces pod events, restarts, OOMKills, PVC pressure with KubectlRead capability gate
2. libvirt/QEMU collector (virsh wrapper) surfaces domain state, snapshot age, host load
3. Docker Compose collector groups containers by project label; integrates with existing Docker collector
4. Integration tests pass against kind cluster + libvirt test VM + compose fixture in CI matrix
5. Detector tuning complete for new K8s/VM/Compose finding types; thresholds in thresholds.toml

**Plans:** TBD

**Status:** Pending

---

### Phase 5: M5 — Alerting

**Goal:** Findings page on-call when they matter.

**Depends on:** Phase 1

**Requirements:** REQ-alerting-routing, REQ-webhook-sink, REQ-slack-sink, REQ-pagerduty-sink

**Success Criteria (what must be TRUE):**
1. Alert routing module implements severity threshold (CRIT/WARN/INFO gated separately), dedup window per fingerprint, rate limiting
2. Webhook sink sends HTTP POST with JSON body and exponential backoff retry
3. Slack sink sends formatted message to incoming webhook URL with severity colors (red/yellow/blue)
4. PagerDuty sink sends Events API v2 events with dedup_key=fingerprint, resolution on lifecycle close
5. End-to-end test: trigger CRIT finding → PagerDuty event fires within 5s → resolution fires on close

**Plans:** TBD

**Status:** Pending

---

### Phase 6: M6 — AI Excellence

**Goal:** Operator selects a finding; a fix plan is already waiting.

**Depends on:** Phase 1, Phase 5

**Requirements:** REQ-auto-plan-background, REQ-auto-plan-apply

**Success Criteria (what must be TRUE):**
1. On finding selection (fingerprint change), background tokio::task spawns LLM call; result cached by fingerprint in Arc<DashMap>
2. Cache hit renders plan instantly; cache miss shows "generating…" spinner; cache hit ratio ≥80% on second open
3. Plans are capability-gated and taint-checked; capability gate enforced before execution
4. `[a]apply` hotkey routes to execute.rs apply-plan with capability gate, taint check, audit append, HMAC verify
5. Plan quality snapshot test passes (20 findings × 5 providers): numbered steps, no destructive ops without confirm, evidence linked

**Plans:** TBD

**Status:** Pending

---

## Progress

| Phase | Status | Completed |
|-------|--------|-----------|
| 1. M1 — UX Foundation + Agent Strip | Complete | 2026-05-18 |
| 2. M2 — Real-Time Trends | Complete | 2026-05-18 |
| 3. M3 — Fleet (Multi-Host) | Not started | — |
| 4. M4 — Kubernetes / VM / Compose | Not started | — |
| 5. M5 — Alerting | Not started | — |
| 6. M6 — AI Excellence | Not started | — |

---

## Verification Gates

### Per-Task Gate (Atomic Commit)

Every task must pass:
```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

### Per-Slice Gate

Slice gates are defined in linked-purring-hearth.md:
- Visible UI slices: snapshot test + manual TUI run
- Non-visible (backend): integration test
- Agent-related: apply-plan validation

### Per-Milestone RC Gate

Each milestone ends with:
```bash
cargo test deterministic_100_run -- --ignored
```

### Cross-Cutting Invariants (Every Milestone)

1. HMAC audit chain verifies after the milestone's representative episode
2. Capability gate denies `*.write` on TaintLevel::External (regression test)
3. Read-only diagnose mode blocks all write tools
4. Secrets file mode is 0600; env vars not auto-imported

### End-to-End Smoke (Post-M6)

1. `helm watch` opens directly to dashboard in <500ms
2. Top CRIT finding visible within 5s (eye-to-finding latency)
3. Select finding → fix plan renders within 2s (cache hit instant)
4. `a` → plan executes → audit log appends → HMAC verifies
5. Fleet of 12 hosts refreshes in ≤2s; selection sticky across refresh
