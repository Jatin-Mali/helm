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
│   ├── helm-core/               # types: capability, taint, message, error, validation, secret
│   ├── helm-providers/          # LLM backends (7 providers)
│   ├── helm-tools/              # tool implementations (15 tools + composite registry + validator)
│   ├── helm-memory/             # SQLite: episodes, sessions, graph, skills, user_profile, skill_learner
│   └── helm-agent/              # ReAct loop, supervisor, budget, parser, plan_cache, model_router
├── helm-cli/                    # binary: main.rs (CLI) + tui.rs (TUI)
├── tests/                       # integration + e2e tests
├── docs/                        # user-facing docs
├── src/                         # IGNORE — empty test host
└── codex/                       # IGNORE — unrelated OpenAI Codex reference
```

---

## crates/helm-core

**Purpose:** Shared types. No logic, no I/O.

| File | Key Types | Notes |
|------|-----------|-------|
| `src/capability.rs` | `Capability` (10 variants), `GrantScope` (Once/Session/15m/Always) | Gate checked before every tool call |
| `src/taint.rs` | `TaintLevel` (User/Tool/External), `Tainted<T>` | External content cannot escalate to `*.write` |
| `src/message.rs` | `Role`, `ContentBlock`, `Message` | Wire format for all LLM chat |
| `src/error.rs` | `HelmError`, `BudgetError`, `ProviderError`, `ToolError` | All error types in one place |
| `src/secret.rs` | `Secret`, `SecretStore`, `RotationPolicy`, `redact_secrets()` | Secret wrapper + `set/get/delete/list/rotate/check_rotation_needed/rotation_history`; stored at `$XDG_CONFIG_HOME/helm/secrets.toml` with 0o600 mode |
| `src/validation.rs` | `Validator`, `ValidationError` | `validate_prompt`, `validate_shell`, `validate_url`; called before every goal/tool input |
| `src/lib.rs` | re-exports | — |

**Grep targets:**
- `Capability::` — all capability usages
- `TaintLevel::External` — taint escalation checks
- `ContentBlock::ToolUse` — tool call format
- `redact_secrets` — all mandatory redaction call sites
- `Validator::validate_prompt` — prompt validation call sites

---

## crates/helm-providers

**Purpose:** LLM backends. Each implements the `Provider` trait.

| File | Provider | API |
|------|----------|-----|
| `src/provider.rs` | `Provider` trait, `ChatRequest`, `ChatResponse`, `ToolDefinition` | Core abstraction |
| `src/anthropic.rs` | Anthropic Claude | `ANTHROPIC_API_KEY` |
| `src/openai_compat.rs` | Groq, OpenRouter, NvidiaNim, OpenAI-compat | `GROQ_API_KEY`, `OPENROUTER_API_KEY`, `NVIDIA_API_KEY` |
| `src/gemini.rs` | Google Gemini | `GEMINI_API_KEY` |
| `src/ollama.rs` | Local Ollama | `OLLAMA_HOST` (default: localhost:11434) |
| `src/mock.rs` | Deterministic mock | Used in all unit/integration tests |
| `src/quirks.rs` | `quirks_for(provider, model)` → `ProviderQuirks` | Per-model overrides: temperature, format, addendum |

**Key functions:**
- `sanitize_tool_name()` in `openai_compat.rs` — strips `<|...|>` token leaks from model output
- `quirks_for()` in `quirks.rs` — call this before every ChatRequest to apply overrides
- `GROQ_DEFAULT_MODEL` in `openai_compat.rs` = `"llama-3.3-70b-versatile"` (not gpt-oss)
- OpenRouter `HTTP-Referer` must point at `https://github.com/Jatin-Mali/helm`

**Grep targets:**
- `impl Provider for` — find all provider implementations
- `ExpectedFormat::` — tool call format variants
- `sanitize_tool_name` — token leak fix

---

## crates/helm-tools

**Purpose:** Every tool the agent can call.

| File | Tool Name | Capability Required |
|------|-----------|-------------------|
| `src/shell.rs` | `shell` | `ShellExec` |
| `src/fs_read.rs` | `fs_read` | `FsRead` |
| `src/fs_write.rs` | `fs_write` | `FsWrite` |
| `src/browser.rs` | `browser` | `BrowserControl` (PinchTab wrapper) |
| `src/network.rs` | `network` | `NetworkOut` (ip, routes, DNS, curl) |
| `src/process.rs` | `process` | `ShellExec` (list, info, kill, nice) |
| `src/disk.rs` | `disk` | `FsRead` (df, du, lsblk, smart) |
| `src/logs.rs` | `logs` | `FsRead` (journalctl, tail, grep) |
| `src/package.rs` | `package` | `PkgInstall` (apt/dnf/pacman auto-detect) |
| `src/service.rs` | `service` | `SystemService` (systemctl + journalctl) |
| `src/http.rs` | `http` | `NetworkOut` — GET/POST/PUT/DELETE/HEAD/PATCH; domain allowlist enforced |
| `src/search.rs` | `search` | `FsRead` — ripgrep-backed grep/glob (falls back to std walk + regex) |
| `src/tool.rs` | `Tool` trait, `ToolContext`, `ToolOutput` | Core abstraction |
| `src/registry.rs` | `ToolRegistry`, `CompositeTool` | Dynamic tool registration + lookup + composite tool sequences |
| `src/validator.rs` | `AllowlistConfig` | `is_shell_allowed`, `is_domain_blocked`, `is_ignored`; shell pattern + domain + helmignore enforcement |

**Grep targets:**
- `impl Tool for` — find all tool implementations
- `ToolRegistry::register` — tool registration
- `register_composite` — composite/macro-tool registration
- `taint` in browser.rs — marks browser output as external-tainted
- `validate_denylist` in `fs_read.rs` — protected path enforcement for HELM local state
- `AllowlistConfig` — allowlist enforcement for http and shell

**Partially implemented (not wired into runtime):**
- `src/git.rs` — `GitTool`: 11 git actions (status, log, diff, add, commit, push, pull, branch, checkout, stash, clone) via `tokio::process::Command`. Requires `Capability::ShellExec`.
- `src/mcp.rs` — `McpTool`: JSON-RPC 2.0 stdio bridge to external MCP servers. Config at `$XDG_CONFIG_HOME/helm/mcp-servers.toml`. Actions: `list_tools`, `call`. CLI: `helm mcp {list,add,remove,test,run}`.

**v1.0.1 reliability notes:**
- `ToolContext::new()` defaults to a 120s per-tool timeout.
- `disk df` honors the requested `path`; do not ignore mount-specific paths.
- `disk du` returns sorted top-level usage with a 5-minute scan window. Prefer it over raw `du -sh /home/*`.
- `disk largest_files` uses a bounded `find | sort | head` pipeline and returns human-readable sizes.
- `shell` returns partial stdout/stderr on timeout with `success=false` instead of dropping all output.

---

## crates/helm-memory

**Purpose:** Persistence. SQLite for episodes; filesystem for skills.

| File | Key Types | Notes |
|------|-----------|-------|
| `src/episodes.rs` | `EpisodeRecord`, `StepRecord`, `AuditEventRecord`, `CapabilityGrantRecord` | Append-only; HMAC chain in AuditEventRecord |
| `src/skills.rs` | `Skill`, `SkillsManager` | Files in `$XDG_DATA_HOME/helm/skills/`; versioned markdown with metadata |
| `src/sessions.rs` | `SessionStore`, `SessionRecord` | SQLite-backed; `list`, `delete`, `export`, `snapshot`, `resume` |
| `src/graph.rs` | `EntityGraph` | SQLite knowledge graph; `find_entities`, `find_relations`, `semantic_search` (cosine, pure Rust), `prune_stale_relations`, `store_embedding`, `export_json`, `import_json` |
| `src/skill_learner.rs` | `SkillLearner` | `extract_skills_from_episode` (SHA256 skill key, confidence scoring), `find_matching_skills` (top 5 by confidence) — called after successful episodes |
| `src/user_profile.rs` | `UserProfileStore`, `Role` | `record_tool_outcome`, `get_tool_preference`, `get_preferred_tools`, `set_preference`, `get_preference`; `Role` enum (Admin/User/Viewer) with `allows(cap: &str) -> bool`, `set_role`, `get_role` |
| `src/merge_episodes.rs` | `MergeResult`, `MergeConflict`, `ConflictResolution` | `merge_episodes`; conflict resolution: KeepFirst/KeepSecond/KeepBoth/Unresolved |

**Key operations:**
- Episode lifecycle: `insert_episode` → `append_step` → `finish_episode`
- Audit: `append_audit_event` with prev_hash chaining → `verify_chain()`
- Skills: `list()`, `show(name)`, `approve(name)`, `test(name)` (gold-example validation)
- Sessions: `list()` → `resume(id)` → auto-save snapshot every N steps (in ReactAgent)
- Skill learning: after successful episode → `extract_skills_from_episode` → stored with SHA256 key
- RBAC: `Role::allows(cap)` checked in ReactAgent before every tool call
- Persistence must redact secrets before writing episode goals, steps, final messages, warnings, audit fields, or errors.

**Grep targets:**
- `stable_hash_hex` — audit chain hashing
- `SkillsManager::test` — gold example validation
- `EpisodeOutcome::` — Success/Failure/Timeout/Cancelled
- `SessionStore::` — session management
- `extract_skills_from_episode` — skill learning entry point
- `Role::allows` — RBAC enforcement
- `merge_episodes` — episode merge logic

---

## crates/helm-agent

**Purpose:** Orchestration. Drives the ReAct loop and Supervisor DAG.

| File | Key Types | Notes |
|------|-----------|-------|
| `src/react.rs` | `ReactAgent`, `AgentEvent` | Main ReAct loop; RBAC check + input validation before every tool call; `#[instrument]` tracing spans on `run_with_events` |
| `src/supervisor.rs` | `Supervisor`, `Plan`, `PlanStep`, `EvidenceVerifier` | DAG-based planning; FSM per step |
| `src/budget.rs` | `CostBudget`, `BudgetStatus` | `warn_threshold`, `record_cost`, `remaining`; `BudgetStatus::Ok/Warning/Exceeded`; warns at 80%, stops at 100% |
| `src/parser.rs` | `parse_tool_calls()` | Handles Native/XmlTag/FunctionTag/Pythonic/BareJson formats |
| `src/context_window.rs` | `ContextWindow` | Rolling window; prunes oldest messages on overflow |
| `src/plan_cache.rs` | `PlanCache` | SQLite plan cache keyed by `goal_hash()`; `lookup`, `store`, `evict_old`; wired into ReactAgent planning path |
| `src/model_router.rs` | `ModelRouter`, `FallbackChain` | `select_for_task`, `record_outcome`, `get_success_rates`, `select_with_fallback`; routes by `TaskComplexity`; persists outcomes to `router_outcomes` table |

**Supervisor step FSM:** `Pending → Running → (Complete | Retrying | Failed)`

**Evidence types** (in `supervisor.rs`):
- `FileExists { path }`
- `FileContains { path, needle }`
- `ExitCode { tool_use_id, code }`
- `StdoutMatch { tool_use_id, pattern }`
- `ServiceStatus { service, status }`
- `HttpStatus { url, status }`

**AgentEvent variants** (all in `react.rs`, exhaustively matched in `tui.rs`):
- `TextDelta { chunk }` — ≤64-byte streaming chunks
- `ToolCall { name, input }` / `ToolResult { name, output, taint }`
- `PlanCacheHit { goal_hash }` / `PlanCacheMiss`
- `SkillSuggested { skill_id, skill_name, tool_sequence, confidence }`
- `ProviderFailover { from, to, reason }`
- `BudgetWarning { spent, limit }` / `BudgetExceeded { spent, limit }`
- `PromptCacheHit { tokens_saved }`
- `PermissionDenied { capability, tool }` — RBAC block (red line in TUI)
- `ValidationFailed { field, reason }` — input validation block (red line)
- `BreakpointHit { step }` — debug pause (yellow banner in TUI)

**Done (v1.4):**
- `src/cancel.rs` — `CancellationToken`: `Arc<AtomicBool>`; `cancel()`, `is_cancelled()`, `child()`; checked every loop iteration; wired to `tokio::signal::ctrl_c()`

**Done (v1.5):**
- `execute_single_tool()` — private helper returning `(ContentBlock, Taint, u32)`
- `execute_tool_uses()` — runs all `ToolUse` blocks concurrently via `futures::future::join_all`, merges taint + corrections deltas, preserves result order
- `futures` crate added to workspace and `helm-agent`

**Lifecycle hooks** (v1.4, in ReactAgent):
- `pre_run` — called before the loop starts
- `post_run` — called after loop ends
- `on_tool_call` — called before each tool execution

**Missing (planned):**
- `src/roles.rs` — sub-agent specialization (v2.0)

**Grep targets:**
- `AgentEvent::` — all event variants; match arms in tui.rs must stay exhaustive
- `EvidenceVerifier::verify` — post-condition checks
- `parse_tool_calls` — multi-format tool call parsing
- `CostBudget::record_cost` — budget enforcement
- `PlanCache::lookup` — plan cache hit path
- `check_capability` — RBAC enforcement in react.rs
- `ModelRouter::select_with_fallback` — provider fallback chain

**v1.0.1 execution policy:**
- The default system prompt requires plan-first execution before tool use.
- Disk/root-cause tasks should follow `disk df` → `disk du` → scoped `disk largest_files`.
- Repeating the same broad timed-out command is a bug; narrow the path or use typed tools.
- Provider trace summaries must be redacted before `trace!` logging.

---

## helm-cli

**Purpose:** Binary. CLI entry point + TUI.

| File | Key Items |
|------|-----------|
| `src/main.rs` | `ProviderChoice`, `ProviderSettings`, `build_provider()`, `default_api_key_env()`, `CliProgressSink`, all subcommands |
| `src/tui.rs` | `TuiApp`, `ModalState`, `TuiRuntimeInner`, `render_modal()`, `handle_modal_key()` |

**Subcommands** (all in `main.rs`):
- `helm run <TASK>` — one-shot agent task; `--fallback`, `--budget`, `--pre-run`, `--post-run`, `--on-tool-call`, `--trace` flags
- `helm tui` — interactive terminal UI
- `helm init` — interactive setup wizard → `$XDG_CONFIG_HOME/helm/config.toml`
- `helm doctor [--json]` — health check (provider reachability, DB, quirks, tool registry)
- `helm episodes [--limit N]` — list episode history
- `helm episodes show <ID>` — show episode detail
- `helm episodes merge <ID1> <ID2>` — merge two episodes (KeepFirst/KeepSecond/KeepBoth resolution)
- `helm replay <EPISODE_ID>` — replay recorded episode
- `helm models` — list models for active provider
- `helm permissions` — manage capability grants
- `helm audit verify` — verify HMAC chain integrity
- `helm audit export` — export audit log
- `helm audit list` — list audit events
- `helm config {get,set,edit,validate,path}` — config inspection and mutation
- `helm completion {bash,zsh,fish}` — shell completion generation
- `helm secrets {set,get,delete,list,rotate,check}` — secrets store with rotation policy
- `helm skills {list,show,delete,learn}` — skill library management
- `helm sessions {list,export,delete,resume}` — session persistence management
- `helm memory graph` — query knowledge graph entities/relations
- `helm memory export` / `helm memory import` — graph JSON export/import
- `helm memory gc` — prune stale graph relations
- `helm profile show` — user profile preferences
- `helm profile set/get` — set/get individual preferences
- `helm profile routes` — model router success rates
- `helm stats` — cost and usage statistics

**Config/secrets rules:**
- `write_helm_config()` must not accept or write plaintext provider keys.
- `FileProviderConfig` should not grow a persistent `api_key` field again.
- `helm init` stores keys in `secrets.toml`; `config.toml` stores only provider metadata.
- TUI provider switching may use an in-memory key for the active session, but
  it must persist only to `SecretsStore`, never to config.

**TUI key bindings** (in `tui.rs`):
- `Ctrl+C` — quit (or cancel task)
- `Ctrl+P` — command palette
- `Ctrl+J` / `Alt+Enter` / `Shift+Enter` — insert newline
- `Ctrl+L` — clear current visible session
- `Ctrl+D` on empty input — quit
- `Ctrl+T` — toggle tool-history sidebar
- `Shift+Tab` — cycle modes (`Chat` → `Plan` → `Diagnose`)
- `PgUp/PgDn` — scroll transcript by half a screen
- `Ctrl+Home/Ctrl+End` — jump to transcript top/latest
- Mouse wheel — scroll transcript
- In ProviderSelector: digits 1-7 switch provider; Up/Down navigate; Enter apply
- In ModelSelector: type to edit model string; Enter apply

**TUI security rules:**
- Startup may read env keys for the active session, but must not auto-save them.
- Auth/onboarding key entry may save to `SecretsStore` only after explicit user action.
- Rendered auth input must stay masked and never appear in transcript snapshots.

**Key patterns:**
- `active_settings: ProviderSettings` on `TuiApp` — mutated by ProviderSelector/ModelSelector; passed to every new task via `run_agent_task()`
- `ModalState` enum — CommandPalette / Permission / ProviderSelector / ModelSelector / ApiKeyInput / AuthRequired / Error / Help
- `friendly_error()` — maps raw error strings to user-readable messages (including HTTP 400 tool rejection)
- TUI theme colors are `Color::Rgb` constants at the top of `tui.rs`; keep the blue palette centralized.
- Transcript scrolling is bottom-relative via `session.transcript_scroll` and rendered with `Paragraph::scroll`, not manual line slicing.
- `EnableMouseCapture`/`DisableMouseCapture` are paired in `run_tui`; do not add mouse support without preserving terminal cleanup.
- `CommandAction::from_slug()` handles slash-command aliases such as `/quit`, `/exit`, and `/q`.

**TUI slash commands** (v1.1+):
- `/new` — start a new session
- `/resume` — resume last session
- `/sessions` — list sessions
- `/snapshot` — save snapshot of current session
- `/theme <name>` — switch theme
- `/help` — show help

**TUI AgentEvent handlers** (v1.4–v1.5, in `tui.rs`):
- `ProviderFailover` — status bar notice
- `BudgetWarning` — yellow warning line in transcript
- `BudgetExceeded` — red line, input blocked until acknowledged
- `PromptCacheHit` — status bar token-savings indicator
- `PermissionDenied` — red line with capability name
- `ValidationFailed` — red line with field + reason
- `BreakpointHit` — yellow pause banner; resumes on Enter
- `PlanCacheHit` / `PlanCacheMiss` — status bar indicator
- `SkillSuggested` — inline suggestion with confidence score

**Grep targets:**
- `ModalState::` — all modal variants
- `active_settings` — live provider settings flow
- `run_agent_task` — task dispatch with settings
- `default_api_key_env` — env var name per provider

---

## tests/

| File | What It Tests |
|------|---------------|
| `reliability_suites.rs` | ReAct loop reliability (MockProvider); 25-run and 100-run suites |
| `e2e_react.rs` | End-to-end ReAct flow validation |
| `browser_prompt_injection.rs` | Taint-based prompt injection blocking |
| `groq_hardened_loop.rs` | Groq quirks + 5-layer security validation |
| `groq_shell_composition.rs` | Shell composition end-to-end (live, ignored) |
| `groq_live.rs` | Live Groq API (ignored by default; needs GROQ_API_KEY) |
| `ollama_live.rs` | Live Ollama (ignored; needs OLLAMA_HOST) |
| `provider_matrix_live.rs` | Multi-provider matrix (live, ignored) |

**Run all non-ignored:** `cargo test --workspace --all-targets`
**Run 100-run suite:** `cargo test deterministic_100_run -- --ignored`

---

## docs/

| File | Contents |
|------|----------|
| `docs/providers.md` | Provider configuration, env vars, model IDs |
| `docs/threat-model.md` | Security model, taint system, attack surface |
| `docs/troubleshooting.md` | Common errors and fixes |

**Docs rules:**
- Keep release install, source build, and first-run setup accurate for this fork.
- If release assets are not guaranteed for all architectures, say so plainly.
- Security docs must explain the difference between env-only use and stored-key use.

---

## XDG Base Directory Compliance

HELM follows the [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html):

| Type | Path |
|------|------|
| Config | `$XDG_CONFIG_HOME/helm/` (or `~/.config/helm/`) |
| Data | `$XDG_DATA_HOME/helm/` (or `~/.local/share/helm/`) |
| Cache | `$XDG_CACHE_HOME/helm/` (or `~/.cache/helm/`) |
| Secrets | `$XDG_CONFIG_HOME/helm/secrets.toml` (mode 0600) |

Files: `config.toml`, `hooks.toml`, `keybindings.json`, `remotes.toml`, `mcp-servers.toml`, `allowlist.toml`, `secrets.toml`
Data: `helm.db`, `graph.db`, `skills/`, `snapshots/`
Logs: `$XDG_DATA_HOME/helm/logs/helm.log`

Grep targets for path discovery:
- `dirs::home_dir().join(".helm"` — non-XDG-compliant paths
- `XDG_CONFIG_HOME` — XDG resolution functions

---

## Quick Reference

| Question | Answer |
|----------|--------|
| Where is the ReAct loop? | `crates/helm-agent/src/react.rs` |
| Where is tool registration? | `crates/helm-tools/src/registry.rs` |
| Where is taint enforcement? | `crates/helm-core/src/taint.rs` + browser.rs tool output |
| Where is the audit chain? | `crates/helm-memory/src/episodes.rs` → `AuditEventRecord` |
| Where is the TUI? | `helm-cli/src/tui.rs` |
| Where is provider selection? | `helm-cli/src/main.rs` → `build_provider()` |
| Where is tool call parsing? | `crates/helm-agent/src/parser.rs` |
| Where is quirks registry? | `crates/helm-providers/src/quirks.rs` |
| Where is the Groq default model? | `crates/helm-providers/src/openai_compat.rs` → `GROQ_DEFAULT_MODEL` |
| Where are skill gold examples? | `crates/helm-memory/src/skills.rs` → `SkillsManager::test()` |
| Where is the Supervisor DAG? | `crates/helm-agent/src/supervisor.rs` |
| Where is budget tracking? | `crates/helm-agent/src/budget.rs` → `CostBudget` |
| Where is session management? | `crates/helm-memory/src/sessions.rs` → `SessionStore` |
| Where is skill learning? | `crates/helm-memory/src/skill_learner.rs` → `extract_skills_from_episode` |
| Where is RBAC? | `crates/helm-memory/src/user_profile.rs` → `Role::allows` + `check_capability` in react.rs |
| Where is input validation? | `crates/helm-core/src/validation.rs` → `Validator` |
| Where is the plan cache? | `crates/helm-agent/src/plan_cache.rs` → `PlanCache::lookup` |
| Where is provider fallback? | `crates/helm-agent/src/model_router.rs` → `FallbackChain::select_with_fallback` |
| Where is the HTTP tool? | `crates/helm-tools/src/http.rs` |
| Where is the search tool? | `crates/helm-tools/src/search.rs` |
| Where is secrets rotation? | `crates/helm-core/src/secret.rs` → `SecretStore::rotate` |
| Where is the knowledge graph? | `crates/helm-memory/src/graph.rs` → `EntityGraph` |
| Where is episode merging? | `crates/helm-memory/src/merge_episodes.rs` → `merge_episodes` |
