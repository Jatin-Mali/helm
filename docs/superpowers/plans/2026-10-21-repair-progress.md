# Repair execution ledger

Branch: `repair/verified-project-completion`. Baseline: `fbbbec3631b23965f692893e0a35b24e43f9f680`.
Plan: `2026-10-21-project-repair.md`; spec: `2026-10-21-project-repair-spec.md`.

Existing user changes in AGENTS.md and .codex/ are preserved. Audit, graph and plan artifacts predate implementation.

Ruling: use a dedicated branch in the supplied checkout, retaining the existing build cache and untracked audit/plan artifacts — avoids copying the entire project and rebuilding dependencies; branch isolation does not isolate filesystem edits.

## Task 1 — complete

Baseline audit: workspace tests passed (732 executions); formatting, strict Clippy and otel compilation failed. Reproducing otel build before repair; repairing known failures without changing monitoring behavior.

## Remaining tasks

2 approvals/schema; 3 process/policy; 4 restoration; 5 monitoring; 6 paths/fleet/retention; 7 privacy/audit; 8 web; 9 planning/TUI; 10 agent runtime; 11 agent tools; 12 sessions/workflows; 13 distribution; 14 real demonstrations.

Verification: reproduced otel compile failure, then cargo check -p helm-cli --features otel PASS; cargo fmt --check PASS; strict workspace all-target Clippy PASS; cargo test -p helm-monitor --lib PASS (198 tests). CI now checks otel.

## Task 2 — complete

Added approval-integrity regressions before changing executor behavior. Root integration target is compiling dependencies; tests will exercise denied shell preflight, shifted approval, and verification mismatch.

Task 2 evidence: three new integration regressions reproduced denied preflight execution, approval shifting and false-positive verification; all three passed after executor repair. TUI regression independently reproduced discarded denial before UI repair.
Ruling: pull F14 forward from Task 5 because live journal volume caused approval verification to panic — deterministic journal boundary test reproduced overflow at indices 127/128; minimal fix subtracts before narrowing. Remaining Task 5 work is unchanged.
Ruling: bind step occurrence to the full immutable plan SHA-256 and its ordinal, rather than adding random persisted IDs to every historical step — any plan edit/reordering invalidates all approvals; cost is requiring review again after harmless plan edits. Local approvals bind working directory and boot identity, expire after 15 minutes and are consumed by the executor.

Verification: cargo test --workspace --all-targets PASS (742 passing executions, 2 ignored); strict workspace all-target Clippy PASS. Added changed-review/consumed-approval regression separately PASS after the full run. TUI regression covers deny/approve, deny/approve-remaining, and approve/deny with positive and negative marker assertions. F14 deterministic boundary regression is green in the full run.

## Task 3 — complete

Scope: shared bounded subprocess runner, cancellation, enforce execution modes across CLI/TUI/web/rollback and remote mutation entry points. No production-readiness claim yet.

Cancellation regression reproduced a child surviving task abort, then passed after process-group cleanup. Shared runner now handles wall timeout, bounded output, partial diagnostics, cooperative cancellation, aborted futures and SSH/bootstrap/collector callers. CLI Ctrl+C drops active work; TUI pending apply has a cancellation guard. TUI previously discarded runtime policy flags; inner runtime now preserves policy. Rendering no longer launches journalctl. Collector total deadlines added; standalone process wrappers use the bounded runner.
Ruling: reject requested sandbox execution before launching host work until all operations can honor a real sandbox — follows spec fail-closed requirement; cost is sandbox functionality remains unavailable, rather than falsely claiming containment.
Limit: process-group cleanup covers ordinary descendants; malicious programs can escape groups. This is lifecycle control, not a substitute for an OS sandbox/cgroup security boundary. Read-only/dry-run guard target operations; application state/logging can still be written.

Verification: workspace all-target tests PASS (760 passing executions, 2 ignored), including policy guards in CLI dispatch/TUI/web/rollback, timeout/output/cancellation tests and journal boundary regression. Standalone CLI integration additionally PASS: restricted bootstrap/agent-install and requested sandbox launch no ssh/scp/bwrap. Strict Clippy and optional otel compile PASS; formatting PASS. A test run initially reproduced a stack overflow from large composed async state; boxing the runner fixed it, with a future-size regression and green full suite. Interactive editor and deliberate daemon launch remain separate lifecycle operations; target mutations use the shared policy.

## Task 4 — complete

Next: durable preparation before mutation, real encrypted backups with original file state, truthful rollback/partial failure, restart recovery and failure-injection tests. Existing rollback remains unsafe until this task is verified; no release readiness claim.

DurableJournal now commits pending steps and pre-change snapshot before execution, then encrypted file recovery material plus running status before mutation. Production CLI/TUI execution always attaches the journal. Database preparation failure prevents mutation; outcome failures retain recoverable interrupted evidence. A per-database process lock prevents concurrent executors. Existing free-text status columns and backup body support this additively without a SQL migration; the encrypted envelope is versioned.

Recovery preserves binary bytes, owner/group, mode and prior existence; uses sibling temporary files, atomic rename and fsync. AES-256-GCM backups bind change/step/backup/path/checksum metadata; the key is created atomically only in the designated secrets store. Missing keys and authentication failures stop restoration. Link targets, special files, extended attributes/ACLs, set-id files and files over 8 MiB are rejected rather than silently losing metadata. Active database and secret-store paths are protected.

Rollback restores only supported persisted file operations in reverse order, verifies results and reports partial failures. Arbitrary shell reversal was removed; unbacked operations require manual recovery. Already-restored backups are skipped on retry. Restart reconciliation marks unfinished work interrupted and never replays it automatically. Low-level library callers without a journal have memory-only evidence, explicitly documented; production entry points attach it.

An additional regression reproduced hidden mutation through verification commands. Verification now accepts bounded literal argv for a small read-only command set and rejects shell syntax/unsupported commands. General command previews default to unsupported rollback.

Verification: workspace all-target tests PASS (785 passing executions, 2 ignored); strict workspace all-target Clippy PASS after a type-alias lint repair; formatting PASS; optional otel build PASS. Failure-injection tests cover initial DB and backup persistence failure, final execution-status failure followed by restart restoration, final rollback-status failure followed by safe retry, tampered backup, missing key, newer user edits, repeated writes to one file, newly created files, binary contents, permissions and mixed shell/file partial rollback. Concurrent key creation returns one consistent key. Tests use temporary stores and harmless commands; real process-kill/power-loss and VM testing remain Task 14.

Limit: file comparison and rename do not provide a filesystem transaction against concurrent external writers or hostile directory replacement. Do not claim universal rollback or sandbox containment. Fork-identity grep found only the existing verification command quoted in PRD/TRD/ROADMAP.

## Task 5 — starting

Next: preserve collector failure/coverage and prevent false incident self-resolution. Database/web configured collector coverage remains to implement.

Task 5 first verified patch: reproduced nonzero collector exit being accepted as an empty healthy result. Shared runner now rejects unexpected exit statuses with redacted stderr/stdout diagnostics, preserving only documented SMART health flags, DNF check-update 100 and checkupdates 2. Service sub-probe and journal failures propagate; incomplete journal-permission/no-files diagnostics are errors. SMART failing-health and history/error bits now survive nonzero exits and emit critical/warning findings respectively. Recovery limits and key requirements are documented in troubleshooting.

Verification: workspace all-target tests PASS (789 passing executions, 2 ignored); strict workspace all-target Clippy PASS; formatting PASS. Graph refreshed (4,769 nodes / 11,491 edges). Tests cover original exit regression, scoped accepted statuses, SMART recorded output and finding severities. No additional live provider/VM calls.

Task 5 remains INCOMPLETE: explicit per-domain observation states; filtering/staleness and host-comparable evidence before self-resolution; failures still swallowed by other collectors; configured DB/web probes; actual container restart counts. Address these before checking off Task 5 or proceeding to Task 6. Existing self-resolution behavior remains a known risk until that repair.

External command semantics checked against primary sources: [smartctl source](https://www.smartmontools.org/static/doxygen/smartctl_8h_source.html), [DNF command reference](https://dnf.readthedocs.io/en/stable/command_ref.html), [Arch checkupdates manual](https://man.archlinux.org/man/checkupdates.8.en). Project behavior was verified through source and regressions.
