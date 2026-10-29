# HELM / HELMOPS: source-based project audit

Audit date: 2026-10-21. Baseline commit: `fbbbec3631b23965f692893e0a35b24e43f9f680`.

## Verdict

HELMOPS has a substantial working Linux monitoring foundation, typed findings, persistence, fleet collection, provider adapters, and two dashboards. **Its current mutation paths should not be treated as safely controlled execution.** Approval decisions can be bypassed, advertised containment flags do not protect those paths, and rollback does not reliably restore previous state. Monitoring also has failure modes that confuse missing evidence with recovery.

The strongest evidence is not a documentation discrepancy: isolated probes against the compiled library reproduced execution of a denied command and approval being applied to the wrong step. The existing test suite still passed. This makes missing negative tests and trust-boundary enforcement the first priorities.

This is a report, not a remediation patch. Production code was not changed.

## Scope, method, and limits

The workspace manifest, source, migrations, installer, workflow, source tests, and a newly generated graph were the evidence sources. Existing README, roadmap, PRD, TRD, and generated graph prose were not accepted as implementation evidence. The supplied navigation index was useful only for orientation; several described crates and commands no longer exist.

The graph was built first with `graphify extract . --code-only --cargo --max-workers 4`: **151 code-classified inputs, 3,426 nodes, 9,886 edges, 127 communities**. This classification includes build/configuration inputs and is not a count of Rust files. The working source tree has 116 Rust files across `crates`, `helm-cli/src`, and `tests`; tracked Rust source totals 56,736 lines including tests and the empty root host. There are six Cargo packages including the root integration-test host, and five explicit workspace members.

Graph integrity diagnostics found no dangling endpoints, missing endpoints, self-loops, or remaining same-endpoint collapse in the exported graph. This does not prove the extractor captured every relationship. The graph labels 97 edges INFERRED; examples include implausible production-to-test call directions. Those edges were not accepted as call-graph proof. SQL extraction reported a missing `tree_sitter_sql` dependency; migration behavior was inspected from SQL and persistence code rather than inferred from graph nodes. Documentation was deliberately excluded. Community names are based on source-file membership, not LLM semantic extraction. Graph extraction used zero semantic LLM tokens.

Review effort concentrated on input → authority → execution → verification → persistence, and on collection failures → findings → presentation. **I did not complete a line-by-line reading of every source/test/rendering line.** Broad mapping and targeted implementation review are complete; exhaustive manual review is not. The findings below state concrete evidence and do not claim an exhaustive vulnerability inventory. In particular, source-only leads from parallel review were rechecked where included below; unfinished worker conclusions were not treated as complete audits.

No real provider credentials were read, no paid provider requests were made, and no real service stop/restart was used for reproduction. SSH deployments, hardware SMART failures, Kubernetes clusters, backup restores, and browser-specific cross-origin attacks were not exercised. No current dependency-advisory scan was completed; no claim of dependency/CVE cleanliness is made.

Source shorthand used below: [main.rs](helm-cli/src/main.rs), [tui.rs](helm-cli/src/tui.rs), [web.rs](helm-cli/src/web.rs), [ops.rs](helm-cli/src/ops.rs), [execute.rs](crates/helm-monitor/src/execute.rs), [reporter.rs](crates/helm-monitor/src/reporter.rs), [troubleshoot.rs](crates/helm-monitor/src/troubleshoot.rs), and [pipeline.rs](crates/helm-monitor/src/pipeline.rs). Graph artifacts: [interactive graph](graphify-out/graph.html), [raw graph](graphify-out/graph.json), and [graph report](graphify-out/GRAPH_REPORT.md).

## What actually exists

| Component | Implemented responsibilities | Important boundary |
|---|---|---|
| Root `helm-workspace` | Integration-test host and shared dependencies | Not an application service |
| `helm-core` | Capabilities, grant scopes, taint labels, chat messages, errors, secret wrapper, redactor, validators | Types do not enforce policy unless callers invoke checks |
| `helm-providers` | Anthropic, Gemini, Ollama, OpenAI-compatible adapters; presets for Groq/OpenRouter/NVIDIA/OpenAI; mock provider; quirks and pricing tables | Four production protocol implementations support several provider choices |
| `helm-memory` | SQLite migrations, historical episode APIs, grants, audit chains/shards, snapshots, finding state, troubleshooting plans, change sets/backups | Separate APIs and direct connections do not share one universal transaction/policy boundary |
| `helm-monitor` | System collection, detectors, custom rules/services, pipeline history, correlation, reports, plan parsing/cache, change execution | Execution is implemented here, not in an agent/tool registry |
| `helm-cli` | `helmops` binary and `helm` compatibility binary, setup/config/secrets, monitoring/daemon/incidents/plans/changes, TUI, Axum dashboard, remotes/bootstrap | Both binaries compile the same `main.rs` |

The current Cargo workspace contains **no `helm-agent` or `helm-tools` members**. There is no live ReAct loop, dynamic agent tool registry, model router/fallback agent, skills-learning runtime, or agent RBAC system matching the historical index. Core taint/capability/provider/episode abstractions remain, but their existence must not be confused with an active enforcement layer. `agent` commands manage remote installed agents; they are not model-driven subagents.

### Current command/product surface

`main.rs:127` defines models, doctor, permissions, audit, secrets, init, config, service registration, completion, tui, remote, agent, bootstrap, trust-report, dashboard, daemon, incident, snapshot, monitor, troubleshoot, explain, apply-plan, change-set, backup, runbook, alerting, and web.

The default invocation opens the web server on `127.0.0.1:9120` (`main.rs:1026`; `web.rs:204`). `dashboard` also starts the web server. `tui` starts the terminal operations interface. Several older argument structs and TUI commands survive even though their underlying runtimes were removed. Do not use their presence as proof of supported behavior.

### Collection and detection

`engine.rs:68` launches collectors concurrently with `tokio::join!`. It collects host identity, load/memory, disks, services, containers, listeners, logs, backups, packages, timers, networking, processes, firewall, Kubernetes, libvirt, Compose, certificates, and pipelines. Results populate `SystemSnapshot`; failed collectors generally become default-valued domains plus `collector_errors`.

Collectors use Linux files and commands such as `/proc`, systemctl/journalctl, df/findmnt/lsblk, container CLIs, kubectl, virsh, package tools, and certificate/network probes. They are heuristic observations with prerequisite and privilege requirements, not authoritative cross-platform inventory.

`DetectorRegistry` applies domain filters and thresholds, normalizes findings, validates them for emission, and sorts them (`detectors/mod.rs:79`). Detector modules cover disk/inodes/read-only filesystems, load/memory/swap/OOM, services/restarts, container/Compose problems, packages/security, exposed ports/SSH/DNS, backup schedules/age/restore evidence, certificates, Kubernetes restarts/PVCs, VM state/snapshot age, monitoring health, and database/web symptoms. Dedicated DB and web collector modules exist but are not wired into `engine.rs` collection; their presence overstates native collection coverage.

`MonitorReporter` adds custom-rule findings, checks registered services, adds pipeline failures, correlates findings, and optionally emits self-resolved lifecycle records (`reporter.rs:82`). Finding identity/fingerprints support persistent user state and incident selection. Correlation is rule/history based; it suggests relationships rather than proving a root cause.

### Daemon, state, and dashboards

The daemon performs repeated collection, stores snapshots, routes alerts, optionally queues LLM plans, and writes a heartbeat/state JSON file with metric history (`main.rs:4512`). It is a locally spawned process managed through PID/state files, not a complete supervised service lifecycle by itself.

The Ratatui TUI contains operations tabs, finding selection/state changes, evidence, plan generation, provider/model/key modals, service actions, change rollback, and fleet display. It combines presentation and substantial persistence/business logic in one 14,974-line source file.

The Axum server exposes `/`, `/ws`, `/api/snapshot`, `/api/refresh`, `/api/plan`, `/api/service-action`, `/api/change-rollback`, and `/api/pipelines/webhook/{provider}/{service_id}` (`web.rs:239`). Background collection/presentation broadcasts snapshots. There is no authentication/session layer around these routes.

### Diagnosis, AI plans, and execution

Deterministic troubleshooting builds hypotheses, read-only checks, suggested changes, and command previews (`troubleshoot.rs`). LLM plans use finding title, severity, affected resource, detector, impact, and evidence summaries in a text prompt (`troubleshoot.rs:481`). Provider responses are parsed into narrative and steps, checked for required fields and dangerous command patterns, then cached/persisted.

Plans flow through multiple representations: `LlmFixStep`, TUI `ValidatedFixStep`, `PlanStep`, JSON persistence, then `CommandPreview`. These representations are not fully compatible. CLI `apply-plan` loads persisted JSON, prompts for decisions, constructs a callback, and invokes `ExecutionEngine` (`main.rs:5995`). The engine captures before/after snapshots, performs idempotency checks, executes shell/file actions, runs verification commands, and returns a change set (`execute.rs:479`).

The text assistant is now a single provider chat call with no tools (`tui.rs:3784`). It does not execute requested diagnostic checks, maintain a ReAct action loop, or enforce historical agent policies. Prompts such as “run this exact follow-up” therefore request prose from a model rather than run the check.

### Persistence and secrets

The main CLI secret store is flat TOML under XDG config, with 0600 files, 0700 parent directories, advisory writer locking, and sibling temporary-file replacement (`helm-cli/src/secrets.rs:44`). Normal secret lookup correctly follows CLI override → store → environment (`secrets.rs:302`). Startup does not silently import environment keys; importing is an explicit secrets command.

There is a second, public JSON-backed `helm_core::SecretStore` with rotation history. It is not the CLI's TOML store and has weaker write semantics. Secrets rotation in that library does not establish rotation support in the current CLI.

SQLite stores snapshots, findings/state, plans, changes, grants and historical episode/audit records. Migrations extend the schema through version 16 even though migration filenames use a different sequence. Hash chaining exists, but the persistence chain is unkeyed FNV-1a, not HMAC. Some monitoring paths redact at the caller, while store methods accept raw strings.

### Fleet, alerts, and distribution

Remote registry and SSH helpers support host IDs, key files, connection settings, installed-agent metadata, parallel report collection, and last-known fleet state. TUI/web fleet views use remote collection. The separate `monitor --fleet` command is a local-host placeholder (finding F11).

Alert sinks include generic webhook, Slack and PagerDuty with severity/dedup/rate policies. Pipeline integrations parse pull/webhook data and retain recent history in a JSON file. These are outbound integrations, not proof of authenticated inbound webhook handling.

The release workflow formats/lints/tests and publishes an x86-64 Linux binary, SHA-256 file, and installer with `contents: write`. The installer selects x86-64 or ARM64, downloads an asset, creates `helmops`/`helm` names, and prints source-build guidance when assets are unavailable. ARM64 is selected by the installer but not built by this workflow.

## Findings: security and execution integrity

Severity reflects the demonstrated reachability. “High” below is not a claim that an arbitrary internet client can exploit a loopback-only service. A malicious plan must reach a local apply action unless another access path is explicitly described.

### F01 — HIGH: denied commands can run before approval — reproduced

**Evidence:** `crates/helm-monitor/src/execute.rs:306` constructs shell idempotency probes by interpolating a service suffix into `sh -c`. `execute.rs:506` runs that probe before `execute.rs:519` invokes the approval callback.

A preview starting with `systemctl start nonexistent; <payload>` causes `<payload>` to execute in the “is-active” check. An isolated compiled-library probe used an always-deny callback, stubbed systemctl, and a temporary marker write. The marker was created even though the returned step status was `skipped`.

**Impact:** code runs under HELMOPS's OS identity despite explicit denial. It also escapes execution/audit accounting because the step is treated as skipped.

**Fix:** authorize before any plan-derived process execution. Replace shell interpolation with an exact parsed operation and literal argv. Until parsing is trustworthy, remove this idempotency heuristic. Add a denied malicious service-name regression test.

### F02 — HIGH: CLI approval decisions move to the wrong step — reproduced

**Evidence:** `main.rs:6140` creates an indexed approval callback; `execute.rs:506` skips idempotent steps without calling it.

With decisions `[approve first, deny second]`, an idempotent first step consumes no decision. The second step receives the first approval. A compiled-library reproduction using the same callback pattern created a marker for that denied second step.

**Fix:** bind authorization to immutable step ID plus command/input hash, never callback position. Test idempotent, denied, failed, and duplicate-title combinations.

### F03 — HIGH: TUI per-step denials are discarded

**Evidence:** `tui.rs:3566–3634` handles approval/skip by advancing `step_index` but saves no decision list. `tui.rs:5142` eventually calls CLI execution with `yes: true`.

Skipping an earlier step and approving the last step executes the entire stored plan with auto-approval. The display and audit can say a step was denied while execution later approves it. This was verified by tracing the handlers; an interactive terminal reproduction was not performed.

**Fix:** pass the exact per-step approval set into the common executor. Do not re-load and auto-approve a mutable plan after reviewing a different snapshot of it.

### F04 — HIGH: sandbox, dry-run and read-only flags do not guard mutation paths

**Evidence:** `main.rs:95–118` declares these global flags. `main.rs:955` resolves sandbox settings and prints “bubblewrap enabled”; dispatch passes settings only into the TUI runtime. The executor (`execute.rs:637`), web service actions (`web.rs:426`), and rollback (`ops.rs:195`) launch host processes directly. Searches found no execution use of the TUI sandbox field. `read_only` is not checked by TUI service mutation handlers or plan application. `dry_run` has no execution guard in these paths.

**Impact:** an operator relying on these flags can change the real host. Read-only mode still exposes destructive actions after grants/plan approval.

**Fix:** enforce execution mode in one shared authority boundary used by CLI/TUI/web/rollback. Reject unsupported flags with a clear error until implemented. Add marker-file tests proving each mode prevents host writes.

### F05 — HIGH: loopback dashboard exposes sensitive data and shared grants without authentication

**Evidence:** `web.rs:239–259` registers all routes without authentication, Host validation, or Origin validation; `web.rs:274` accepts WebSocket upgrades. Service actions check grants stored for the HELM process/user, not a requesting principal (`web.rs:385`).

Any local process/user able to connect can retrieve operational inventory and logs. A local attacker can use an available service/rollback grant before the intended UI action. WebSocket reads have a potential malicious-site path where browser local-network policy permits; browser reachability was not tested. The loopback bind limits direct network reachability and must be preserved.

**Fix:** authenticate all API and WebSocket connections, bind grants to principal/action/step, validate Host and Origin, and gate mutations against CSRF. Use an authenticated local session or Unix socket. Do not expose this server through a proxy/tunnel without resolving authentication.

### F06 — HIGH: pre-change backups do not support reliable restoration

**Evidence:** `execute.rs:704` writes original file content to `/tmp/helm-backup-*`, ignores backup-write failure, and returns a `PreChangeBackup` with empty `checksum_before` and empty `backup_content`. `main.rs:6187` persists steps but does not persist the returned backup records.

**Impact:** the engine can report successful file modification without a usable durable restoration point. Temporary copies use default file permissions and can contain sensitive original content; there is no backup link retained in the returned record.

**Fix:** securely persist original bytes, existence, permissions, checksum and restoration metadata before mutation; abort if required backup creation fails. Test edit→restart→rollback and newly-created-file rollback.

### F07 — HIGH: rollback can claim success after failures and rerun forward side effects

**Evidence:** `ops.rs:146` appends failed/skipped reversal messages but still sets `status='rolled_back'` at `ops.rs:226`. `ops.rs:480` reverses commands through substring replacement and executes the whole resulting string in `sh -c`.

For a successful command containing `systemctl stop ...; other_side_effect`, replacing “stop” with “start” reruns the unrelated side effect too. Missing backups and unsupported reversals still allow a rolled-back state. The web handler maps an `Ok(results)` containing failure messages to `ok: true` (`web.rs:473`).

**Fix:** store explicit structured inverse operations, track partial/failed rollback, verify restored state, and never infer rollback from arbitrary shell text. Refuse automatic reversal when no safe inverse exists.

### F08 — MEDIUM: persistence audit chain is forgeable, not HMAC

**Evidence:** `helm-memory/src/episodes.rs:1224–1250` hashes pipe-joined fields with an unkeyed 64-bit FNV-1a function. All inputs and the algorithm are available to a database writer.

**Impact:** it catches accidental/simple edits but cannot authenticate records against an attacker who rewrites the database and recomputes the chain. Delimiter-based encoding also lacks unambiguous field boundaries. Separate execution-chain comments should not be treated as proof of cryptographic authentication.

**Fix:** define the actual tamper threat model; use canonical encoding plus keyed authentication and protect/anchor the key/head outside attacker-writable records. Do not label the current persistence chain HMAC.

### F09 — MEDIUM: current protected paths are not redacted — reproduced

**Evidence:** `helm-core/src/secret.rs:268` matches legacy `/.helm/` paths. The compiled redactor returned XDG `secrets.toml`, `helm.db`, and `logs/helm.log` paths unchanged.

**Fix:** recognize resolved XDG paths and defaults, including the lock file; test custom XDG roots. Pattern-only provider-key redaction also does not protect arbitrary passwords, DB credentials or bearer tokens. Do not rely on it as the only data-export control.

### F10 — MEDIUM: config commands can persist provider keys; config rewrites lose settings

**Evidence:** `main.rs:1378–1416` accepts arbitrary dotted keys through `set_toml_path`, then writes TOML. Thus `config set provider.api_key <dummy>` persists precisely the field forbidden by the project rules, even though `FileProviderConfig` ignores it. TUI config editing uses the same helper. `write_helm_config` (`main.rs:1861`) constructs a new document, resets security preferences and disables telemetry rather than preserving unrelated settings; TUI provider/model switches call it (`tui.rs:4389`).

**Fix:** reject secret-bearing fields at every config mutation boundary, migrate accidental keys securely, validate without silently discarding malformed configuration, and update only the intended metadata using an atomic write.

## Findings: monitoring correctness and operational gaps

### F11 — HIGH: `monitor --fleet` labels local observations as remote results

**Evidence:** `main.rs:5125–5177` iterates registered remotes but calls the local `run_monitor_cycle` for every one. The comment explicitly describes future SSH work.

**Impact:** a fleet operator can receive fabricated remote health conclusions. TUI/web remote collection exists, so this is an inconsistent command path rather than absence of all fleet support.

**Fix:** reuse remote report collection here, preserve host provenance, and report unreachable hosts as unknown/offline. Never substitute local collection under a remote label.

### F12 — HIGH: collection failures can look healthy or resolve an active incident

**Evidence:** `collectors/mod.rs:49` returns `Ok(out)` regardless of subprocess exit status. `engine.rs:164` substitutes default domain values for returned errors. `reporter.rs:140` compares prior findings to current findings without requiring successful collection of the affected domain, and `reporter.rs:193` emits “No issues detected” for an empty finding set. `domains_checked` is derived from findings when unfiltered, not successful checks. JSON reports serialize only findings (`reporter.rs:258`), omitting collection health.

**Impact:** command/permission/daemon failures may become empty inventory and disappearances; an active condition can be marked self-resolved because evidence was unavailable. Stateful monitoring also resolves prior findings excluded by a later domain filter.

**Fix:** represent healthy/unhealthy/unknown/not-applicable separately; preserve exit status/stderr; require successful comparable observation for recovery. Render coverage/errors and never claim health based only on no findings.

### F13 — MEDIUM: idempotency and verification semantics are unsafe even without an attacker

**Evidence:** `execute.rs:335` skips `systemctl restart` when a service is active. Active does not mean configuration reload/restart is unnecessary. `execute.rs:453` treats verification as passed when expected output matches **or** the verification exits successfully. Expected-content mismatch can pass. Engine status tracks action failure independently of textual verification failures.

**Fix:** remove restart idempotency, use explicit postconditions, require exit status and declared content checks, and make failed verification affect change-set outcome. Run only independently authorized, read-only verification operations.

### F14 — MEDIUM: journal context indexing can panic in debug/test builds

**Evidence:** `collectors/logs.rs:255` computes `idx as i8 - latest_index as i8`. Around indices 127/128, individually narrowing the indices makes the subtraction overflow even though the correct relative offset is only -1/0/+1.

**Impact:** ordinary journal volume can panic with overflow checks; release wrapping typically produces the intended tiny offset, so this is not claimed as a default optimized-release crash.

**Fix:** subtract in a wide signed type, then convert the bounded difference. Add a fixture spanning the 127/128 boundary.

### F15 — MEDIUM: command execution lacks a complete timeout/resource/cancellation boundary

**Evidence:** `execute.rs:637` and rollback `ops.rs:195` await shell output without a timeout or output cap. Collector `run_timed` times out a child-output future but does not set `kill_on_drop` or manage a process group (`collectors/mod.rs:56`). Multiple sequential probes each receive a timeout rather than sharing one collector deadline.

**Impact:** hung fixes/rollbacks can block indefinitely; timed-out collectors can leave processes alive; large command output can exhaust memory. Timing a future does not establish process-tree termination.

**Fix:** one process runner with wall deadlines, process-group cancellation/reaping, bounded output and preserved partial diagnostics. Use it for fixes, checks, SSH and rollback.

### F16 — MEDIUM: snapshot/change persistence is best-effort after real mutations

**Evidence:** `main.rs:6187–6221` discards change/step/outcome write errors; `persist_monitor_snapshot` (`main.rs:5236`) discards snapshot insertion errors. The CLI later prints “Change set saved”. No transaction spans the change header, steps and backups. Before/after snapshot IDs returned by the executor are not persisted through this apply path.

**Impact:** disk-full, locking or schema failures can leave a changed host with absent/partial evidence and misleading success text. Before/after links can reference snapshots absent from the DB.

**Fix:** write a durable pending change and backup records before mutation; persist execution transitions explicitly and surface failures. Persist snapshots associated with the change. Do not promise that a record was saved without confirming it.

### F17 — MEDIUM: raw persistence and outbound prompt boundaries are inconsistent

**Evidence:** `SnapshotStore::insert` accepts/stores `findings_json` unchanged (`snapshots.rs:26`). `persist_monitor_snapshot` redacts snapshot JSON but passes findings separately (`main.rs:5236`). `write_audit_event` inserts caller fields without a redaction boundary (`main.rs:6339`). Runbook output is written without redaction (`main.rs:5963`). `build_narrative_prompt` appends evidence verbatim (`troubleshoot.rs:481`), then callers send it to the selected provider.

**Impact:** logs/evidence containing non-provider secrets may reach persistence or a cloud provider. Some callers sanitize correctly, but there is no universal guarantee.

**Fix:** centralize redaction at persistence and trace/export boundaries; apply data minimization before provider requests, label the outbound boundary, and test actual persistence/export paths. Avoid weakening exact executable input while redacting its display/audit copy.

### F18 — MEDIUM: generated-plan JSON formats are incompatible with application

**Evidence:** background generation serializes `LlmFixStep` directly (`main.rs:5384`); apply deserializes `Vec<PlanStep>` and silently defaults on failure (`main.rs:6024`). TUI manually builds a nested command object (`tui.rs:5191`) but writes free-text blast-radius/rollback strings where `CommandPreview` expects externally tagged enums or specific enum values.

**Impact:** a plan can be shown as ready while application reports no fix steps. This is especially significant because existing parser/cache tests do not prove generate→persist→load→apply interoperability.

**Fix:** one canonical persisted executable-plan schema with explicit conversion and versioning; fail with a parse error rather than `unwrap_or_default`. Add end-to-end schema round trips for CLI, TUI and web-generated plans.

### F19 — MEDIUM: stored-only provider keys disable background plan generation

**Evidence:** `llm_provider_usable` (`main.rs:5273`) checks session key, default environment key and Ollama, but does not consult `SecretsStore` or consistently honor custom `api_key_env`. `build_provider` does consult the store.

**Impact:** a correctly configured stored-key provider can be usable for direct calls but treated as disabled by daemon/web planning. Key presence and request capability are being decided differently.

**Fix:** share the existing key-resolution logic and make readiness a fallible, consistent operation. Test store-only, env-only, custom-env and unauthenticated compatible endpoints.

### F20 — MEDIUM: web plan lookup and cache can trap failed/stale results

**Evidence:** `web.rs:329` returns the latest persisted plan whenever one exists without checking status, TTL or current context. `PlanCache::ensure_plan` (`plan_cache.rs:93`) stores failure statuses under the same 24-hour TTL and uses separate check/insert operations for in-flight work. The TUI rate-limit countdown toggles a flag but does not dispatch the promised retry (`tui.rs:2400` area).

**Impact:** timeout/auth/malformed results can prevent regeneration, and concurrent calls can race to duplicate work. This is a functional/cost concern, not a demonstrated unlimited spending attack.

**Fix:** distinguish ready/error/loading expiry, explicitly support retry, atomically acquire in-flight ownership, and validate plan freshness against host/finding/evidence context.

### F21 — MEDIUM: assistant requests block the TUI event loop and cannot execute checks

**Evidence:** `tui.rs:3784` awaits the provider directly inside the input handler, supplies one user message and no tools, and does not populate the cancellable `agent_task`. `cancel_running_task` only aborts that optional handle (`tui.rs:3876`). Model catalog refresh uses `block_in_place`/`block_on` serially (`tui.rs:4259–4300`).

**Impact:** normal chat/key validation/catalog calls can freeze UI event processing until network completion. Follow-up instructions say to run checks, but no tool executor exists to do so. Responses lack previous conversation history.

**Fix:** background cancellable requests with result events, bounded concurrency for catalogs, and truthful text-only assist language. Restore tool execution only behind the shared authorization boundary if it remains a product requirement.

### F22 — MEDIUM: global database selection is ignored by many subcommands

**Evidence:** `main.rs:1013` honors global `--db-path` when opening initial memory, but monitor (`main.rs:5180`), snapshot (`main.rs:4738`), troubleshoot and apply-plan (`main.rs:5990`) independently call `default_db_path`.

**Impact:** a command can initialize one DB and subsequently query/write another; expected isolated operations can touch the real default state. It also undermines reproducible testing and fleet/incident consistency.

**Fix:** resolve runtime paths once and pass them to every command/service. Add an isolated alternate-DB test proving the default path remains untouched.

### F23 — MEDIUM: pipeline webhook integrity and history writes are weak

**Evidence:** `web.rs:491` accepts provider/service payloads without signature or token validation. `pipeline.rs:223` merges supplied run status. `pipeline.rs:844` writes one predictable `.tmp` file in a read/modify/write flow without a writer lock, while collectors can also update the file.

**Impact:** clients reaching the local API can forge configured pipeline health; simultaneous pull/webhook writers can lose updates or race rename operations. Present loopback binding limits direct remote webhook usability as well as exposure.

**Fix:** verify provider-specific signatures against secret-store credentials before parsing, prevent replays, and serialize history updates or reuse SQLite transactions. Provide a deliberately authenticated ingress design if external webhooks are required.

### F24 — MEDIUM: DB/log permissions and duplicate secret storage need hardening

**Evidence:** `MemoryStore::open` (`episodes.rs:208`) and `LogFileMakeWriter` (`main.rs:3895`) create files without explicit restrictive modes. Effective privacy depends on directory mode and umask. `helm_core::SecretStore::save_data` (`secret.rs:239`) writes before chmod, has no atomic update/lock, and retains old plaintext secret values in rotation history.

**Impact:** sensitive state can be readable under permissive local defaults; the unused/alternate store is an unsafe API to adopt accidentally. This is conditional on filesystem permissions, not a claim every installation exposes its DB.

**Fix:** secure state directories/files at creation, check symlinks/ownership according to the threat model, and consolidate storage behind the stronger CLI semantics. Keep only rotation metadata, not recoverable retired credentials.

### F25 — MEDIUM: installer skips the checksum asset and has a platform mismatch

**Evidence:** `install.sh` downloads and immediately chmod/moves the binary; it never downloads/verifies the published `.sha256`. It selects a GNU/Linux target from architecture without checking OS. `.github/workflows/release.yml` only publishes x86-64.

**Impact:** corruption is not detected independently, and an x86-64 non-Linux host may receive a Linux binary. ARM64 correctly receives source-build advice when the missing asset returns an error, but has no published build from this workflow.

**Fix:** verify checksum before installation, check OS/architecture explicitly, stage the replacement in the destination filesystem, and accurately enumerate published targets. A checksum from the same release is integrity checking, not independent release authentication.

### F26 — MEDIUM: no automatic snapshot retention in the monitoring write loop

**Evidence:** daemon and monitor-watch persist every collection cycle; the snapshot store has single-row deletion but no automatic age/size pruning wired into these loops. Persistent histories include operational inventory and logs, while ring buffers only bound in-memory metrics.

**Impact:** DB growth can eventually create the disk pressure being monitored and extend exposure of sensitive history.

**Fix:** bounded, configurable retention by age/size, with references needed for active incidents/changes protected and failures reported. Measure realistic snapshot sizes before selecting defaults.

### F27 — LOW/MEDIUM: SSH shell argument boundaries are not preserved

**Evidence:** `remote.rs:384` appends `sh`, `-lc`, and the script as local ssh arguments; `bootstrap.rs:136` similarly forwards argv. SSH constructs a remote command string, so local argument separation is not sufficient to preserve a script as one remote `-c` argument.

**Impact:** scripts containing spaces/metacharacters can be parsed at the wrong shell level, breaking bootstrap and remote commands. User-editable SSH options/install paths are trusted local configuration; this is not reported as an unauthenticated injection vulnerability.

**Fix:** send a script over stdin to a fixed remote shell, or serialize the remote command with correct shell quoting exactly once. Test generated scripts through a local shell/SSH-command fixture before real deployment.

### F28 — LOW: UTF-8 snapshot diff truncation can panic

**Evidence:** `main.rs:4888` uses `&s[..77]` on arbitrary JSON text when byte length exceeds 80. The boundary may split a multibyte character from a hostname, service description, path or log value.

**Fix:** truncate by character/valid byte boundary using an existing Unicode-safe helper. Add a non-ASCII diff fixture.

### F29 — MEDIUM: advertised optional telemetry feature does not compile — reproduced

**Evidence:** `cargo check -p helm-cli --features otel` fails at `helm-cli/src/telemetry.rs:15,18`: the selected `opentelemetry-otlp` version has no `new_pipeline` or `new_exporter` functions used by this module. The default build does not compile this optional code, so ordinary tests do not catch it.

The same check also reports incompatible tracing-layer types at `main.rs:3860–3861`; updating only the two exporter function calls is insufficient.

**Fix:** adapt this small module to the installed dependency API and add an optional-feature compile check to CI. No telemetry network endpoint was contacted by this build check.

## Features present but incomplete, and what should exist next

| Capability | Current state | Required next step |
|---|---|---|
| Monitoring | Many typed collectors/detectors; partial-failure handling exists | Truthful coverage and unknown-state semantics; failed-command tests |
| Controlled changes | Previews, approval prompts and execution results exist | Exact authorization binding, process limits, durable state transitions |
| Rollback | Backup schema, reversal heuristic and status UI exist | Reliable backups and structured inverses; verified partial failure |
| Sandboxing/read-only/dry-run | CLI flags/settings exist | Enforce centrally or reject; remove false assurance |
| Web operations | Live inventory, plans, services, rollback | Authenticated client identity and action-scoped grants |
| Fleet | Real SSH collection in dashboard paths | Reuse it in CLI; consistent provenance and freshness |
| AI assist | Provider adapters and narrative planning work at component level | Canonical plan schema, proper retries, store-aware readiness, cancellable calls |
| Agent safety | Capabilities/taint types survive | Do not promise historical agent enforcement; decide whether agent runtime is still intended |
| Audit | Chained records and verification exist | Cryptographic threat model, canonical encoding, complete operation coverage |
| Security/redaction | Secret wrapper and strong CLI store are useful | XDG-aware paths, arbitrary-secret minimization, storage/export invariants |
| Database/web health | Standalone collector modules and detectors exist | Wire the intended collectors into snapshots or explicitly label heuristic-only coverage |
| Service registration | Typed form and registry exist | Mask credential-bearing connection inputs; store secret references instead of embedding credentials in metadata |
| Pipelines | Pull parsing and webhook history exist | Signature/replay validation, concurrent storage and retry behavior |
| Alerts | Multiple sinks with dedup/rate policies | End-to-end delivery/failure/recovery validation with mock endpoints |
| Maintenance | Ring buffers bound live metric history | Persistent retention and backup/recovery procedures |
| TUI workflow | Extensive rendering/state logic and tests | Stop presenting removed tools/undo/MCP as working actions; split business services from views |
| Distribution | x86-64 release workflow and source fallback | Checksum verification, platform gate, tested supported-target matrix |
| Optional telemetry | Config, feature flag and integration module exist | Repair dependency API usage; `--features otel` currently fails to compile |

No new feature should precede fixing authorization, false-health reporting, and restoration. For the current product direction, an authenticated operations console with predictable deterministic diagnosis is a smaller and more useful completion target than adding another general agent layer.

## What is solid / issues not claimed

- CLI secrets use a separate store, restrictive permissions, an advisory writer lock and atomic replacement. Normal key precedence is correct, and startup env keys are not automatically saved.
- Most inspected SQL uses bound parameters. No general SQL-injection finding was established. Dynamically interpolated internal schema-table names are not equivalent to public SQL input.
- Web service actions pass action/unit as process arguments rather than interpolating them into `sh -c`. F05 concerns identity/shared authority; it is not a shell-metacharacter exploit in this handler.
- The web server binds loopback. It was not found to listen on all interfaces by default.
- Provider HTTP clients have request deadlines. F15 concerns process execution and total lifecycle, not a claim every network client lacks timeouts.
- The release job has write permission and lists the required installer/binary/checksum assets. Missing checksum *verification* is an installer issue.
- The source identity search found the prohibited URL strings only inside instructions/checklist examples, not an established wrong runtime endpoint. OpenRouter uses the fork referer.
- Strong typing, deterministic fixtures, mock providers, finding validation and broad unit coverage are useful foundations. They do not compensate for missing cross-layer negative tests.

## Verification performed

| Check | Result |
|---|---|
| Graph extraction, Cargo dependency introspection | Completed; counts above |
| Exported graph integrity diagnostic | Passed for exported endpoints/edges; extractor completeness not guaranteed |
| `cargo fmt --check` | Failed: formatting in `crates/helm-monitor/benches/helmops.rs` |
| `cargo test --workspace --all-targets` | Passed: 732 test executions, 0 failures, 2 ignored executions; CLI tests run twice because two binaries share `main.rs` |
| Always-deny execution probe | Reproduced pre-approval marker write in isolated temporary state with stubbed systemctl |
| Indexed-decision probe | Reproduced wrong-step approval with idempotent first step |
| XDG redaction probe | Reproduced unchanged protected paths using actual compiled `helm_core` |
| `cargo clippy --workspace --all-targets -- -D warnings` | Failed: four test-code lints in `metric_history.rs:150,167,196` and `ring_buffer.rs:138` |
| `cargo check -p helm-cli --features otel` | Failed: unavailable `new_pipeline` / `new_exporter` APIs in `telemetry.rs:15,18` |
| `cargo build --release -p helm-cli` | Passed; both helm and helmops release binaries built |
| Live providers, real remote deployments, destructive operations | Not run |
| Dependency advisory/CVE scan | Not run; no dependency safety verdict |

Initial test/Clippy/build attempts were blocked by missing registry data/DNS. After permitted dependency access, the full tests completed. These initial environment errors are not counted as code defects. Probe source and raw build/test logs were kept under `/tmp`, not committed as source tests or production changes.

## Required regression tests

1. Always-denied malicious preflight command cannot create a marker, including service and package probes.
2. Approval set stays attached to exact steps across skips, duplicate titles, failures and reordering.
3. TUI approve/deny sequence produces exactly the permitted operations, including earlier denial followed by final approval.
4. Sandbox/read-only/dry-run prevent host mutation for CLI, TUI, HTTP actions and rollback.
5. Edit→persist→restart→rollback restores original bytes/mode/existence; backup failure prevents mutation; failed reversal cannot produce `rolled_back`.
6. Collector exit failure/timeout/missing privilege produces unknown coverage, never healthy or self-resolved incidents.
7. Generated plans round-trip through each UI's persistence into the executor without silent empty defaults.
8. Stored-only credentials enable intended planning; failures can be retried; no duplicate work under concurrent requests.
9. Auth/Origin/Host tests cover HTTP and WebSocket reads and mutations; grants cannot be borrowed by another client.
10. Persistence and outbound prompts redact XDG paths and representative arbitrary credentials; Unicode/log-index boundaries cannot panic.

## Remediation order

**P0 — before relying on mutation features:** F01–F07. Remove unsafe preflight evaluation, bind approvals to commands, respect denials, enforce modes, authenticate dashboard access, and make backup/rollback truthful. Introduce one execution service reused by all three frontends; this is justified reuse of a safety boundary, not a new general framework.

**P1 — before relying on unattended monitoring:** F11–F18, F22–F24. Correct remote provenance, unknown/recovery semantics, timeouts, persistence failures, plan schema, data boundaries and webhook integrity. Add retention before long-lived deployment.

**P2 — reliability and usability:** F19–F21, F25–F29. Repair readiness/cache/retry, unblock the TUI, verify installers, retain state predictably, fix remote argument handling and boundary panics, and restore optional telemetry compilation.

**P3 — maintainability:** extract shared plan/operation/persistence services from `main.rs` and `tui.rs`, remove stale flags/commands/types or implement them deliberately, and test public feature configurations. Avoid a rewrite: preserve working collectors and tested models, replace the smallest shared broken boundaries first.

The acceptance target is concrete: no denied or read-only action changes the host; no failed observation claims recovery; every applied change has durable evidence and an honest restoration outcome.

## Final validation addendum

The default release build completed successfully. Formatting and strict Clippy remain failed, and the optional otel feature fails compilation. The required release gate is therefore not clear despite passing default tests and release compilation. No tag, commit, deployment, or production-code change was made.
