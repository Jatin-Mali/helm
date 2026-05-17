# HELMOPS — Project Context & Promises

**Date Created:** 2026-05-18  
**Status:** Active (Milestones M1–M2 complete; M3–M6 in planning)

---

## Core Value

HELMOPS is a **TUI-first incident-and-monitoring daily driver** for Linux systems.

**Primary Job:** Senior DevOps opens the dashboard, sees the top problem in ≤5 seconds, reads a fix plan auto-generated in the background, applies it with one keystroke, and the audit log proves what happened.

**Product Thesis:** Monitoring-first safety model. Read-only-first, guided troubleshooting, reviewed commands, zero automation without explicit permission. Operators feel safe on day one.

---

## What We Strip

- **ReAct agent loop** — removed entirely from codebase
- **Chat/Plan/Diagnose/Auto modes** — 4 TUI modes collapsed to 1 Dashboard
- **Agent infrastructure** — serve.rs, agent_remote.rs, hooks.rs, ndjson_sink.rs, snapshot_sink.rs
- **Massive dead CLI surface** — episodes, skills, sessions, mcp, replay, memory, profile, undo, redo, export, stats, serve, chat, plan, diagnose, auto (15+ commands deleted)
- **10-tab TUI layout** — collapsed to 5 tabs + subsection toggles
- **Binary bloat** — ≥30% size reduction target

---

## What We Keep (Security Spine)

<decision status="LOCKED">
**D7: Security Spine Preserved**

HMAC-chained append-only audit log, Tainted<T> propagation with TaintLevel::External for external content, capability gate before every tool call, *.write blocked on tainted input, finding lifecycle with stable fingerprint, changesets (config-diff snapshots), multi-provider LLM for narrative/plan generation only, RemoteRegistry (extend, don't replace), apply-plan path via std::process (no agent dep), secrets policy (0600 file mode, 0700 parent, no silent env import).
</decision>

---

## What We Add

1. **Fleet Management** — Parallel SSH JoinSet, multi-host findings, credential abstraction
2. **Infrastructure Collectors** — Kubernetes (kubectl), libvirt/QEMU (virsh), Docker Compose (project-grouped)
3. **Real Alerting** — Webhook, Slack, PagerDuty sinks with severity gate + dedup
4. **Auto-Plan Generation** — Background LLM call on finding selection, fingerprint-keyed cache (≥80% hit ratio)
5. **Real-Time Trends** — Ring-buffer metrics, sparklines in left pane + Resources tab
6. **Severity Grouping** — CRIT/WARN/INFO blocks, fingerprint-stable selection across refresh

---

## Locked Architectural Decisions

<decision status="LOCKED">
**D1: HELMOPS Rebirth as TUI-First Incident-and-Monitoring Platform**

Rebrand HELM as HELMOPS, a TUI-first incident-and-monitoring daily driver. Operator opens it, sees the top problem in ≤5 seconds, reads a fix plan auto-generated in the background, applies it with one keystroke, and the audit log proves what happened. Strip the ReAct agent entirely; keep the security spine (capability gate, taint, HMAC audit). Add fleet, K8s, libvirt, Compose, real alerting.

**Verification:** Full workspace builds clean; binary size drop ≥30%; M1–M6 deterministic_100_run passes.
</decision>

<decision status="LOCKED">
**D2: Fixed TUI Selection via Fingerprint-Based Pinning**

Replace `selected_finding: usize` (positional index) with `selected_fingerprint: Option<u64>` (stable finding identity). Capture fingerprint pre-refresh, restore post-refresh by fingerprint lookup, fall back to clamp if finding is gone. This prevents selection jumping when `refresh_dashboard` rebuilds the findings Vec.
</decision>

<decision status="LOCKED">
**D3: Severity-Grouped Left Pane with Collapsed Correlations**

Group left-pane findings into three blocks (CRIT, WARN, INFO). Deduplicate correlated findings by title via `HashMap<String, (Severity, usize)>`, render `● {sev} {title} (×{count})` per group. Add ring-buffer sparklines beneath each block for cpu/mem/disk/net/load trends.
</decision>

<decision status="LOCKED">
**D4: Reduce TUI Tabs from 10 to 5 + Resources Subsection Toggle**

Collapse `OpsTab` enum to 5 variants: Alerts, Services, Resources, Logs, Changes. Move Processes, Network, Storage render functions under Resources tab via sub-section toggle. Compress status bar to single line (host, fleet count, uptime, clock, tab list, hotkeys).
</decision>

<decision status="LOCKED">
**D5: Strip ReAct Agent, Keep Apply-Plan Execution**

Delete `helm-cli/src/serve.rs`, `agent_remote.rs`, `hooks.rs`, `ndjson_sink.rs`, `snapshot_sink.rs`, and entire `crates/helm-agent/` crate. Remove `TuiMode::{Chat,Plan,Diagnose,Auto}` and associated CLI commands. Default subcommand becomes `helm watch`. Validate that `execute.rs` apply-plan path uses only `std::process::Command` (no agent dep).

**Target:** ≥30% binary size reduction.
</decision>

<decision status="LOCKED">
**D6: Helm Watch as Default Zero-Arg Entry Point**

Make `helm watch` the default subcommand. Invoking `helm` with no arguments launches directly to the dashboard TUI without argument parsing or mode selection. Launch latency: <500ms.
</decision>

<decision status="LOCKED">
**D8: Parallel SSH Fleet with Credential Abstraction**

Extend `RemoteEntry` with `host_id: Uuid`. Implement parallel SSH `JoinSet` in `RemoteRegistry` to collect findings from N hosts concurrently. Introduce credential abstraction layer: `enum Credential { SshAgent, KeyFile(PathBuf), Password(Secret) }`. No passwords stored in registry file. Fleet panel shows `fleet: 12/12 up`; new tab `[6]Fleet` with per-host CRIT counts.

**Target:** 20-host fixture completes full refresh in ≤2s.
</decision>

<decision status="LOCKED">
**D9: Kubernetes, libvirt, Docker Compose Collectors**

Add three new collectors:
1. **Kubernetes (kubectl wrapper):** events (Warning), pod restarts, OOMKills, PVC pressure. Capability-gated `KubectlRead`.
2. **libvirt/QEMU (virsh wrapper):** domain state, snapshot age, host load.
3. **Docker Compose:** layered over existing Docker collector, groups containers by project label.

All gated by new capabilities and integrated into detector pipeline.
</decision>

<decision status="LOCKED">
**D10: Alerting Sinks with Severity Threshold, Dedup, Rate Limit**

Implement alert routing module (`crates/helm-monitor/src/alerting/mod.rs`) with severity threshold, dedup window per fingerprint, and rate limiting. Implement three sinks:
1. **Webhook:** HTTP POST, JSON body, retry with backoff
2. **Slack:** incoming webhook URL, channel override, severity color
3. **PagerDuty:** Events API v2, dedup_key = finding fingerprint
</decision>

<decision status="LOCKED">
**D11: Auto-Plan Generation via Background LLM Call + Fingerprint-Keyed Cache**

On finding selection (fingerprint change), spawn background `tokio::task` that calls LLM to generate a fix plan. Cache result by fingerprint in `Arc<DashMap<Fingerprint, PlanState>>`. Render cached plan instantly on cache hit; show "generating…" spinner otherwise. Plans must be capability-gated, check taint, and append to audit log on apply.

**Target:** ≥80% cache hit ratio on second open of same finding.
</decision>

---

## Non-Locked Design Decisions (Informed by Research)

<decision status="PROPOSED">
**D12: Threat Model — HELM as Local Machine-Control Agent**

HELM is a Linux-first local machine-control agent. Main risk is that model output or external content can request dangerous local actions. Trust boundaries: User prompt (trusted by default), Tool output (tool-tainted), Browser/web/email/downloads (external-tainted), LLM response (untrusted until parsed).

Controls: JSON-schema validation before execution, capabilities gate dangerous actions, external-tainted requires fresh approval for privileged actions, all tool calls recorded in hash-chained audit log, browser content marked external-tainted, file tools enforce allowlist/denylist, shell has explicit exec/shell modes.

API key storage: `$XDG_CONFIG_HOME/helm/secrets.toml` mode 0600, parent 0700, atomic write, never world-readable, no silent env import.
</decision>

<decision status="PROPOSED">
**D13: Trust Ladder — Five-Level Capability Hierarchy**

HELM's trust model is a ladder with five rungs:
- **Level 0 (Dashboard/Monitor):** Read-only snapshots and findings. No mutation.
- **Level 1 (Troubleshoot):** Builds hypotheses and read-only verification steps from findings. Fix steps rendered but not executed.
- **Level 2 (Local Approved):** Reviewed local execution with evidence, command preview (effect, risk, blast radius, rollback, verification).
- **Level 3 (Remote Approved):** SSH targets, per-target audit.
- **Level 4 (Governed Automation):** Future policy-driven automation.
</decision>

<decision status="PROPOSED">
**D14: Monitoring-First DevOps Assistant Product Thesis**

HELM is a read-only-first DevOps assistant for Linux systems. Primary job is not automation; it is to understand system context, surface issues operators miss, explain evidence, and guide troubleshooting with reviewed commands.

Feel safe on day one:
1. Observes before reasoning
2. Reasons before suggesting
3. Suggests before asking permission
4. Explains exactly what approved commands will do to this specific system
5. Changes nothing without explicit permission

Center of gravity: monitoring + guided troubleshooting. Automation is a later execution layer, not the core product identity.
</decision>

---

## Constraints (Non-Negotiable)

### Performance

- **Dashboard refresh:** ≤16ms at 60 findings + 5 sparklines (cargo bench)
- **Fleet parallel SSH:** 20-host fixture completes in ≤2s
- **Sparkline render:** ≤4ms at 60 points × 8 metrics
- **Dashboard launch:** <500ms
- **Top finding visible:** within 5s (eye-to-finding latency)
- **Plan render:** within 2s (cache hit instant)
- **Auto-plan cache hit ratio:** ≥80% on second open

### Security (Invariants)

- **Secrets file mode:** 0600 (user only), parent 0700
- **API key resolution:** CLI flag → secrets store → env var (no silent auto-import)
- **Audit append-only:** Hash chain, verifies after every episode
- **Capability gate:** Blocks `*.write` on `TaintLevel::External` until fresh approval
- **Tool validation:** JSON schema validation before dispatch
- **Finding fingerprints:** Stable across refresh cycles
- **Apply-plan independence:** Uses only `std::process::Command`, zero agent deps
- **Binary size reduction:** ≥30% post-agent-strip

### Process

- **Atomic commit gate:** cargo fmt + clippy + test (every task)
- **Milestone RC gate:** cargo test deterministic_100_run -- --ignored
- **Cross-cutting invariants:** HMAC chain, capability gate, taint, secrets (every milestone)

---

## Out of Scope (Explicit Non-Goals)

- Fine-tuning, weight updates, self-modifying code (never)
- Windows / macOS support (deferred to v2.0)
- Web UI (TUI is the product)
- General chat / Q&A interface
- MCP / skills / sessions / episodes / replay UX surface

---

## Active Milestones

**M1 (Complete):** UX Foundation + Agent Strip  
**M2 (Complete):** Real-Time Trends  
**M3 (Pending):** Fleet (Multi-Host)  
**M4 (Pending):** Kubernetes / VM / Compose  
**M5 (Pending):** Alerting  
**M6 (Pending):** AI Excellence  

See ROADMAP.md for full slice-by-slice breakdown.
