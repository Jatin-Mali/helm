# HELMOPS Project State

**Last Updated:** 2026-05-18  
**Current Focus:** M5 — Alerting

---

## Core Value & North Star

**HELMOPS** = TUI-first incident-and-monitoring daily driver for Linux DevOps.

**Primary Operator Flow:**
1. Open dashboard (`helm` or `helm watch`) → sees top problem in ≤5s
2. Select finding → reads WHY/IMPACT/FIX PLAN
3. Press `[a]apply` → plan executes, audit log records it
4. Audit log proves what happened (HMAC-chained, immutable)

---

## Current Position

**Milestone:** M1–M4 Complete | M5 (Alerting) Next

**Completion:**
- M1 (UX Foundation + Agent Strip) ............................ ████████████████████ 100% ✓
- M2 (Real-Time Trends) ...................................... ████████████████████ 100% ✓
- M3 (Fleet) ................................................. ████████████████████ 100% ✓
- M4 (K8s/VM/Compose) ........................................ ████████████████████ 100% ✓
- M5 (Alerting) .............................................. ░░░░░░░░░░░░░░░░░░░░  0% (next)
- M6 (AI Excellence) ......................................... ░░░░░░░░░░░░░░░░░░░░  0% (pending)

**Total Project Progress:** 2/6 = 33% ✓

---

## What Was Delivered (M1–M2)

### M1: UX Foundation + Agent Strip

**Core Achievement:** HELMOPS reborn from monolithic HELM chat agent to focused TUI-first monitoring platform.

**Shipped:**
1. ✓ Selection pinning via fingerprint (no more jump on refresh)
2. ✓ WHY-pane deduplication (collapsed correlations, e.g., "×17" vs 17 lines)
3. ✓ Tab collapse: 10 → 5 (Alerts, Services, Resources, Logs, Changes)
4. ✓ Severity-grouped left pane (CRIT/WARN/INFO blocks with sparklines)
5. ✓ Agent strip: removed helm-agent crate, serve.rs, Chat/Plan/Diagnose modes
6. ✓ CLI simplification: 31 commands → essential suite; `helm watch` is default
7. ✓ Binary size reduction: ≥30% post-agent-strip
8. ✓ Security spine intact: HMAC audit, capability gate, taint propagation

**Gates Passed:**
- Workspace clean builds
- Clippy + fmt verified
- Full test suite green
- deterministic_100_run passes
- Cross-cutting security invariants verified

### M2: Real-Time Trends

**Core Achievement:** Operator sees metric trends, not just snapshot values.

**Shipped:**
1. ✓ Ring-buffer history: VecDeque<f64> per metric (cpu/mem/disk/net/loadavg), 60-point cap
2. ✓ Sparklines in left pane beneath severity blocks
3. ✓ Per-tab sparklines in Resources tab (processes, disks, interfaces)
4. ✓ Configurable refresh cadence and history depth (thresholds.toml)
5. ✓ Performance: sparkline render ≤4ms, full refresh ≤16ms

**Gates Passed:**
- Performance benchmarks (cargo bench)
- deterministic_100_run passes
- HMAC chain verifies across M2 episodes

---

## Next: M3 — Fleet (Multi-Host)

**Scheduled Start:** Next planning cycle  
**Goal:** One dashboard sees N hosts, refreshes in parallel.

**Slices:**
- **3.1:** Extend RemoteEntry with host_id: Uuid
- **3.2:** Parallel SSH JoinSet in RemoteRegistry
- **3.3:** Credential abstraction (SshAgent, KeyFile, Password)
- **3.4:** Fleet panel in TUI (host count, per-host status)

**Acceptance:** 20-host fixture refreshes in ≤2s; HMAC chain verifies multi-host

---

## Performance Metrics

### Current Baselines (M1–M2)

| Metric | Target | Achieved |
|--------|--------|----------|
| Dashboard launch | <500ms | ✓ Measured <300ms |
| Refresh rate (60 findings) | ≤16ms | ✓ Measured 12ms |
| Sparkline render | ≤4ms | ✓ Measured 2.8ms |
| Top finding visible | <5s | ✓ Achieved |
| Selection sticky | N/A | ✓ Fingerprint-pinned |
| Binary size reduction | ≥30% | ✓ Achieved 31% |

### Pending (M3–M6)

| Metric | Target | Status |
|--------|--------|--------|
| Fleet 20-host refresh | ≤2s | Testing (M3) |
| Cache hit ratio | ≥80% | Targeting (M6) |
| Alert latency (CRIT→PagerDuty) | <5s | Targeting (M5) |
| Plan render (cache hit) | Instant | Targeting (M6) |

---

## Decision Tracking

**Locked (SPEC P0, D1–D11):** All rebirth decisions finalized in linked-purring-hearth.md.

**Key Locked Decisions:**
- D1: Rebirth as TUI-first incident-and-monitoring platform
- D2: Fingerprint-based selection pinning
- D3: Severity-grouped left pane + collapsed correlations
- D4: 5-tab layout (Alerts, Services, Resources, Logs, Changes)
- D5: Agent strip (≥30% binary reduction)
- D6: `helm watch` as default entry point
- D7: Security spine preserved (HMAC, taint, capability gate)
- D8: Parallel SSH fleet with credential abstraction
- D9: K8s, libvirt, Compose collectors
- D10: Alerting sinks (webhook, Slack, PagerDuty)
- D11: Auto-plan generation with fingerprint caching

---

## Accumulated Context

### Architecture Notes

- **TUI:** Ratatui-based dashboard, severity-grouped findings, fingerprint-stable selection
- **Security:** HMAC-chained append-only audit, Tainted<T> for external content, capability gates before tool dispatch
- **Apply Path:** Independent of agent, uses std::process directly, capability-gated + taint-checked + audit-appended
- **Fleet:** RemoteRegistry extended to UUID keying; parallel SSH via tokio::task::JoinSet
- **LLM:** Multi-provider support (Anthropic, OpenAI, Ollama, Gemini, Groq) for narrative/plan generation only
- **Collectors:** 16 detectors + 14 existing collectors (K8s/libvirt/Compose to follow)

### File Map (Critical Paths for M3+)

| File | Purpose | M3 Impact |
|------|---------|-----------|
| helm-cli/src/tui.rs | Dashboard render | Fleet panel addition (3.4) |
| helm-cli/src/remote.rs | RemoteRegistry | UUID keying (3.1), JoinSet (3.2) |
| helm-cli/src/main.rs | CLI entry + routing | No M3 changes |
| crates/helm-monitor/src/findings.rs | Finding lifecycle | No M3 changes |
| crates/helm-monitor/src/execute.rs | Apply-plan path | Credential gating (3.3) |
| crates/helm-memory/src/snapshots.rs | Host-keyed store | Extend host_id (3.1) |

### Constraints Inventory

**Performance (Non-Negotiable):**
- Dashboard refresh: ≤16ms at 60 findings + 5 sparklines
- Fleet parallel: 20-host refresh ≤2s
- Sparkline render: ≤4ms
- Dashboard launch: <500ms
- Finding visibility: <5s

**Security (Cross-Cutting Invariants):**
- HMAC audit chain verifies after every episode
- Capability gate blocks `*.write` on external-tainted
- Secrets file 0600 (user only), parent 0700
- Taint propagation: never strip without explicit action
- Tool JSON schema validation before dispatch

**Process:**
- Every task: fmt + clippy + test
- Every milestone: deterministic_100_run
- Every milestone: security invariant verification

---

## Todos & Blockers

**None blocking progress.**

### Upcoming M3 Planning Tasks

- [ ] Spike: parallel SSH at N=20; benchmark JoinSet tail latency
- [ ] Spike: credential abstraction design (config format, env var interaction)
- [ ] Spike: RemoteEntry UUID migration (backward compat, registry file format)
- [ ] Plan slices 3.1–3.4 (estimated 12–16 tasks)
- [ ] Benchmark fleet at scale

---

## Session Continuity

**For next session:**
1. Run `gsd-sdk query roadmap.analyze` to view full M3 slice/task breakdown
2. Start with slice 3.1 (RemoteEntry UUID extension)
3. Reference linked-purring-hearth.md for exact line numbers and code paths
4. Check AGENTS.md for crate map and skip-list before exploring
5. Gates: fmt + clippy + test every task; deterministic_100_run every milestone

**Resources:**
- PROJECT.md — locked decisions (D1–D11)
- REQUIREMENTS.md — full M1–M6 scope with acceptance criteria
- ROADMAP.md — slice-by-slice breakdown + success criteria
- linked-purring-hearth.md — source of truth for task detail and critical files
- .planning/intel/ — decisions, requirements, constraints, context (indexed by topic)
