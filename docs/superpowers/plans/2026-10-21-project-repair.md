# HELM project repair and agent implementation plan

> **For agentic workers:** Use `superpowers:executing-plans` to implement task-by-task. Work sequentially by default to control usage; independent work is optional only when explicitly authorized. Checkboxes track execution, not promises of completion.

**Goal:** Ship a trustworthy Linux monitoring/remediation product plus a working general AI agent, with repeatable demonstrations.

**Architecture:** Preserve working collectors, models, provider adapters and SQLite stores. Repair the shared authorization/process/persistence boundary first, route CLI/TUI/web through it, then build the missing agent runtime on the same boundary. Extract only business logic needed to eliminate inconsistent enforcement; avoid a wholesale rewrite of the large UI files.

**Tech stack:** Rust workspace, Tokio, SQLite/rusqlite, existing provider clients, Ratatui, Axum, existing Linux/SSH/browser integrations.

**Spec:** [project-repair-spec](2026-10-21-project-repair-spec.md). Findings F01–F29 refer to [the verified audit](../../../HELM_CODEBASE_AUDIT.md).

## Global constraints

- Linux x86-64 is the initial binary support target.
- Repository/release identity remains `Jatin-Mali/helm`.
- Provider secrets persist only in `$XDG_CONFIG_HOME/helm/secrets.toml` or `~/.config/helm/secrets.toml`.
- Key precedence remains CLI override → secrets store → environment; startup does not auto-save environment keys.
- Preserve existing local changes and data. No release tag or destructive host demonstration during repair.
- Every security or state-changing fix includes a regression test that fails before the fix.
- Reuse dependencies first; justify any added cryptographic/process dependency against installed capabilities.
- Do not claim working sandboxing, recovery, providers, browser control or fleet behavior without exercising them.

## Review focus

- A plan changes after approval: refuse the changed command, target, host or working directory (Tasks 2–3).
- A child forks, emits excessive output or hangs during cancellation: terminate/reap descendants and retain bounded diagnostics (Task 3).
- The process crashes or disk fills between backup, mutation and status update: recover an interrupted state without claiming success (Task 4).
- A domain becomes unavailable or excluded after an active incident: keep its health unknown and do not resolve it (Task 5).
- Retrieved content contains secrets or permission-changing instructions: sanitize outbound data and prevent authority escalation (Tasks 7, 11).

## Delivery order and checkpoints

| Milestone | Tasks | Observable checkpoint |
|---|---|---|
| A: controlled execution | 1–4 | Denials hold; policies apply; approved reversible edits restore after restart |
| B: trustworthy operations | 5–9 | Honest monitoring, secure dashboard, real fleet and functional planning |
| C: general agent | 10–12 | Multi-step CLI/TUI tasks with tools, budgets, cancellation and evidence |
| D: demonstrable release | 13–14 | Installer, feature builds and fresh-VM demonstrations pass |

Do not postpone security fixes to make a demo look complete. Each task follows reproduce → minimal fix → focused regression → affected integration check → reviewable commit. Run the full suite at milestones and the final gate, rather than after every small edit. Record actual commands/results in a completion log; leave failed tasks unchecked.

### Task 1 — Pin the baseline and unblock build checks

**Owns:** `crates/helm-monitor/benches/helmops.rs`, `metric_history.rs`, `ring_buffer.rs`, `helm-cli/src/telemetry.rs`, tracing setup in `main.rs`, `.github/workflows/release.yml`. **Finding:** F29 plus existing fmt/Clippy failures.

- [x] Record current revision and test results; preserve the existing audit and graph artifacts.
- [x] Fix formatting and four known Clippy lints without changing monitoring behavior.
- [x] Adapt telemetry exporter and subscriber composition to installed OpenTelemetry APIs. Add CI compilation of `otel`; do not merely suppress compilation of the broken module.
- [x] Verify default and optional-feature compilation; commit these bounded build repairs.

```sh
rtk cargo fmt --check
rtk cargo clippy --workspace --all-targets -- -D warnings
rtk cargo check -p helm-cli --features otel
```

**Acceptance:** all three succeed; no telemetry endpoint is required to compile.

### Task 2 — Canonical plans and exact approvals

**Owns:** `crates/helm-monitor/src/{troubleshoot,execute}.rs`, `crates/helm-memory/src/changesets.rs`, `helm-cli/src/{main,tui,web}.rs`, `tests/v20_controlled_execution.rs`; new `tests/approval_integrity.rs`. **Findings:** F01–F03, F13, F18.

- [x] Convert the two isolated exploit probes into permanent tests. Add TUI decision-state tests for deny-first/approve-last and duplicate titles.
- [x] Add a versioned persisted executable-plan representation using existing `PlanStep`/`CommandPreview` models. Give each step a stable ID; approval records bind ID and canonical operation digest, host, cwd and plan revision. Denial is the default. Do not use callback index or title as identity.
- [x] Replace unsafe shell-based idempotency inference with literal argv for recognized trusted operations. Authorize before any plan-derived process launch; remove restart-is-active skipping. Checks needing execution have their own policy decision.
- [x] Make every producer explicitly convert into this schema and every loader return useful errors. Eliminate silent empty-plan defaults. Preserve TUI decisions through dispatch; reject altered approved plans.
- [x] Require successful exit **and** declared output/postconditions for verification. Return failed verification in the change outcome.

**Test contracts:** always-denied `systemctl start absent; printf ...` creates no marker; idempotent first step cannot lend approval to denied second step; approval of command A rejects command B; CLI/TUI/web generation round-trips to identical executable steps; invalid legacy JSON reports incompatibility.

```sh
rtk cargo test --test approval_integrity
rtk cargo test --test v20_controlled_execution
```

**Produces:** the one executable plan and approval representation consumed by Tasks 3–4 and the agent. Final field/signature changes must be made in all producers in this task.

### Task 3 — One execution and process boundary

**Owns:** new `crates/helm-monitor/src/{process_runner,execution_policy}.rs`, exports in `lib.rs`, `execute.rs`, `collectors/mod.rs`, CLI `ops.rs`, `sandbox.rs`, `main.rs`, `tui.rs`, `web.rs`; new `tests/execution_policy.rs`. **Findings:** F04, F15.

- [ ] Inventory every child-process launch, including hooks, remote commands and rollback; classify observation versus mutation using trusted operation definitions.
- [ ] Introduce shared execution policy and bounded process runner using existing types where possible: explicit argv or explicitly approved shell operation, cwd/environment policy, deadline, output limit, cancellation and process-group reaping.
- [ ] Enforce read-only/dry-run at the execution boundary across all frontends. A dry run returns a preview and creates no action child process. Deny arbitrary shell in read-only mode.
- [ ] Apply the actual sandbox launch wrapper to supported operations. If unavailable/incompatible, fail requested sandboxed execution before host launch; never print an enabled claim for unused settings.
- [ ] Migrate subprocess callers; preserve partial stdout/stderr and explicit timeout/cancel/output-limit results. Add total collector deadlines in addition to individual probe limits.

**Acceptance matrix:** CLI/TUI/web/service/rollback × denied/read-only/dry-run leaves markers absent. Forking child cannot survive cancellation. Large output remains bounded. Hung commands end within deadline plus a bounded kill/reap grace period. Sandbox test verifies the host filesystem boundary, not just a flag value.

### Task 4 — Durable changes and honest restoration

**Owns:** `execute.rs`, `helm-cli/src/ops.rs`, apply handlers in `main.rs`/`web.rs`, memory `changesets.rs`/`snapshots.rs`, new additive migration and `tests/change_recovery.rs`. **Findings:** F06, F07, F16.

- [ ] Create secure backups before mutation; persist bytes or durable artifact reference, checksum, original mode and whether the file existed. Fail closed if backup or pending-record persistence fails.
- [ ] Persist pending/running/applied/verified/failed/interrupted states, step results and snapshot references. Use transactions for related DB writes; propagate errors instead of printing saved unconditionally.
- [ ] Replace substring reversal of arbitrary shell with explicit supported inverse operations. Require confirmation for irreversible or unbacked actions; never invent a rollback command.
- [ ] Restore from persisted backups after process restart; verify content, mode and existence. Mark partial rollback honestly and return API failure when any required restoration fails.
- [ ] Add startup reconciliation for interrupted changes and ensure retry does not duplicate an already completed mutation without a new decision.

**Tests:** edit existing file, create new file, permission change, missing/corrupt backup, unwritable DB, terminated worker between stages, and mixed successful/failed rollback. Original state returns exactly for supported operations; every failure stays visible.

### Task 5 — Truthful monitoring and actual collector coverage

**Owns:** `collectors/{mod,logs,db_health,web_servers}.rs`, `engine.rs`, `snapshot.rs`, `reporter.rs`, relevant detectors, CLI/TUI/web rendering, `tests/v18_monitor.rs`; new `tests/monitoring_coverage.rs`. **Findings:** F12, F14.

- [ ] Preserve subprocess exit status/stderr; represent successful, failed, unsupported, filtered and stale observations explicitly per domain.
- [ ] Gate self-resolution on a fresh successful comparable observation of the same host/domain. Render unknown and coverage in both text and JSON.
- [ ] Wire DB/web collectors for explicitly configured targets using secret references, bounded safe probes and clear unsupported status. Do not discover credentials or probe arbitrary database endpoints automatically.
- [ ] Fix journal index arithmetic before narrowing. Review SMART exit bitmasks and container restart parsing with recorded fixtures so meaningful nonzero statuses are interpreted correctly.
- [ ] Test missing binaries, denied permissions, exit failure, timeout, filtered runs, disconnected collectors and indices crossing 127/128.

**Acceptance:** an unavailable collector cannot produce healthy status or a recovery event; an actually recovered service can. JSON exposes observation health even with zero findings.

### Task 6 — Consistent paths, real fleet and bounded history

**Owns:** CLI `paths.rs`, `main.rs`, `remote.rs`, `bootstrap.rs`, memory snapshot/change stores, daemon/watch loops; new `tests/runtime_paths.rs` and SSH fixtures. **Findings:** F11, F22, F26, F27.

- [ ] Resolve config/data/cache/database paths once and pass them through commands/services. An alternate DB must never cause default-DB writes.
- [ ] Reuse actual SSH collection for `monitor --fleet`; persist target host identity and observation timestamps. An unreachable host returns unknown with last-known data labeled stale.
- [ ] Send scripts over stdin to a fixed remote shell or quote the remote command exactly once. Test spaces, quotes and metacharacters using a shell fixture before two-VM SSH testing.
- [ ] Add bounded snapshot retention with protected references for active incidents and change restoration. Measure representative growth, choose documented defaults, then test cleanup and cleanup failures.
- [ ] Verify two remote hosts differ from each other and the local machine; interruption cannot silently substitute local results.

### Task 7 — Privacy, configuration and verifiable audit

**Owns:** core `secret.rs`, CLI `secrets.rs`, `service_form.rs`, config/logging/audit handlers, memory persistence boundaries, monitor prompt construction; new `tests/privacy_boundaries.rs` and additive audit migration. **Findings:** F08–F10, F17, F24.

- [ ] Extend redaction to resolved XDG protected paths and credential-bearing evidence. Apply at actual persistence/export/trace/provider boundaries while preserving exact executable payloads separately.
- [ ] Reject plaintext provider key fields through CLI and TUI config editing. Preserve unrelated security/telemetry configuration during provider changes. Service forms mask credentials and persist secret references.
- [ ] Reuse the stronger atomic locked CLI store semantics; remove retained historical plaintext values from the alternate core store and define an explicit compatible migration/deprecation path.
- [ ] Create private state directories/files with restrictive modes from the start; reject unsafe symlink/ownership cases without overwriting user files.
- [ ] Version the audit format with canonical encoding and cryptographic chaining. Use keyed integrity only with a real key-management design; label old FNV records legacy. State that same-user compromise can access a locally held key and that externally anchored audit is needed for stronger guarantees.

**Tests:** actual DB/log/export/prompt outputs contain no sentinel keys or protected paths; env keys never auto-save; provider switching preserves other config; concurrent writers do not lose secrets; changed audit fields fail verification; a pre-upgrade DB opens without silently discarding history.

### Task 8 — Authenticated web operations and pipelines

**Owns:** `helm-cli/src/web.rs`, `web/dashboard.html`, `crates/helm-monitor/src/pipeline.rs`, secrets integration; new `tests/web_security.rs` and pipeline tests. **Findings:** F05, F23.

- [ ] Add explicit local-client pairing/session authentication; protect snapshots, WebSockets and actions. Do not place reusable credentials in URLs, logs or HTML served to anonymous clients.
- [ ] Validate Host and Origin, enforce session-bound mutation authorization and CSRF protection where cookie sessions are used. Reject unauthorized WebSocket handshakes before streaming.
- [ ] Route service actions and rollback through Tasks 2–4. Existing global grants do not identify a web client.
- [ ] Verify webhook signatures against raw request bytes with stored secret references; enforce provider-specific freshness/deduplication. Unsupported providers remain disabled until their verification is implemented.
- [ ] Serialize pipeline updates with a shared transactional store or locked atomic writer; test concurrent pull/webhook events.

**Acceptance:** anonymous reads/actions fail, wrong-origin clients cannot connect or mutate, expired sessions fail, tampered/replayed webhooks fail, and authorized exact actions still work. Use a real browser for the dashboard positive/negative checks.

### Task 9 — Useful planning and a responsive TUI

**Owns:** CLI provider-readiness and plan handlers, `tui.rs`, `web.rs`, monitor `plan_cache.rs`; integration tests. **Findings:** F19–F21, F28.

- [ ] Reuse key resolution for direct requests and readiness: store-only/env-only/custom-env/Ollama/auth-free configured endpoints behave consistently without revealing keys.
- [ ] Separate ready/error/loading cache states; use atomic in-flight ownership, contextual freshness and an explicit retry action. Failed results must not block retry for 24 hours.
- [ ] Move provider/chat/catalog calls out of input handlers into cancellable bounded background jobs; deliver progress/results via events. Make rate-limit retry dispatch actual work.
- [ ] Use Unicode-safe truncation and test multibyte snapshot diffs.
- [ ] Prove generated plans survive persist/restart/apply through all frontends using the canonical schema, including provider error and malformed response cases.

**Acceptance:** TUI remains scrollable/cancellable while a mock provider hangs; simultaneous identical plan requests perform one generation; a failed request can be retried successfully.

### Task 10 — Restore the general agent runtime

**Owns:** new `crates/helm-agent/{Cargo.toml,src/lib.rs,src/runtime.rs,src/context.rs}`, workspace membership, core/provider integration; new `tests/agent_runtime.rs`.

- [ ] Build a bounded multi-step loop over existing `Message`, `ContentBlock` and `Provider` abstractions. Prefer native structured tool calls; normalize provider differences without executing ambiguous text.
- [ ] Model explicit running/waiting-for-approval/cancelled/failed/completed outcomes. Emit structured progress events for both CLI and TUI; do not couple runtime to rendering.
- [ ] Bound steps, context, request retries and wall time. Track actual provider usage and enforce a conservative budget before subsequent requests; surface unknown pricing rather than bypassing a hard configured limit.
- [ ] Preserve tool-call/result pairing while trimming context. Record evidence references for completion and failure; do not claim a tool ran when only prose was generated.
- [ ] Use scripted mock-provider transcripts to cover multiple tool turns, bad arguments, provider failures, cancellation, step/budget exhaustion and successful completion.

**Produces:** one agent runtime and event stream used by CLI/TUI in Task 12; all tool execution delegates to Task 11, never directly to a shell.

### Task 11 — Agent tools, permissions and injection resistance

**Owns:** new `crates/helm-agent/src/{tools,permissions}.rs`, existing execution policy/runner, core taint/capability models, browser adapter; new `tests/agent_security.rs`.

- [ ] Provide small schema-validated tools for filesystem read/search/write, shell, HTTP and browser interaction. Reuse installed backends; backend absence yields a clear unavailable result.
- [ ] Route mutations through exact Task 2 approvals and Task 3 policies. Constrain workspace paths, protected local state, HTTP destinations/redirects and browser sessions. File permission checks must withstand symlink/path traversal attempts at use time.
- [ ] Treat tool/web/file content as untrusted data. It cannot enable tools, expand scope, change grants, access secrets or replace user instructions. Require a user decision for new authority.
- [ ] Keep tool output bounded and redact/minimize content before sending it to providers. Protect config secrets, DB and logs from general file/shell exposure according to capability policy; broad shell authorization must disclose its broad OS authority.
- [ ] Test malicious project files/web pages, disguised writes, denied actions, redirects to prohibited destinations, symlink escapes, browser text trying to grant permission, and cancellation during execution.

**Acceptance:** the same denied marker tests pass through model tool calls; a real browser can open/read/navigate a test page under an approved session. Prompt wording is supplementary; the host-side policy decides access.

### Task 12 — Agent sessions and usable CLI/TUI workflows

**Owns:** CLI `main.rs`/`tui.rs`, new agent entry module as needed, existing memory session/episode APIs inspected before reuse, additive migration if required; new `tests/agent_workflows.rs`.

- [ ] Provide explicit `helm run <task>` behavior and connect the TUI assistant to the runtime event stream. Keep chat, monitoring and execution states distinguishable.
- [ ] Persist redacted conversation, tool outcomes, approvals and terminal outcome; implement resume with fresh authorization for pending mutations. Do not replay tool effects on restart.
- [ ] Support cancellation, progress, failure explanation and a final evidence-based summary. Remove stale UI actions that still invoke deleted runtimes or advertise unavailable tools.
- [ ] Demonstrate a multi-step temporary-project edit/test task and a Linux diagnostic task. One real provider is mandatory for release; test every provider's tool-call shape with fixtures and label live coverage accurately.
- [ ] Verify existing monitor/remediation commands still work and both `helm`/`helmops` accept the supported agent command.

### Task 13 — Installer and public feature completeness

**Owns:** `install.sh`, `.github/workflows/release.yml`, new installer tests/CI checks, user docs and current navigation index. **Finding:** F25; completes additional feature gaps.

- [ ] Check OS/architecture, download and validate the checksum, stage safely, and leave the old binary intact on mismatch/failure. Publish only supported targets; retain clear source-build fallback.
- [ ] Add mock endpoint alert tests for Slack/PagerDuty/webhook success, rejection, rate limits, deduplication and recovery. Exercise daemon startup/stop/restart and stale heartbeat handling.
- [ ] Add optional-feature builds and integration regressions to CI. Run a dependency-advisory scan with current data and triage actual affected dependencies rather than claiming cleanliness from compilation.
- [ ] Update install/setup/security/agent/rollback docs from tested commands. Correct the stale crate/command index while preserving unrelated local edits. List unsupported operations and required external binaries plainly.
- [ ] Verify required release assets and `contents: write`; do not publish a release as part of writing or implementing this plan without the release gate.

### Task 14 — Fresh-machine acceptance and demo package

**Owns:** new `tests/demo/` fixtures, `scripts/verify-demo.sh`, `docs/demo.md`, completion/evidence log. Depends on all prior tasks.

- [ ] Create a disposable VM scenario with isolated XDG state, temporary project, harmless test service and two SSH targets. Make setup/teardown repeatable; fixtures must never target real user services.
- [ ] Run both demonstrations in the spec with a real provider/browser where required. Capture actual commands, host identities, before/after checks, cancellation and restoration outcomes without secrets.
- [ ] Add cross-frontend acceptance for denial, read-only, unsupported sandbox, failed verification and failed rollback. Verify CLI exit codes, JSON states and UI labels agree.
- [ ] Perform DB upgrade/backup/recovery and a prolonged monitoring run; measure growth, latency and memory against documented target hardware and retention settings.
- [ ] Run the release gate below, inspect the final diff and regenerate the source graph with `graphify update .`. Close findings only with linked tests/evidence; create a short reproducible demonstration guide and supported-feature matrix.

```sh
rtk cargo fmt --check
rtk cargo clippy --workspace --all-targets -- -D warnings
rtk cargo test --workspace --all-targets
rtk cargo check -p helm-cli --features otel
rtk cargo build --release -p helm-cli
rtk proxy bash scripts/verify-demo.sh
```

The verification script must fail on broken assertions, use temporary state, and distinguish optional unavailable integrations from required checks. Skipping a required real-provider/browser/fleet test leaves the corresponding capability unverified and blocks its working-demo claim.

## Audit coverage index

| Finding | Task(s) | Finding | Task(s) | Finding | Task(s) |
|---|---|---|---|---|---|
| F01 | 2, 3 | F11 | 6 | F21 | 9, 12 |
| F02 | 2 | F12 | 5 | F22 | 6 |
| F03 | 2 | F13 | 2, 4 | F23 | 8 |
| F04 | 3 | F14 | 5 | F24 | 7 |
| F05 | 8 | F15 | 3 | F25 | 13 |
| F06 | 4 | F16 | 4 | F26 | 6 |
| F07 | 4 | F17 | 7, 11 | F27 | 6 |
| F08 | 7 | F18 | 2, 9 | F28 | 9 |
| F09 | 7 | F19 | 9 | F29 | 1 |
| F10 | 7 | F20 | 9 | General agent | 10–12 |

## Completion rules

“Fixed” means a regression fails on the old behavior and passes on the repaired path. “Working” means the integrated workflow ran, with failure/denial cases checked. “Demo-ready” additionally means installation and both demonstrations pass on a fresh supported host. A tag requires the project's release gate; no passing mock suite substitutes for that gate.

Implement Milestone A first, then B, C and D. There is no credible fixed completion date until the shared execution/persistence repairs establish their actual migration scope. Track milestones and evidence rather than consuming usage on repeated broad audits or speculative features.
