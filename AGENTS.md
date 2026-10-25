# AGENTS.md — HELM Codebase Navigation

> One-stop index. Never explore from scratch — look here first.

---

## Mandatory Rules

These are release-blocking rules. Follow them for every change.

1. **Fork identity must stay correct**
   - Repository URLs, installer URLs, release headers, and docs must point to
     `Jatin-Mali/helm`, not upstream or placeholder values.
   - Grep before shipping:
     - `rg -n "white-phantom|github.com/helm|helm.sh/install" .`

2. **Never persist plaintext provider keys outside the secrets store**
   - `$XDG_CONFIG_HOME/helm/secrets.toml` (or `~/.config/helm/secrets.toml`) is the only persistent store for provider secrets.
   - `config.toml` must never contain `provider.api_key` or any raw key value.
   - The TUI must never silently copy env keys into the secrets store on startup.
   - Resolution order remains:
     `--api-key` override → secrets store → environment variable.

3. **HELM local state is sensitive**
   - Treat these paths as protected local state:
     - `$XDG_CONFIG_HOME/helm/secrets.toml`
     - `$XDG_CONFIG_HOME/helm/.secrets.toml.lock`
     - `$XDG_DATA_HOME/helm/helm.db`
     - `$XDG_DATA_HOME/helm/logs/helm.log`
   - `fs_read` must deny these by default.
   - Redaction must hide both provider-style keys and these HELM paths before
     persistence and trace logging.

4. **Do not weaken redaction or audit persistence**
   - Before writing goals, transcript content, final messages, warnings, audit
     fields, or trace summaries, pass them through `helm_core::redact_secrets`.
   - Terminal output can remain human-readable when required, but local
     persistence and tracing must be redacted.

5. **Installer and release flow must stay usable**
   - `install.sh` must either install a published asset or fail with clear
     source-build instructions.
   - The release workflow must publish:
     - `install.sh`
     - `helm-x86_64-unknown-linux-gnu`
     - `helm-x86_64-unknown-linux-gnu.sha256`
   - GitHub Actions release jobs need `contents: write`.

6. **README/docs must match the real build path**
   - Source build path is:
     - `cargo build --release -p helm-cli`
     - `./target/release/helm`
   - Keep release-install and source-build instructions clearly separated.
   - Document first-run provider setup and secrets behavior accurately.

7. **Release gate before tagging**
   - Required before moving a release tag:
     - `cargo fmt --check`
     - `cargo clippy --workspace --all-targets -- -D warnings`
     - `cargo test --workspace --all-targets`
     - `cargo build --release -p helm-cli`

8. **Do not commit local debugging artifacts**
   - CI logs, ad hoc `test/` dumps, scratch files like `1_test.txt`, and other
     one-off local evidence should stay untracked unless explicitly requested.

---

## Workspace Layout

```
helm/
├── Cargo.toml                   # workspace root; members listed below
├── crates/
│   ├── helm-core/               # shared types: capability, taint, message, error, validation, secret, private_fs
│   ├── helm-providers/          # LLM backends (anthropic, openai-compat family, gemini, ollama, mock)
│   ├── helm-memory/             # SQLite: episodes + audit chain, snapshots, change sets, plans, finding state
│   ├── helm-monitor/            # collectors, detectors, plans, approvals, execution engine, alerting, pipelines
│   └── helm-agent/              # bounded general-agent runtime + host tools/permissions
├── helm-cli/                    # binaries `helm` and `helmops` (same main.rs): CLI, TUI, web dashboard
├── tests/                       # root integration tests
├── scripts/                     # test-install.sh (installer checks)
├── docs/                        # user-facing docs; docs/superpowers/ = repair spec, plan and ledger
├── src/                         # IGNORE — empty test host
└── codex/                       # IGNORE — unrelated OpenAI Codex reference
```

---

## crates/helm-core

**Purpose:** Shared types and small primitives.

| File | Key Items | Notes |
|------|-----------|-------|
| `src/capability.rs` | `Capability`, `GrantScope` | Stored grants (`helm permissions`) |
| `src/taint.rs` | `TaintLevel`, `Tainted<T>` | External content cannot escalate to `*.write` |
| `src/message.rs` | `Role`, `ContentBlock`, `Message` | Wire format for all provider chat |
| `src/error.rs` | `HelmError`, `ProviderError`, ... | |
| `src/secret.rs` | `Secret`, `SecretStore` (deprecated for provider keys), `redact_secrets()`, `is_protected_local_path()` | Redaction is the persistence/prompt boundary; protected state paths |
| `src/private_fs.rs` | `create_private_dir`, `open_private_file` | 0700 dirs created / 0600 files, no symlink follow |
| `src/validation.rs` | `Validator` | Input validation |

Grep: `redact_secrets`, `is_protected_local_path`, `TaintLevel::External`.

---

## crates/helm-providers

**Purpose:** LLM backends implementing `Provider` (`chat(ChatRequest) -> ChatResponse`, native tool calls).

| File | Provider | Key |
|------|----------|-----|
| `src/provider.rs` | `Provider`, `ChatRequest`, `ChatResponse`, `ToolSchema`, `Usage` | |
| `src/anthropic.rs` | Anthropic | `ANTHROPIC_API_KEY` |
| `src/openai_compat.rs` | Groq, OpenRouter, NVIDIA NIM, generic OpenAI-compatible | `GROQ_API_KEY`, `OPENROUTER_API_KEY`, `NVIDIA_API_KEY` |
| `src/gemini.rs` | Gemini | `GOOGLE_API_KEY` / `GEMINI_API_KEY` |
| `src/ollama.rs` | Ollama | `OLLAMA_HOST` (default localhost:11434) |
| `src/quirks.rs` | `quirks_for(provider, model)` | Per-model overrides; check before provider edits |
| `src/pricing.rs` | model pricing table | Unknown models have no price; budgets then require explicit prices |
| `src/mock.rs` | scripted provider | unit tests |

Tool-call response fixtures exist for every provider; live coverage is only what has actually been run (see `docs/demo.md` once written).

---

## crates/helm-memory

**Purpose:** Persistence in SQLite (`$XDG_DATA_HOME/helm/helm.db`).

| File | Key Items | Notes |
|------|-----------|-------|
| `src/episodes.rs` | `MemoryStore`, `AuditEventRecord`, `verify_partitioned_audit_events`, `audit_hash` | Append-only audit chain: v2 = SHA-256 with domain tag; legacy FNV rows still verify; legacy-after-v2 rejected as downgrade; **unkeyed** (print heads, anchor off-host) |
| `src/snapshots.rs` | `SnapshotStore` | Redacted on insert; `prune` keeps 7 days/1000 rows and anything referenced |
| `src/changesets.rs` | `ChangeSetStore`, `TroubleshootingPlanStore` | Change sets, steps, encrypted-backup metadata, dashboard plan states |
| `src/finding_state.rs` | finding lifecycle | |

Do not break the audit-log schema; changes must be additive.

---

## crates/helm-monitor

**Purpose:** Observation, planning and controlled execution for Linux hosts.

| Area | Files | Notes |
|------|-------|-------|
| Collection | `collectors/*`, `snapshot.rs`, `process_runner.rs` | Per-domain `CollectionCoverage`; nonzero exits are errors; bounded process groups |
| Detection | `detectors/*`, `engine.rs`, `findings.rs`, `correlation.rs`, `reporter.rs` | Incidents resolve only on comparable fresh successful observation |
| Plans | `troubleshoot.rs`, `plan_document.rs`, `plan_cache.rs` | Canonical plan schema; single-owner generation with retry |
| Execution | `execute.rs`, `execution_policy.rs`, `file_change.rs` | `PlanApprovals` (digest+cwd+boot, 15 min, single use), `ExecutionEngine`, `ExecutionPolicy`, `ExecutionJournal`, encrypted pre-change backups |
| Pipelines | `pipeline.rs`, `webhook_auth.rs` | Signed (GitHub HMAC / GitLab token / generic timestamped HMAC), replay-protected webhooks; flock-serialized history |
| Services | `custom_service.rs`, `custom_rules.rs` | No DB passwords in connection strings |
| Alerting | `alerting/*` | Slack, PagerDuty, webhook |

---

## crates/helm-agent

**Purpose:** General agent. Provider-neutral, bounded, host-authorized.

| File | Key Items | Notes |
|------|-----------|-------|
| `src/runtime.rs` | `AgentRuntime`, `AgentConfig`, `AgentRun`, `AgentEvent`, `AgentOutcome` | Bounded steps/context/retries/wall time/budget; running, waiting-for-approval, cancelled, failed, completed; evidence-based summaries |
| `src/context.rs` | context trimming | Preserves tool-call/result pairing |
| `src/tools.rs` | `HostTools` (`fs_read`, `search`, `fs_write`, `shell`, `http`, `browser`) | Workspace-confined, symlink-safe, protected HELM state, SSRF-safe HTTP, journaled mutations |
| `src/permissions.rs` | exact single-use call approvals | Tool/web/file content is untrusted data and never grants authority |

The browser tool is an isolated static reader (no JavaScript). Agent tasks are local-only.

---

## helm-cli

**Purpose:** Binaries `helm` and `helmops` (identical `main.rs`).

| File | Key Items |
|------|-----------|
| `src/main.rs` | `Command` enum, provider resolution (`build_provider`, `llm_provider_usable`), config editing (`update_config_file`), audit/verify, daemon, snapshot/monitor/troubleshoot/apply-plan |
| `src/agent_cli.rs` | `helm run`: `setup()` (shared with TUI), `observe`, `conclude`, `sanitize_for_terminal` |
| `src/agent_session.rs` | persisted sessions: redacted JSON under `<data>/agent-sessions` |
| `src/tui.rs` | `TuiApp`, `ModalState` (`AgentApproval`, `ActionReview`, ...), dashboard, assistant over the agent runtime |
| `src/web.rs`, `src/web_auth.rs`, `src/web/dashboard.html` | loopback dashboard: pairing code, HttpOnly session, Host/Origin/CSRF checks, exact single-use action reviews, signed webhooks |
| `src/fleet.rs`, `src/remote.rs`, `src/bootstrap.rs` | SSH fleet observation (`remote_command` quoting), remote registry |
| `src/execution_journal.rs`, `src/ops.rs` | durable change journal; rollback via authenticated backups |
| `src/paths.rs` | XDG paths, `--db-path` override |

**Subcommands** (`helm --help` is authoritative): `run`, `models`, `doctor`, `permissions {list,grant,revoke}`, `audit {verify,show}`, `secrets`, `init`, `config`, `service`, `completion`, `tui`, `remote`, `agent` (remote monitoring agent install), `bootstrap`, `trust-report`, `dashboard`, `daemon {start,stop,status,run}`, `incident`, `snapshot`, `monitor [--fleet]`, `troubleshoot`, `explain`, `apply-plan`, `change-set`, `backup`, `runbook`, `alerting`, `web`.

**`helm run <task>`:** `--workspace`, `--resume <id>`, `--sessions`, `--json`, `--max-steps`, `--timeout-secs`, `--budget-usd` (needs `--input-price`/`--output-price`). Approvals are interactive and single-use; without a terminal the run stops `waiting_for_approval` and executes nothing. `--remote` is refused.

**Config/secrets rules:**
- `update_config_file()` is the only config editor; it rejects credential-looking keys/values.
- `config.toml` stores provider metadata only; keys live in `secrets.toml` (0600).
- The TUI may use an env key for the active session but never silently copies it to the store.

**TUI rules:** auth input is masked; agent approval and service/rollback review modals show the exact operation; cancel goes through the runtime token.

---

## tests/

Root `tests/`: `approval_integrity`, `change_recovery`, `execution_policy`, `privacy_boundaries`, `v17_snapshot`, `v18_monitor`, `v19_troubleshoot`, `v20_controlled_execution`.
`helm-cli/tests/`: `agent_workflows` (scripted provider, pty approvals, SIGINT), `daemon_lifecycle`, `execution_modes`, `fleet` (stub `ssh`), `runtime_paths`, `web_security`.
`crates/helm-agent/tests/`: `agent_runtime`, `agent_security`. `scripts/test-install.sh`: installer checks with mocked uname/curl.

Run: `cargo test --workspace --all-targets`. Advisory scan: `cargo audit --deny warnings`.

---

## docs/

| File | Contents |
|------|----------|
| `docs/providers.md` | Provider configuration, env vars, model IDs |
| `docs/threat-model.md` | Security model, attack surface |
| `docs/data-boundary.md` | What is persisted/sent to providers, redaction and audit-chain limits |
| `docs/trust-ladder.md` | Autonomy levels |
| `docs/troubleshooting.md` | Common errors, recovery limits |
| `docs/agent-on-remote.md` | Remote monitoring agent |
| `docs/superpowers/plans/` | Repair spec, plan and progress ledger |

**Docs rules:** keep install/source-build/first-run accurate; say plainly which platforms and operations are unsupported.

---

## XDG Base Directory Compliance

| Type | Path |
|------|------|
| Config | `$XDG_CONFIG_HOME/helm/` (or `~/.config/helm/`) |
| Data | `$XDG_DATA_HOME/helm/` (or `~/.local/share/helm/`) |
| Cache | `$XDG_CACHE_HOME/helm/` (or `~/.cache/helm/`) |

Config: `config.toml`, `secrets.toml` (0600), `remotes.toml`, `hooks.toml`, `keybindings.json`, `allowlist.toml`.
Data: `helm.db`, `agent-sessions/`, `fleet-state.json`, `daemon-state.json`, `logs/helm.log`, backup key/material (encrypted).

---

## Quick Reference

| Question | Answer |
|----------|--------|
| Agent loop | `crates/helm-agent/src/runtime.rs` → `AgentRuntime::run/resume` |
| Agent tools + path/URL policy | `crates/helm-agent/src/tools.rs` → `HostTools` |
| `helm run` | `helm-cli/src/agent_cli.rs` |
| Plan approvals / execution | `crates/helm-monitor/src/execute.rs` → `PlanApprovals`, `ExecutionEngine` |
| Rollback / backups | `helm-cli/src/ops.rs` → `rollback_change_set`; `execution_journal.rs` |
| Audit chain | `crates/helm-memory/src/episodes.rs` → `audit_hash`, `verify_partitioned_audit_events` |
| Redaction | `crates/helm-core/src/secret.rs` → `redact_secrets` |
| Provider selection | `helm-cli/src/main.rs` → `build_provider` |
| Quirks registry | `crates/helm-providers/src/quirks.rs` |
| Dashboard auth | `helm-cli/src/web_auth.rs`; handlers in `web.rs` |
| Webhook verification | `crates/helm-monitor/src/webhook_auth.rs` |
| Fleet | `helm-cli/src/fleet.rs`, `remote.rs` |
| TUI | `helm-cli/src/tui.rs` |
