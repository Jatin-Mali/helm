# Synthesized Context

Supporting documentation and contextual references extracted from classified DOCs.

---

## Topic: HELM Project Overview

**Source:** `/home/white_devil/code/helm/README.md` (DOC, precedence 3)

HELM is a TypeScript/Rust DevOps monitoring and automation system. The project provides:

- Terminal user interface (TUI) for system monitoring and incident response
- Multi-provider LLM integration (Anthropic, OpenAI, Ollama, Gemini, and compatible APIs)
- Typed Linux machine-control tools (process, service, package, disk, network, logs)
- Audit log with HMAC chaining for compliance
- Capability gates and taint propagation for security
- Episode log (SQLite-backed session tracking)
- Troubleshooting and auto-plan generation workflows

The current version (M1 complete) focuses on read-only-first monitoring with safe escalation to troubleshooting and eventual automation.

---

## Topic: Providers and LLM Integration

**Source:** `/home/white_devil/code/helm/docs/providers.md` (DOC, precedence 3)

HELM supports multiple LLM providers for narrative generation and plan synthesis:

- **Anthropic Claude** — primary provider, full feature set
- **OpenAI** — gpt-4, gpt-3.5-turbo via OpenAI API
- **OpenAI-compatible** — local or third-party APIs matching OpenAI schema
- **Ollama** — local models (Llama 2, Mistral, etc.)
- **Gemini** — Google Gemini API
- **Groq** — high-speed inference via Groq API
- **Mock provider** — testing and offline development

Provider selection is configured via CLI flags or interactive prompt on first use. Each provider has quirks (token limits, format preferences, failure modes) documented in `quirks.rs`.

---

## Topic: Agent-on-Remote Transport (Obsolete in Rebirth)

**Source:** `/home/white_devil/code/helm/docs/agent-on-remote.md` (DOC, precedence 4)

Agent-on-remote transport uses NDJSON over SSH for remote task execution. This is part of the legacy agent infrastructure being removed in M1 (agent strip).

Documentation is kept for historical reference. The rebirth replaces this with direct SSH-based finding collection from remote hosts (via parallel `JoinSet` in RemoteRegistry).

---

## Topic: Detector False-Positive Review Checklist

**Source:** `/home/white_devil/code/helm/docs/detector-review-checklist.md` (DOC, precedence 4)

Detectors are pattern-matching rules that classify system state into findings. Version 1.8 of the checklist defines review criteria:

- Signal clarity: Does the detector reliably identify a real issue?
- Specificity: False-positive rate < 5% on baseline workloads?
- Severity assignment: Does the severity match the operator's expected response time?
- Remediability: Can an operator take action based on the finding?
- Evolvability: Can the threshold be configured per environment via `thresholds.toml`?

All new detectors (e.g., for K8s, libvirt, Compose in M4) must pass this checklist before merge.

---

## Topic: v1.0 Release Notes and Features

**Source:** `/home/white_devil/code/helm/docs/release-notes-v1.0.md` (DOC, precedence 4)

v1.0 release shipped:

- Terminal TUI with 5 tabs: Alerts, Services, Resources, Logs, Changes
- 16 detectors + 14 collectors
- Capability gates (Level 0–2 of trust ladder)
- Taint model with external-tainted inputs
- HMAC-chained audit log
- Episode-backed session memory
- Multi-provider LLM support
- Sandbox mode for safe exploration
- Troubleshooting and plan-generation workflows

Post-v1.0 (M1–M6), the rebirth adds:
- Sparkline history (M2)
- Fleet management with parallel SSH (M3)
- K8s, libvirt, Compose collectors (M4)
- Webhook, Slack, PagerDuty alerting (M5)
- Auto-plan caching and background generation (M6)

---

## Topic: Troubleshooting Guide

**Source:** `/home/white_devil/code/helm/docs/troubleshooting.md` (DOC, precedence 4)

Operators commonly encounter:

1. **Dashboard renders slowly** → Check detector complexity, spawn in background, tune thresholds in `thresholds.toml`
2. **Selection jumps on refresh** → Known bug in v1.0, fixed in M1.1.1 via fingerprint pinning
3. **Plan execution fails** → Verify capability is granted, check audit log for denials
4. **Remote host unreachable** → SSH timeout, verify registry credentials, check host_id in dashboard
5. **Secrets file permission errors** → Verify mode 0600, parent 0700, check `helm secrets status`

All troubleshooting paths include `helm doctor` diagnostic output and log tailing via `helm logs`.

---

## Topic: Security Model — Trust Boundaries

**Source:** `/home/white_devil/code/helm/docs/threat-model.md` (SPEC, precedence 2)

HELM operates in four trust zones:

1. **Trusted User Prompt** — User input is trusted by default; explicit task commands
2. **Untrusted LLM Output** — Model response requires validation before execution
3. **External-Tainted Input** — Browser, SSH, MCP, web content marked `TaintLevel::External`
4. **Tool Output** — System tool output is tool-tainted but distinct from external

Interactions flow through these controls:
- JSON schema validation (before execution)
- Capability gate (feature availability)
- Taint level check (privilege escalation guard)
- Audit log (append-only, hash-chained)

---

## Topic: Trust Ladder — Capability Escalation

**Source:** `/home/white_devil/code/helm/docs/trust-ladder.md` (SPEC, precedence 2)

Five rungs, each with increasing capability and audit burden:

| Level | Name | Capability | Approval Req | Invocation |
|-------|------|-----------|-------------|-----------|
| 0 | Dashboard/Monitor | Read-only snapshots + findings | None | `helm`, `helm snapshot`, `helm monitor` |
| 1 | Troubleshoot | Read-only verification + plan render | None | `helm troubleshoot "<problem>"` |
| 2 | Local Approved | Reviewed local execution | Per-command | `helm explain`, followed by `[a]pply` in TUI |
| 3 | Remote Approved | SSH targets, per-target audit | Per-host + target | Fleet operations, remote `helm apply-plan` |
| 4 | Governed Automation | Policy-driven automation (future) | Policy engine | Not yet implemented |

Users may start at Level 0 (safe, read-only) and escalate only as confidence grows. Each level requires explicit approval or invocation change.

---

## Topic: Critical Files and Codebase Map

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

Key files for HELMOPS Rebirth:

**CLI and TUI:**
- `helm-cli/src/tui.rs` — Dashboard rewrite (L1008–1046 DashboardData, L1143 selected_finding→fingerprint, L4351–4360 clamp, L4441/L4808 refresh, L5414 spawn, L6017–6144 detail pane, L10090–11600 tabs)
- `helm-cli/src/main.rs` — Strip TuiMode + commands (L133–187, L275–282, L1721–1725); remove helm_agent imports (L35, L5902, L6458, L7030)
- `helm-cli/src/remote.rs` — Add host_id UUID, parallel JoinSet (L11, L28, L120)

**Deletion:**
- `helm-cli/src/serve.rs` — Agent JSON-RPC server
- `helm-cli/src/agent_remote.rs`, `hooks.rs`, `ndjson_sink.rs`, `snapshot_sink.rs` — Agent event sinks
- `crates/helm-agent/` — Entire ReAct loop crate

**Core Monitoring:**
- `crates/helm-monitor/src/findings.rs` — Add `correlate()` + `dedup_by_title()` near `compute_fingerprint` (L224)
- `crates/helm-monitor/src/execute.rs` — Validate apply-plan (L235–293 uses std::process directly)
- `crates/helm-monitor/src/collectors/` — New: `kubernetes.rs`, `libvirt.rs`, `compose.rs`
- `crates/helm-monitor/src/alerting/` — New: `webhook.rs`, `slack.rs`, `pagerduty.rs`

**Memory and State:**
- `crates/helm-memory/src/snapshots.rs` — Host-keyed (L11, L43–44, L60), extend host_id
- `Cargo.toml` — Remove `helm-agent` member

---

## Topic: Milestones and Delivery Timeline

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

Six milestones, each atomic commit gated by `cargo fmt`, `cargo clippy`, `cargo test`, and RC gate at milestone end.

**M1 — UX Foundation + Agent Strip** (slices 1.1–1.4)
- Goal: Operator opens HELMOPS, sees grouped findings, selects one, reads it without it jumping, binary is half the size
- Slices: Fix selection-jump bug, Fix WHY-pane dedup, Tab + status-bar collapse, Strip the agent

**M2 — Real-Time Trends** (slices 2.1–2.4)
- Goal: Operator sees the curve, not just the current value
- Slices: Ring-buffer history, Ratatui Sparkline rendering, Per-tab sparklines, Configurable refresh cadence

**M3 — Fleet (Multi-Host)** (slices 3.1–3.4)
- Goal: One dashboard, N hosts, parallel
- Slices: Extend RemoteEntry UUID, Parallel SSH JoinSet, Credential abstraction, Fleet panel

**M4 — Kubernetes / VM / Compose** (slices 4.1–4.4)
- Goal: Cover the real infrastructure surface
- Slices: Kubernetes collector, libvirt collector, Compose collector, Detector tuning

**M5 — Alerting** (slices 5.1–5.4)
- Goal: It pages people
- Slices: Alert routing module, Webhook sink, Slack sink, PagerDuty sink

**M6 — AI Excellence** (slices 6.1–6.4)
- Goal: Operator selects a finding; a fix plan is already waiting
- Slices: Auto-plan on selection, Render cached plan, Plan-quality test, Apply hotkey

---

## Topic: Verification Strategy

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

Three-tier verification:

**Per Task (atomic commit):**
```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

**Per Slice (slice gate):**
- Manual TUI run for visible slices
- Integration test for non-visible slices
- Snapshot test where applicable

**Per Milestone (RC gate):**
```bash
cargo test deterministic_100_run -- --ignored
```

**Cross-Cutting Invariants (every milestone):**
1. HMAC audit chain verifies after the milestone's representative episode
2. Capability gate denies `*.write` on `TaintLevel::External` (regression test)
3. Read-only diagnose-equivalent mode blocks all write tools
4. Secrets file mode is 0600; env vars not auto-imported

**End-to-End Smoke (post-M6):**
1. `helm watch` opens directly to dashboard in <500ms
2. Top CRIT finding visible within 5s
3. Select finding → fix plan renders within 2s (cache hit instant)
4. `a` → plan executes → audit log appends → HMAC verifies
5. Fleet of 12 hosts refreshes in ≤2s; selection sticky across refresh

---

## Topic: Out of Scope (Explicitly Deferred)

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

Do not build in M1–M6:
- Fine-tuning, weight updates, self-modifying code (never, per CLAUDE.md §10)
- Windows / macOS support (deferred to v2.0)
- Web UI (TUI is the product)
- General chat / Q&A interface
- MCP / skills / sessions / episodes / replay UX surface

These are explicitly not requirements and should not be implemented as part of this roadmap.

---

## Topic: Spikes (Pre-Commitment Research)

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

Three spikes to run before M3 final design:

1. **Parallel SSH at N=20:** Benchmark `JoinSet` vs serial; measure tail latency; decide pool size
2. **Sparkline render cost:** Measure ratatui Sparkline at 60-point buffer × 8 metrics; budget ≤4ms
3. **Auto-plan cache economics:** Measure LLM round-trip per provider; expected cache hit ratio on 24h trace; decide TTL

Results inform performance budgets and architectural decisions.

