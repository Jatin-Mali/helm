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

Task 5 observation/recovery patch: persisted additive CollectionCoverage inside domains JSON, with hashed machine/boot/hostname identity and explicit successful/failed/unsupported/filtered/stale/unknown status. Legacy records deserialize with unknown coverage; five-minute freshness bounds and error precedence prevent stale or failed data being labeled healthy. Shared task-scoped probe tracking catches failures that individual collectors swallow, with bounded redacted diagnostic accumulation and concurrent attribution regression. Unsupported runtime flags are recorded; incomplete backup/cron/load/firewall/pipeline evidence is explicitly unknown, not successful.

Reporter regressions reproduced false self-resolution on failed/filtered collection before repair. Resolution now requires comparable fresh successful observations, same host/boot/profile, selected domain and an enabled detector; cross-domain DB/web/monitoring checks require ports plus services. Custom/configuration checks stay unresolved without independent coverage. Unverified incidents retain last-known evidence; reporter host switches cannot migrate and later close another host's incidents. Resolved events clear obsolete uncertainty annotations and are excluded from active severity counts.

CLI JSON compatibility change: schema_version 1 report object contains findings plus coverage, identity, time and collector errors; former bare-array consumers must read .findings. Text/Markdown expose coverage. TUI/web use explicit status rather than absence-of-error as healthy. Browser empty state says health unknown with missing coverage.

Verification: workspace all-target tests PASS (797 passing executions, 2 ignored) before final host-state refinements; latest monitor library tests PASS (219), with seven focused reporter regressions PASS after the final output-label change. Strict all-target Clippy and fmt passed; final incremental check is recorded separately below. Desktop 1440x1000 and mobile 390x844 isolated static browser fixtures verified unknown empty-state text and dark background/contrast tokens; no live API/provider/DB connections were exercised. Static fixture API/WebSocket errors were expected. Screenshots remain under /tmp, not committed. Impeccable contrast warnings fixed through stronger secondary text tokens, explicit dark background and placeholder styling; no ignores added. Existing decorative-grid advisory remains for the later UI phase.

Task 5 STILL INCOMPLETE. Next priorities: strict collector parsing and real container restart counts; complete coverage evidence for remaining unknown collectors; remove implicit default DB probing and wire explicitly configured DB/web targets using secret references. Do not advance to Task 6 or claim production readiness yet. Graph updated: 4,797 nodes / 11,591 edges.

Final incremental verification for observation/recovery patch: strict workspace all-target Clippy PASS and cargo fmt --check PASS after final source edits; focused seven reporter tests PASS. Browser placeholder and dark-background computed-token assertions PASS; temporary browser and static HTTP server were closed.

## Task 5 — complete (resumed by Claude)

Container restart counts now come from `docker|podman inspect` (`.RestartCount`); `ps` status "Restarting (N)" is an exit code and is no longer parsed as a count (unknown when inspect fails). Implicit `psql -U postgres` / `mysql` probing removed from `DbHealthCollector`; explicit DB/web targets are the registered services (TCP/HTTP health checks, no credential discovery). Regression: `collector_never_probes_databases_implicitly`, `inspect_counts_apply_by_id_and_malformed_lines_stay_unknown`.
Ruling: backups/timers/load/firewall/pipelines stay `Unknown` coverage (never resolve incidents) rather than fabricating a success signal — conservative; cost: incidents in those domains need manual resolution.
Verification: helm-monitor lib 221 PASS, strict clippy PASS.

## Task 6 — complete (Claude)

- F22: `--db-path` stored once via `paths::set_db_override`; `default_db_path()` (11 callers) honors it. `helm-cli/tests/runtime_paths.rs` RED on old code (stray write to default DB), GREEN now.
- F11: `monitor --fleet` rewritten (`helm-cli/src/fleet.rs`) to use real SSH `collect_report`; per-host identity (hashed machine/boot id), local `observed_at`, unavailable hosts print UNKNOWN with last-known data labeled STALE; state persisted atomically in `fleet-state.json`. Stub-ssh end-to-end test `helm-cli/tests/fleet.rs`.
- Found while wiring: remote collection parsed `Vec<Finding>` but `monitor --format json` now emits the versioned report object (Task 5 change) — remote monitoring was silently broken. Parser accepts both; a report without `findings` is an error.
- Fleet per-host budget was 2s (every real host would time out); now 60s overall, 5s SSH connect, remote runs use `--profile quick`.
- F27: `remote_command()` quotes every remote argument once (only a leading `$HOME/` expands); used by `run_shell`, remote helm calls and bootstrap. Tests run hostile arguments through a real shell.
- F26: `SnapshotStore::prune` (7 days, 1000 rows; measured ~135 KB/snapshot ⇒ ≈135 MB cap); newest 2 and rows referenced by changes/plans/unresolved findings are protected. `persist_monitor_snapshot` now returns errors to daemon/monitor/TUI/snapshot callers; prune failure is logged.
Ruling: fleet state keyed by remote name, not host_id — legacy remotes.toml entries receive a fresh random host_id each load. Cost: renaming a remote forgets its stale data.
Ruling: remote-vs-local identity equality is not checked at runtime; the demo (Task 14) compares host identities. Cost: a remote that is actually this machine is only flagged if two remotes share it.
Verification: workspace all-target tests 818 passed / 0 failed; strict clippy and fmt PASS.

## Task 7 — complete (Claude)

- F09/F17: `redact_secrets` extended (XDG/legacy protected paths incl. wal/shm/lock/log, bearer/basic, URL passwords, quoted or bare `password=`-style values, PEM, GitHub/Slack/AWS/JWT shapes; counters like `max_tokens` untouched). Applied at actual boundaries: `SnapshotStore::insert`, `write_audit_event`, runbook output, `build_narrative_prompt` (provider-bound). `tests/privacy_boundaries.rs` reads the real DB rows / prompt.
- F10: `update_config_file` is the single editor for CLI `config set` and TUI `/config`: rejects credential-looking keys/values, refuses to overwrite unparseable TOML, atomic 0600 write. `write_helm_config` updates only provider identity and keeps security/telemetry/custom sections. `config validate` flags credentials already on disk.
- F24: `helm_core::private_fs` (0700 dirs created, 0600 files from creation, O_NOFOLLOW, owner check) used by `MemoryStore::open`, audit shards, log writer and state dirs. Existing directories are never chmod-ed. Core `SecretStore`: atomic 0600 write, retired values no longer retained, legacy history values scrubbed on next write; marked deprecated for provider keys (CLI `secrets.toml` store remains the home).
- Service registry refuses new DB connection strings with embedded passwords; legacy entries are redacted in findings/forms.
- F08: audit hash v2 = SHA-256 over domain tag + length-prefixed fields (`v2:` prefix). Legacy FNV rows verify and are counted; legacy-after-v2 is rejected as downgrade; `audit verify` prints heads for external anchoring. Pre-upgrade DB test passes. Docs corrected (they claimed HMAC).
Ruling: no keyed/HMAC audit — a key readable by the same user adds no protection against the stated same-user threat and key management is out of scope; head hashes + documentation instead. Cost: no tamper evidence against a same-user attacker without off-host anchoring.
Ruling: provider-key values pasted into `config set` are blocked by pattern, but arbitrary unmarked secrets are not detectable; documented in docs/data-boundary.md.

## Task 8 — PARTIAL (Claude; usage limit reached)

Done: webhook ingress (F23). `webhook_auth::verify` checks raw-body signatures (GitHub HMAC-SHA256 verified against GitHub's published vector; GitLab token; generic timestamped HMAC); Jenkins/others disabled. Per-service secret `HELM_WEBHOOK_SECRET_<SERVICE_ID>` in the secrets store, none ⇒ 503. Replay (delivery id, 7-day memory) and stale-run rejection; pipeline history updates are now one flock-serialized read-merge-write with unique temp files (concurrent test: 16 writers, no loss). Tests: webhook_auth (5), pipeline replay/stale/concurrency.
NOT done (still required by Task 8): dashboard pairing/session authentication for `/`-data routes, `/api/*` and `/ws`; Host/Origin validation; CSRF; session-bound per-action approval for service actions and rollback via `PlanApprovals` + `DurableJournal` (design: prepare → approve → execute, single-use, 15 min, bound to session/host/cwd); updating `web/dashboard.html`; real-browser positive/negative checks; `tests/web_security.rs`. Until then the dashboard remains unauthenticated on loopback (F05 open) — do not expose it.

## Remaining tasks (not started)
9 planning/TUI; 10 agent runtime; 11 agent tools/permissions; 12 sessions/workflows; 13 distribution/installer/CI/docs; 14 real demonstrations (needs real provider, browser, two SSH hosts, VM). Real-provider keys exist in the environment (GROQ/GEMINI/OPENROUTER/DEEPSEEK) but were not used.


## Task 8 — complete (Codex continuation)

Dashboard remains loopback-only. A 256-bit terminal pairing code creates one HttpOnly/SameSite=Strict process-local session; both pairing and session expire after 15 minutes. Restart invalidates all sessions. Anonymous API/snapshot/WebSocket requests are rejected. Exact Host and same-origin validation cover requests; mutations require Origin and session-bound CSRF; WebSockets require Origin and terminate at session expiry. Responses disable caching/framing and external script loading. Anonymous HTML contains no credential or host observation.

Service stop/restart now prepare a review, then consume a single-use, session/operation/host/cwd-bound PlanApprovals review through ExecutionEngine + DurableJournal. Rollback reviews bind the full persisted change-set digest and use the existing authenticated backup restoration. Changed, cross-session, expired and consumed reviews fail. Global grants no longer authorize web mutations. UI masks and clears pairing input, authenticates before streaming, and confirms exact prepared operations.

Verification: full workspace all-target suite PASS (852 passed, 0 failed, 2 ignored); strict workspace all-target clippy PASS; fmt PASS. Expanded HTTP regression separately PASS after the suite: fake systemctl marker remains absent before approval and on changed/replayed approval, appears only for the approved exact operation, and produces durable change evidence. Auth/review unit tests cover expiry, CSRF, cross-session and tampered review. Real Chromium via Playwright: anonymous pairing form; wrong code rejected; correct code closes login and connects WebSocket; authenticated session 200; missing-CSRF POST 403; authorized read-only mutation returns explicit denial. No real service was changed. Browser/server used isolated /tmp state. Graph updated (4969 nodes / 11995 edges); SQL extraction still unavailable (tree_sitter_sql missing), source graph remains partial for SQL.

Ruling: one terminal pairing per process, restart to pair again after expiry — the smallest explicit local-client authentication flow; costs restarting the console every 15 minutes. Webhook authentication remains separate from browser sessions. Real disposable-VM operations/fleet demonstrations remain Task 14; this is not a production-readiness claim.

## Tasks 9–11 — complete (Codex wrote, Claude verified and fixed)

Found uncommitted after Codex's limit: helm-agent crate (runtime, tools, permissions, context; 12 runtime + 5 security tests), store-aware readiness (`llm_provider_usable`), `PlanCache` single-owner generation with retry (`concurrent_generation_has_one_owner`, `failed_generation_can_retry`), non-blocking cancellable provider jobs in the TUI (`TuiApp` Drop aborts tasks), Unicode-safe truncation, installer/release/alert/daemon-heartbeat work (Task 13 partial). Build was broken (two TUI tests moved out of a `Drop` type), one runtime test was timing-flaky (20ms cancel vs 100ms wall) and two TUI tests leaked the host's real GROQ key; daemon stale-heartbeat test was RED (feature missing) — implemented `effective_daemon_status` (3 missed intervals, min 30s). Added `persisted_plan_states_survive_restart_and_failures_stay_retryable` (ready/timeout/auth/malformed/rate-limit round-trip through the plan table).
Ruling: browser tool is an isolated static-DOM reader (no JS, no network). The "real browser opens/reads/navigates a test page" acceptance is carried to Task 14. Cost: JS-rendered pages are unreadable to the agent.
Verification: workspace all-target tests 909 passed / 0 failed; strict clippy and fmt PASS. Committed db39d3b, d501ac6, a138fdb.

## Task 12 — complete except live-provider demonstrations (Claude)

`helm run <task>` (both binaries): `agent_cli.rs` builds provider + `HostTools` (workspace, `ExecutionPolicy`, `DurableJournal`, protected db/secrets/config/sessions paths) and drives `AgentRuntime`; `--resume`, `--sessions`, `--json`, `--max-steps`, `--timeout-secs`, `--budget-usd` (requires explicit prices; HELM does not guess costs). Approvals are interactive and single-use per exact call; without a terminal the run stops `waiting_for_approval`, nothing executes, exit 1. `--yes` is not honored by `run`. Ctrl-C cancels through the runtime token (session saved as `cancelled`); a second Ctrl-C terminates.
Sessions (`agent_session.rs`): one 0600 JSON file per session under `<data>/agent-sessions`, redacted transcript/tool log/approvals/evidence/usage; ids are validated against path escape. Resume drops unanswered tool calls from history, so nothing replays and every mutation needs fresh approval.
TUI: the assistant now runs the same runtime (events → status/tool timeline; `AgentApproval` modal for exact calls; cancel via token; follow-ups keep context; session id shown). Removed stale actions: fake "pending" permission modal (`a` key could grant ShellShell *always*), `helm replay` hint, `wrap_for_remote` (advertised ssh tools that no longer exist). Dashboard service stop/restart and rollback now use an exact-review modal and execute through `PlanApprovals` + `ExecutionEngine` + `DurableJournal` instead of a global capability grant plus a bare `systemctl`.
Ruling: sessions are JSON files, not DB tables — keeps the audit/episode schema untouched (CLAUDE.md invariant) and avoids a migration. Cost: no SQL queries over sessions.
Ruling: agent tasks are local-only; with a remote selected the TUI/CLI refuse rather than act locally under a remote label. Cost: no remote agent work until a remote tool exists.
Ruling: pricing is never inferred; a hard budget without prices is a usage error.
Tests: `helm-cli/tests/agent_workflows.rs` (scripted OpenAI-compatible server: read/evidence/redaction/0600, no-terminal pending + resume-without-replay, pty approve/deny, SIGINT cancel, both binaries) and 5 TUI tests (tool run + evidence, approve/deny, cancel, reviewed service action with fake systemctl + change-set evidence). Provider tool-call fixtures exist for anthropic/gemini/ollama/openai-compat; none claim live coverage.
Not yet done (Task 14): multi-step temp-project and Linux diagnostic demonstrations against a real provider.
Verification: workspace all-target tests PASS, strict clippy and fmt PASS.

## Task 13 — complete (Codex wrote installer/CI/alert/daemon pieces; Claude verified and finished)

Installer: Linux x86-64 only; asset + `.sha256` fetched over HTTPS, strict checksum parse, staged on the destination filesystem, atomic replace only after verification, old binary untouched on any failure (`scripts/test-install.sh`: mismatch, missing/malformed checksum, OS, arch, success, alias — PASS, also run in CI). Release workflow publishes `install.sh`, `helm-x86_64-unknown-linux-gnu` and `.sha256` with `contents: write`; CI adds `--all-features`, `--no-default-features` and a `cargo audit --deny warnings` job. Alert sinks have mock-endpoint tests for delivery, dedup, recovery, rejection, rate limits and retry. Daemon start/stop/restart and stale-heartbeat tests added (`stale` was missing; implemented).
Advisory scan (run with cargo-audit 0.22.2 against the current database): 13 vulnerabilities found in transitive crates (rustls, rustls-webpki, webpki-roots, quinn-proto ×5, h2, crossbeam-epoch) — all cleared by `cargo update` (Cargo.lock is gitignored, so fresh builds already resolve patched versions); 3 warnings (lru ×2 unsound, paste unmaintained) both reached only through ratatui 0.29 — cleared by upgrading ratatui to 0.30 (no source changes needed). `cargo audit --deny warnings` now PASS.
Ruling: `rust-version` raised 1.85 → 1.89 — the resolved dependency set already requires 1.89 (uuid 1.29), so 1.85 was false. Raising it enabled the `collapsible_if` lint; applied mechanically with `cargo clippy --fix` (let-chains), tests green. Cost: toolchains older than 1.89 are unsupported.
Docs from tested commands: README rewritten, new `docs/agent.md`, `docs/agent-on-remote.md` rewritten (the NDJSON agent-on-remote transport it described no longer exists), AGENTS.md index rebuilt to the real crates/commands/tests (the user's uncommitted graphify section was left unstaged).
Also fixed: security review finding (terminal control/bidi characters in approval prompts — `sanitize_for_terminal`, used by CLI prompt, progress lines and TUI modal); agent sessions now live beside the database (a test run had written session files into the repository; removed from the index); `--remote` with `run` is refused and the global flag's help no longer promises remote tool execution.
Verification: workspace all-target tests PASS, strict clippy PASS, fmt PASS, `cargo check --workspace --all-targets --all-features` and `--no-default-features` PASS (before ratatui bump; re-run in the Task 14 release gate).
