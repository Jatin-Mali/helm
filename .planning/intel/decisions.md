# Synthesized Decisions

Extracted from classified SPECs and locked architectural decisions.

## D1: HELMOPS Rebirth as TUI-First Incident-and-Monitoring Platform

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Rebrand HELM as HELMOPS, a TUI-first incident-and-monitoring daily driver. Operator opens it, sees the top problem in ≤5 seconds, reads a fix plan auto-generated in the background, applies it with one keystroke, and the audit log proves what happened. Strip the ReAct agent entirely; keep the security spine (capability gate, taint, HMAC audit). Add fleet, K8s, libvirt, Compose, real alerting.

**Scope:** TUI redesign, Agent strip, Fleet management, Kubernetes collector, libvirt/QEMU collector, Docker Compose collector, Alerting sinks, Auto-plan generation

**Rationale:** Current HELM fails the 5-second test (10 tabs, no severity grouping, no trend lines), selection jumps on live refresh, WHY pane has duplicate spam, no fleet view, no alerting, no auto-plan generation, and massive dead surface (31 CLI commands, 4 TUI modes, agent loop, MCP, skills, episodes, replay).

**Verification Gate:** Full workspace builds clean; binary size drop ≥30%; M1–M6 deterministic_100_run passes.

---

## D2: Fixed TUI Selection via Fingerprint-Based Pinning

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Replace `selected_finding: usize` (positional index) with `selected_fingerprint: Option<u64>` (stable finding identity). Capture fingerprint pre-refresh, restore post-refresh by fingerprint lookup, fall back to clamp if finding is gone. This prevents selection jumping when `refresh_dashboard` rebuilds the findings Vec.

**Scope:** TUI selection, Live refresh, Finding persistence

**Rationale:** Current implementation uses positional indexing, which breaks when the findings list is rebuilt. Finding fingerprints provide stable identity across refresh cycles.

---

## D3: Severity-Grouped Left Pane with Collapsed Correlations

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Group left-pane findings into three blocks (CRIT, WARN, INFO). Deduplicate correlated findings by title via `HashMap<String, (Severity, usize)>`, render `● {sev} {title} (×{count})` per group. Add ring-buffer sparklines beneath each block for cpu/mem/disk/net/load trends.

**Scope:** TUI layout, Finding correlation, Sparkline rendering

**Rationale:** Current WHY pane iterates `correlated_finding_ids` with zero dedup, emitting "● WARN has failed..." ~20× in a row. Severity grouping and collapsing makes the operator's mental load tractable.

---

## D4: Reduce TUI Tabs from 10 to 5 + Resources Subsection Toggle

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Collapse `OpsTab` enum to 5 variants: Alerts, Services, Resources, Logs, Changes. Move Processes, Network, Storage render functions under Resources tab via sub-section toggle. Compress status bar to single line (host, fleet count, uptime, clock, tab list, hotkeys).

**Scope:** TUI tabs, Status bar, Tab layout

**Rationale:** 10 tabs overwhelm operators. Five tabs with subsections improve navigation and reduce visual clutter.

---

## D5: Strip ReAct Agent, Keep Apply-Plan Execution

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Delete `helm-cli/src/serve.rs`, `agent_remote.rs`, `hooks.rs`, `ndjson_sink.rs`, `snapshot_sink.rs`, and entire `crates/helm-agent/` crate. Remove `TuiMode::{Chat,Plan,Diagnose,Auto}` and associated CLI commands (episodes, skills, sessions, mcp, replay, memory, profile, undo, redo, export, stats, serve, chat, plan, diagnose, auto). Default subcommand becomes `helm watch`. Validate that `execute.rs` apply-plan path uses only `std::process::Command` (no agent dep).

**Scope:** Agent removal, CLI simplification, Command routing

**Rationale:** Agent loop, skills, episodes, sessions, MCP, and replay are noise for a sysadmin who just wants `helm watch`. Removing them cuts binary size by ≥30% and eliminates the "AI with shell access" market misunderstanding.

---

## D6: Helm Watch as Default Zero-Arg Entry Point

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Make `helm watch` the default subcommand. Invoking `helm` with no arguments launches directly to the dashboard TUI without argument parsing or mode selection.

**Scope:** CLI entry point, Dashboard launch

**Rationale:** Sysadmins expect `helm` to "just work" and show the problem immediately.

---

## D7: Security Spine Preserved: Capability Gate, Taint, HMAC Audit

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Keep the security spine intact across the agent strip:
- HMAC-chained audit log (append-only, verifies after every episode)
- `Tainted<T>` propagation and `TaintLevel::External` on browser/SSH/MCP inputs
- Capability gate before every tool call; `*.write` blocked on tainted input
- Finding lifecycle (open/new/recurring/suppressed/resolved/self-resolved) with stable fingerprint
- Changesets (config-diff snapshots)
- Multi-provider LLM for narrative/plan generation only
- `RemoteRegistry` (extend, don't replace)
- Apply-plan path via `std::process` (no agent dep)
- Secrets policy: `$XDG_CONFIG_HOME/helm/secrets.toml` mode 0600; no env auto-import

**Scope:** Security, Audit, Taint model, Capability gates, Secrets

**Rationale:** The security model is the load-bearing layer. It must survive agent removal intact.

---

## D8: Parallel SSH Fleet with Credential Abstraction

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Extend `RemoteEntry` with `host_id: Uuid`. Implement parallel SSH `JoinSet` in `RemoteRegistry` to collect findings from N hosts concurrently. Introduce credential abstraction layer: `enum Credential { SshAgent, KeyFile(PathBuf), Password(Secret) }`. No passwords stored in registry file. Fleet panel shows `fleet: 12/12 up`; new tab `[6]Fleet` with per-host CRIT counts.

**Scope:** Fleet management, Multi-host SSH, Credential storage, Remote registry

**Rationale:** Single-host scope is a blocker for production adoption. Credential abstraction prevents credential leakage in config files.

---

## D9: Kubernetes, libvirt, Docker Compose Collectors

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Add three new collectors:
1. **Kubernetes (kubectl wrapper):** events (Warning), pod restarts, OOMKills, PVC pressure. Capability-gated `KubectlRead`.
2. **libvirt/QEMU (virsh wrapper):** domain state, snapshot age, host load.
3. **Docker Compose:** layered over existing Docker collector, groups containers by project label.

All gated by new capabilities and integrated into detector pipeline.

**Scope:** Kubernetes collector, libvirt collector, Compose collector, Detector tuning

**Rationale:** Operators run their apps on K8s, VMs, and Compose. HELM must see all three to be a daily driver.

---

## D10: Alerting Sinks with Severity Threshold, Dedup, Rate Limit

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** Implement alert routing module (`crates/helm-monitor/src/alerting/mod.rs`) with severity threshold, dedup window per fingerprint, and rate limiting. Implement three sinks:
1. **Webhook:** HTTP POST, JSON body, retry with backoff
2. **Slack:** incoming webhook URL, channel override, severity color
3. **PagerDuty:** Events API v2, dedup_key = finding fingerprint

**Scope:** Alerting, Alert routing, Severity gating, Dedup

**Rationale:** Findings sitting in a TUI don't page anyone. Real alerting is required for production adoption.

---

## D11: Auto-Plan Generation via Background LLM Call + Fingerprint-Keyed Cache

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Status:** Proposed

**Decision Statement:** On finding selection (fingerprint change), spawn background `tokio::task` that calls LLM to generate a fix plan. Cache result by fingerprint in `Arc<DashMap<Fingerprint, PlanState>>`. Render cached plan instantly on cache hit; show "generating…" spinner otherwise. Plans must be capability-gated, check taint, and append to audit log on apply.

**Scope:** Auto-plan generation, Plan caching, Background execution, Taint integration

**Rationale:** Operators must not wait for plan generation. Background cache + fingerprint keying ensures ≥80% cache hit ratio on second open of same finding.

---

## D12: Threat Model — HELM as Local Machine-Control Agent

**Source:** `/home/white_devil/code/helm/docs/threat-model.md` (SPEC, precedence 2)

**Status:** Proposed

**Decision Statement:** HELM is a Linux-first local machine-control agent. Main risk is that model output or external content can request dangerous local actions. Trust boundaries: User prompt (trusted by default), Tool output (tool-tainted), Browser/web/email/downloads (external-tainted), LLM response (untrusted until parsed).

Controls: JSON-schema validation before execution, capabilities gate dangerous actions, external-tainted requires fresh approval for privileged actions, all tool calls recorded in hash-chained audit log, browser content marked external-tainted, file tools enforce allowlist/denylist, shell has explicit exec/shell modes.

API key storage: `$XDG_CONFIG_HOME/helm/secrets.toml` mode 0600, parent 0700, atomic write, never world-readable, no silent env import.

**Scope:** Threat model, Security controls, Trust boundaries, API key storage, Secrets policy

**Rationale:** Codifies the security assumptions and controls required for a machine-control agent.

---

## D13: Trust Ladder — Five-Level Capability Hierarchy

**Source:** `/home/white_devil/code/helm/docs/trust-ladder.md` (SPEC, precedence 2)

**Status:** Proposed

**Decision Statement:** HELM's trust model is a ladder with five rungs:

- **Level 0 (Dashboard/Monitor):** Read-only snapshots and findings. No mutation.
- **Level 1 (Troubleshoot):** Builds hypotheses and read-only verification steps from findings. Fix steps rendered but not executed.
- **Level 2 (Local Approved):** Reviewed local execution with evidence, command preview (effect, risk, blast radius, rollback, verification).
- **Level 3 (Remote Approved):** SSH targets, per-target audit.
- **Level 4 (Governed Automation):** Future policy-driven automation.

**Scope:** Trust model, Capability gates, Approval workflows, Audit trail

**Rationale:** Builds trust incrementally. Users can start at Level 0 (safe) and escalate only as they gain confidence.

---

## D14: Monitoring-First DevOps Assistant Product Thesis

**Source:** `/home/white_devil/code/helm/ROADMAP.md` (SPEC, precedence 1)

**Status:** Proposed

**Decision Statement:** HELM is a read-only-first DevOps assistant for Linux systems. Primary job is not automation; it is to understand system context, surface issues operators miss, explain evidence, and guide troubleshooting with reviewed commands.

Feel safe on day one:
1. Observes before reasoning
2. Reasons before suggesting
3. Suggests before asking permission
4. Explains exactly what approved commands will do to this specific system
5. Changes nothing without explicit permission

Center of gravity: monitoring + guided troubleshooting. Automation is a later execution layer, not the core product identity.

**Scope:** Product thesis, Safety model, Operator trust

**Rationale:** Market signals clearly ask for safe visibility, context, and troubleshooting. Current positioning ("AI with shell access") misses the wedge.

---

## D15: Default Entry Point `helm watch` → Dashboard

**Source:** `/home/white_devil/code/helm/ROADMAP.md` (SPEC, precedence 1)

**Status:** Proposed

**Decision Statement:** Running `helm` with no arguments opens the terminal-native dashboard (health, findings, services, containers, disk, ports, logs, backups, plans, provider boundary, recent system context). From the dashboard, operators refresh host state, open findings, inspect evidence, run read-only follow-up checks, generate troubleshooting plans, and open the reviewed apply flow.

**Scope:** CLI entry point, Default subcommand, Dashboard launch

**Rationale:** Sysadmins expect the tool to "just work" and show the problem immediately.

---

## D16: Project Promise v1 — Monitoring Dashboard + Natural-Language Task Execution

**Source:** `/home/white_devil/code/helm/PROJECT_PROMISE.md` (PRD, precedence 1)

**Status:** Proposed

**Decision Statement:** HELM v0.1 ships with:
- Natural-language task execution via `helm "<task>"` (Anthropic API)
- Three first-class tools: shell execution, filesystem read, filesystem write
- SQLite-backed episode log of every task
- ReAct loop with iteration cap and token budget enforcement
- Apache 2.0 source on GitHub

Non-goals for v0.1: Multi-provider, browser automation, skill learning, permission/capability system, macOS/Windows.

Note: This is an older vision. The HELMOPS Rebirth (D1) supersedes this with monitoring-first and agent-strip focus.

**Scope:** v0.1 commitment, Feature scope, Tool coverage, Audit

**Rationale:** Public commitment; kept for traceability even though newer rebirth plan diverges significantly.

