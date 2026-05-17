# Synthesized Constraints

Technical and operational constraints extracted from classified SPECs.

## CONST-tui-perf-refresh

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Performance

**Title:** Dashboard Refresh Performance Budget

**Constraint:** Full dashboard refresh must complete in ≤16ms at 60 findings + 5 sparklines. Measured in `cargo bench`.

**Rationale:** Visual fluidity at 60 FPS (16.6ms per frame) is required for a TUI to feel responsive. Refresh is on the critical path of every user interaction.

---

## CONST-fleet-parallel-latency

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Performance

**Title:** Fleet Parallel SSH Tail Latency

**Constraint:** 20-host fleet (fixture with mock SSH responder) must complete full refresh in ≤2s.

**Rationale:** Operators expect dashboard updates in seconds, not minutes. Tail latency is the constraint, not median.

---

## CONST-sparkline-render-budget

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Performance

**Title:** Sparkline Rendering Performance

**Constraint:** Ratatui Sparkline rendering at 60-point buffer × 8 metrics (left pane) must consume ≤4ms.

**Rationale:** Sparklines are visual noise if they consume significant CPU. Budget is a tight spike.

---

## CONST-auto-plan-cache-economics

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Performance

**Title:** Auto-Plan Cache Hit Ratio

**Constraint:** Cache hit ratio ≥80% on second open of same finding; measure LLM round-trip per provider; decide TTL on 24h trace.

**Rationale:** Users see the same problems repeatedly. Cache misses degrade experience. 80% is the threshold for "feels instant".

---

## CONST-task-verification-gate

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Process

**Title:** Atomic Commit Verification Gate

**Constraint:** Every task (atomic commit) must pass:
```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

**Rationale:** Prevents technical debt accumulation and ensures consistent code quality across milestones.

---

## CONST-milestone-rc-gate

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Process

**Title:** Milestone Release Candidate Gate

**Constraint:** Each milestone ends with RC gate: `cargo test deterministic_100_run -- --ignored`

**Rationale:** Deterministic tests catch race conditions, non-determinism, and flaky behavior before release.

---

## CONST-cross-cutting-invariants

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Security

**Title:** Cross-Cutting Security Invariants

**Constraint:** Every milestone must verify:
1. HMAC audit chain verifies after the milestone's representative episode
2. Capability gate denies `*.write` on TaintLevel::External (regression test)
3. Read-only diagnose-equivalent mode blocks all write tools
4. Secrets file mode is 0600; env vars not auto-imported

**Rationale:** Security invariants must not regress as features are added.

---

## CONST-helm-watch-latency

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Performance

**Title:** Dashboard Launch Latency

**Constraint:** `helm watch` opens directly to dashboard in <500ms.

**Rationale:** Operator expects immediate visual feedback when invoking the tool.

---

## CONST-finding-visibility

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** UX

**Title:** Top Finding Eye-to-Finding Latency

**Constraint:** Top CRIT finding visible within 5s (eye-to-finding latency).

**Rationale:** This is the core user expectation: "I open HELMOPS and see my problem in 5 seconds."

---

## CONST-apply-plan-instant

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Performance

**Title:** Plan Render Latency

**Constraint:** Select finding → fix plan renders within 2s (cache hit instant).

**Rationale:** Cache hit should be imperceptible; cache miss is a spike but within user tolerance.

---

## CONST-fleet-selection-sticky

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** UX

**Title:** Selection Persistence Across Refresh

**Constraint:** Fleet of 12 hosts refreshes in ≤2s; selection sticky across refresh.

**Rationale:** Operator must not lose context during multi-host operations.

---

## CONST-secrets-storage-mode

**Source:** `/home/white_devil/code/helm/docs/threat-model.md` (SPEC, precedence 2)

**Type:** Security

**Title:** Secrets File Unix Mode

**Constraint:** Secrets file `$XDG_CONFIG_HOME/helm/secrets.toml` must be mode 0600 (user only), parent directory 0700. HELM refuses to load if wider. Atomic write via temp-file + rename.

**Rationale:** Prevents accidental credential leakage via file permissions.

---

## CONST-api-key-resolution-order

**Source:** `/home/white_devil/code/helm/docs/threat-model.md` (SPEC, precedence 2)

**Type:** Security

**Title:** API Key Resolution Order

**Constraint:** API key resolution: CLI `--api-key` flag → secrets store → environment variable. No silent env auto-import by TUI.

**Rationale:** Explicit ordering prevents surprises and accidental credential leakage from environment pollution.

---

## CONST-capability-gate-blocking

**Source:** `/home/white_devil/code/helm/docs/trust-ladder.md` (SPEC, precedence 2)

**Type:** Security

**Title:** Write Capability on External Taint

**Constraint:** Capability gate blocks `*.write` on TaintLevel::External until fresh approval is obtained.

**Rationale:** Prevents model-prompted writes to the filesystem based on untrusted external input.

---

## CONST-audit-append-only

**Source:** `/home/white_devil/code/helm/docs/threat-model.md` (SPEC, precedence 2)

**Type:** Security

**Title:** Audit Log Append-Only Invariant

**Constraint:** Audit log is append-only; no modification or deletion. Hash chain with previous record. Verification runs after every episode.

**Rationale:** Forensic integrity. Tampering is detectable.

---

## CONST-snapshot-host-keyed

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Data Structure

**Title:** Snapshot Storage Keying

**Constraint:** Snapshot store is keyed by `host_hostname` (already), extend to `host_id: Uuid` for multi-host fleet.

**Rationale:** Each host must have independent snapshot history for proper fleet diagnostics.

---

## CONST-no-destructive-without-approval

**Source:** `/home/white_devil/code/helm/docs/trust-ladder.md` (SPEC, precedence 2)

**Type:** Security

**Title:** Destructive Operations Approval Gate

**Constraint:** No destructive operation (file write, service restart, package install, process kill) may execute without explicit user approval. Command preview must include expected effect, risk, blast radius, rollback, and verification steps.

**Rationale:** Builds operator trust and prevents accidents.

---

## CONST-finding-fingerprint-stable

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Data Structure

**Title:** Finding Fingerprint Stability

**Constraint:** Finding fingerprints must be stable across refresh cycles. Fingerprints computed via `compute_fingerprint` (findings.rs:224). Selection pinned to fingerprint, not position.

**Rationale:** Allows selection to persist without position-dependent indexing.

---

## CONST-tool-json-schema-validation

**Source:** `/home/white_devil/code/helm/docs/threat-model.md` (SPEC, precedence 2)

**Type:** Security

**Title:** Tool Input Validation

**Constraint:** All tool inputs validated against JSON schema before execution. Validation must pass before dispatch.

**Rationale:** Prevents malformed tool calls from reaching the execution layer.

---

## CONST-no-agent-in-apply

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Architecture

**Title:** Apply-Plan Independence from Agent

**Constraint:** Apply-plan path (`crates/helm-monitor/src/execute.rs:235-293`) must use only `std::process::Command` directly. Zero agent dependencies.

**Rationale:** Agent removal must not break the apply-plan execution path, which is the critical user workflow.

---

## CONST-binary-size-reduction

**Source:** `/home/white_devil/.claude/plans/linked-purring-hearth.md` (SPEC, precedence 0)

**Type:** Performance

**Title:** Binary Size Reduction Target

**Constraint:** Agent strip must reduce binary size by ≥30%.

**Rationale:** Smaller binary = faster download, faster startup, lower resource usage. Tangible evidence of successful agent removal.

