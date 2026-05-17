# Intel Synthesis Summary

**Generated:** 2026-05-18
**Mode:** new (fresh bootstrap)
**Status:** READY

---

## Input Overview

**Total Documents Classified:** 12

| Type | Count | Precedence Range |
|------|-------|------------------|
| SPEC | 5 | P0–P2 |
| PRD | 1 | P1 |
| DOC | 6 | P3–P4 |

**Key Document (Source of Truth):**
- `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, P0)
  - HELMOPS Rebirth Roadmap: TUI-first incident-and-monitoring platform
  - Defines M1–M6 milestones with atomic slices, verification gates, and critical file map

---

## Synthesized Intel Files

All files written to `/home/white_devil/code/helm/.planning/intel/`

### decisions.md
**Decisions Extracted:** 16

High-precedence locked decisions from SPEC documents:

1. **D1–D11** — HELMOPS Rebirth decisions (from linked-purring-hearth.md, SPEC P0)
   - Rebirth as TUI-first incident-and-monitoring platform
   - Fixed TUI selection via fingerprint-based pinning
   - Severity-grouped left pane with collapsed correlations
   - Tab reduction (10 → 5) + status bar collapse
   - Agent strip with apply-plan preservation
   - Default `helm watch` entry point
   - Security spine preservation (capability gate, taint, HMAC audit)
   - Parallel SSH fleet with credential abstraction
   - K8s, libvirt, Compose collectors
   - Alerting sinks (webhook, Slack, PagerDuty)
   - Auto-plan generation via background LLM + fingerprint cache

2. **D12–D14** — Security/Trust decisions (from threat-model.md, trust-ladder.md, SPEC P2)
   - Threat model: HELM as local machine-control agent
   - Trust ladder: five-level capability hierarchy
   - Monitoring-first product thesis

3. **D15–D16** — Product strategy (from ROADMAP.md P1, PROJECT_PROMISE.md P1)
   - Default entry point `helm watch` → dashboard
   - v0.1 promise (superseded by rebirth but retained)

---

### requirements.md
**Requirements Extracted:** 25

Ordered by precedence; covers M1–M6 delivery scope:

**M1 — UX Foundation + Agent Strip (9 REQs)**
- REQ-ux-dashboard-5sec: 5-second operator perception target
- REQ-tui-selection-sticky: Selection persistence across refresh
- REQ-tui-why-pane-dedup: Correlation deduplication
- REQ-tui-tabs-5-layout: Tab collapse + severity grouping
- REQ-agent-strip: Remove ReAct agent, reduce binary ≥30%
- REQ-fleet-uuid-parallel: Multi-host SSH fleet (M3 prep)
- Additional security invariant requirements

**M2 — Real-Time Trends (1 REQ)**
- REQ-sparkline-history: Ring-buffer metrics + rendering

**M3–M6 — Fleet, Infrastructure, Alerting, AI (15 REQs)**
- Fleet: credential abstraction, parallel SSH
- K8s/libvirt/Compose collectors
- Alerting: webhook, Slack, PagerDuty sinks
- Auto-plan: background generation + caching
- Security: HMAC audit, capability gate, taint propagation, secrets policy

---

### constraints.md
**Constraints Extracted:** 21

Performance, security, and process constraints:

**Performance (4 constraints)**
- Dashboard refresh ≤16ms at 60 findings + 5 sparklines
- 20-host fleet refresh ≤2s
- Sparkline render ≤4ms
- Auto-plan cache hit ratio ≥80%

**UX/Latency (3 constraints)**
- Dashboard launch <500ms
- Top finding visible within 5s
- Plan render within 2s (cache hit instant)

**Process/Gating (3 constraints)**
- Atomic commit gate: cargo fmt/clippy/test
- Milestone RC gate: deterministic_100_run
- Cross-cutting invariants: HMAC, capability gate, taint, secrets

**Security/Data (11 constraints)**
- Secrets file mode 0600, parent 0700
- API key resolution order (CLI → store → env)
- Audit append-only with hash chain
- Capability gate blocks `*.write` on external taint
- Tool JSON schema validation
- No destructive ops without approval
- Apply-plan independence from agent
- Finding fingerprint stability
- Binary size reduction ≥30%

---

### context.md
**Context Topics Extracted:** 10

Supporting documentation organized by topic:

1. **HELM Project Overview** — Foundational context from README
2. **Providers and LLM Integration** — Multi-provider support (Anthropic, OpenAI, Ollama, Gemini, Groq)
3. **Agent-on-Remote Transport** — Legacy documentation (being replaced by direct SSH fleet)
4. **Detector False-Positive Review** — v1.8 quality checklist for findings
5. **v1.0 Release Notes** — Feature inventory from v1.0 release
6. **Troubleshooting Guide** — Operator workflows and common issues
7. **Security Model — Trust Boundaries** — Threat model zones and controls
8. **Trust Ladder — Capability Escalation** — Five-rung approval hierarchy
9. **Critical Files and Codebase Map** — File-by-file implementation guide (line number references)
10. **Milestones and Delivery Timeline** — M1–M6 overview and slicing
11. **Verification Strategy** — Task/slice/milestone/cross-cutting/end-to-end gates
12. **Out of Scope** — Explicit non-goals (Windows/macOS, fine-tuning, web UI, MCP/skills)
13. **Spikes** — Pre-commitment research (parallel SSH, sparkline cost, cache economics)

---

## Conflict Detection

**Report Location:** `/home/white_devil/code/helm/.planning/INGEST-CONFLICTS.md`

**Summary:**
- **Blockers:** 0
- **Competing Variants:** 0
- **Auto-Resolved:** 8 (all by precedence, no locked contradictions)

**Key Resolutions:**
- linked-purring-hearth.md (SPEC P0) is canonical for rebirth architecture
- threat-model.md (SPEC P2) is canonical for security/secrets design
- ROADMAP.md (SPEC P1) is canonical for monitoring-first product thesis
- PROJECT_PROMISE.md (PRD P1) is superseded by rebirth; retained for traceability

---

## Coverage Map

**Documents → Intel:**

| Source | Type | Content → Intel File |
|--------|------|----------------------|
| linked-purring-hearth.md | SPEC P0 | Decisions (D1–D11), Requirements (REQ-*), Constraints (CONST-*), Context (critical files, milestones, verification) |
| threat-model.md | SPEC P2 | Decisions (D12), Constraints (secrets, audit), Context (trust boundaries) |
| trust-ladder.md | SPEC P2 | Decisions (D13), Constraints (capability gate), Context (trust ladder rungs) |
| ROADMAP.md | SPEC P1 | Decisions (D14–D15), Context (product thesis, final state) |
| PROJECT_PROMISE.md | PRD P1 | Decisions (D16 — historical), Context (v0.1 vision, superseded) |
| README.md | DOC P3 | Context (project overview, architecture foundation) |
| providers.md | DOC P3 | Context (LLM provider support) |
| agent-on-remote.md | DOC P4 | Context (legacy NDJSON transport, superseded by fleet) |
| detector-review-checklist.md | DOC P4 | Context (detector quality criteria) |
| release-notes-v1.0.md | DOC P4 | Context (v1.0 feature inventory) |
| troubleshooting.md | DOC P4 | Context (operator workflows, common issues) |

---

## Ready for Downstream

**Intel Directory Contents:**
- `/home/white_devil/code/helm/.planning/intel/decisions.md` (16 decisions)
- `/home/white_devil/code/helm/.planning/intel/requirements.md` (25 requirements, M1–M6)
- `/home/white_devil/code/helm/.planning/intel/constraints.md` (21 constraints: perf, security, process)
- `/home/white_devil/code/helm/.planning/intel/context.md` (13 context topics)
- `/home/white_devil/code/helm/.planning/intel/SYNTHESIS.md` (this file)

**Conflicts Report:**
- `/home/white_devil/code/helm/.planning/INGEST-CONFLICTS.md` (0 blockers, 0 variants, 8 auto-resolved)

**Status:** ✓ READY
- All documents consumed
- No blocking conflicts
- Precedence rules applied cleanly
- Cycle detection passed (no cycles in cross-ref graph)
- Safe for gsd-roadmapper to route → PROJECT.md, REQUIREMENTS.md, ROADMAP.md generation

---

## Validation Checklist

- [x] All 12 classifications in CLASSIFICATIONS_DIR consumed
- [x] Cycle detection run on cross-ref graph (no cycles detected)
- [x] Per-type intel files written to INTEL_DIR
- [x] INGEST-CONFLICTS.md written with three buckets (0 blockers, 0 variants, 8 auto-resolved)
- [x] SYNTHESIS.md written as entry point
- [x] LOCKED-vs-LOCKED check passed (no locked documents in ingest)
- [x] Competing variants check passed (no PRD conflicts)
- [x] Confidence check passed (no UNKNOWN-confidence docs)
- [x] All decisions mapped to source with precedence
- [x] All requirements traced to PRD/SPEC with acceptance criteria
- [x] All constraints tied to source with rationale
- [x] Context topics sourced and attributed

---

## Next Steps

**For gsd-roadmapper:**
1. Read `/home/white_devil/code/helm/.planning/intel/SYNTHESIS.md` (this file)
2. Consume `/home/white_devil/code/helm/.planning/intel/{decisions,requirements,constraints,context}.md`
3. Generate `/home/white_devil/code/helm/.planning/PROJECT.md` (project context + promises)
4. Generate `/home/white_devil/code/helm/.planning/REQUIREMENTS.md` (M1–M6 rollup)
5. Generate `/home/white_devil/code/helm/.planning/ROADMAP.md` (slice-by-slice breakdown)

**For operators:**
1. Review `/home/white_devil/code/helm/.planning/INGEST-CONFLICTS.md` for any concerns
2. Cross-check against existing codebase if merge-mode (not applicable in fresh bootstrap)
3. Proceed to roadmapper output for planning and task assignment

