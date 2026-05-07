# AGENTS.md — HELM Codebase Navigation

> One-stop index. Never explore from scratch — look here first.

---

## Workspace Layout

```
helm/
├── Cargo.toml                   # workspace root; members listed below
├── crates/
│   ├── helm-core/               # types: capability, taint, message, error
│   ├── helm-providers/          # LLM backends (7 providers)
│   ├── helm-tools/              # tool implementations (13 tools)
│   ├── helm-memory/             # SQLite episodes + skills library
│   └── helm-agent/              # ReAct loop, supervisor, budget, parser
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
| `src/lib.rs` | re-exports | — |

**Grep targets:**
- `Capability::` — all capability usages
- `TaintLevel::External` — taint escalation checks
- `ContentBlock::ToolUse` — tool call format

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
| `src/tool.rs` | `Tool` trait, `ToolContext`, `ToolOutput` | Core abstraction |
| `src/registry.rs` | `ToolRegistry` | Dynamic tool registration + lookup |
| `src/validator.rs` | `InputValidator` | Path traversal checks, input sanitization |

**Missing (not yet implemented):**
- `src/git.rs` — planned for v1.1
- `src/mcp.rs` — planned for v1.1

**Grep targets:**
- `impl Tool for` — find all tool implementations
- `ToolRegistry::register` — tool registration
- `taint` in browser.rs — marks browser output as external-tainted

---

## crates/helm-memory

**Purpose:** Persistence. SQLite for episodes; filesystem for skills.

| File | Key Types | Notes |
|------|-----------|-------|
| `src/episodes.rs` | `EpisodeRecord`, `StepRecord`, `AuditEventRecord`, `CapabilityGrantRecord` | Append-only; HMAC chain in AuditEventRecord |
| `src/skills.rs` | `Skill`, `SkillsManager` | Files in `~/.helm/skills/`; versioned markdown with metadata |

**Key operations:**
- Episode lifecycle: `insert_episode` → `append_step` → `finish_episode`
- Audit: `append_audit_event` with prev_hash chaining → `verify_chain()`
- Skills: `list()`, `show(name)`, `approve(name)`, `test(name)` (gold-example validation)

**Missing (planned):**
- `src/graph.rs` — entity graph (v1.2)
- `src/procedures.rs` — procedural memory (v1.2)
- `src/skill_learner.rs` — Voyager-style learning (v1.3)
- `src/user_profile.rs` — user-style learning (v1.3)
- `src/plan_cache.rs` — move here or into helm-agent (v1.2)

**Grep targets:**
- `stable_hash_hex` — audit chain hashing
- `SkillsManager::test` — gold example validation
- `EpisodeOutcome::` — Success/Failure/Timeout/Cancelled

---

## crates/helm-agent

**Purpose:** Orchestration. Drives the ReAct loop and Supervisor DAG.

| File | Key Types | Notes |
|------|-----------|-------|
| `src/react.rs` | `ReactAgent`, `AgentEvent` | Main ReAct loop: observe→think→act→repeat |
| `src/supervisor.rs` | `Supervisor`, `Plan`, `PlanStep`, `EvidenceVerifier` | DAG-based planning; FSM per step |
| `src/budget.rs` | `BudgetTracker` | Token counting; hard limits enforced per step |
| `src/parser.rs` | `parse_tool_calls()` | Handles Native/XmlTag/FunctionTag/Pythonic/BareJson formats |
| `src/context_window.rs` | `ContextWindow` | Rolling window; prunes oldest messages on overflow |

**Supervisor step FSM:** `Pending → Running → (Complete | Retrying | Failed)`

**Evidence types** (in `supervisor.rs`):
- `FileExists { path }`
- `FileContains { path, needle }`
- `ExitCode { tool_use_id, code }`
- `StdoutMatch { tool_use_id, pattern }`
- `ServiceStatus { service, status }`
- `HttpStatus { url, status }`

**Missing (planned):**
- `src/plan_cache.rs` — plan caching by goal embedding (v1.2)
- `src/model_router.rs` — meta-learning / model routing (v1.3)
- `src/roles.rs` — sub-agent specialization (v2.0)
- `src/cancel.rs` — cancellation tokens (v1.4)

**Grep targets:**
- `AgentEvent::` — event sink for monitoring
- `EvidenceVerifier::verify` — post-condition checks
- `parse_tool_calls` — multi-format tool call parsing
- `BudgetTracker::check` — budget enforcement

---

## helm-cli

**Purpose:** Binary. CLI entry point + TUI.

| File | Key Items |
|------|-----------|
| `src/main.rs` | `ProviderChoice`, `ProviderSettings`, `build_provider()`, `default_api_key_env()`, all subcommands |
| `src/tui.rs` | `TuiApp`, `ModalState`, `TuiRuntimeInner`, `render_modal()`, `handle_modal_key()` |

**Subcommands** (all in `main.rs`):
- `helm run <TASK>` — one-shot agent task
- `helm tui` — interactive terminal UI
- `helm init` — interactive setup wizard → `~/.helm/config.toml`
- `helm doctor [--json]` — health check (provider reachability, DB, quirks, tool registry)
- `helm episodes [--limit N]` — list episode history
- `helm replay <EPISODE_ID>` — replay recorded episode
- `helm models` — list models for active provider
- `helm permissions` — manage capability grants
- `helm audit` — view/verify audit log chain
- `helm skills {list,show,approve,test}` — skill library management

**TUI key bindings** (in `tui.rs`):
- `Ctrl+C` — quit (or cancel task)
- `Ctrl+P` — command palette
- `PgUp/PgDn` — scroll output
- In ProviderSelector: digits 1-7 switch provider; Up/Down navigate; Enter apply
- In ModelSelector: type to edit model string; Enter apply

**Key patterns:**
- `active_settings: ProviderSettings` on `TuiApp` — mutated by ProviderSelector/ModelSelector; passed to every new task via `run_agent_task()`
- `ModalState` enum — CommandPalette / Permission / ProviderSelector / ModelSelector / Error / Help
- `friendly_error()` — maps raw error strings to user-readable messages (including HTTP 400 tool rejection)

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

---

## Config Files (Runtime, not in repo)

| Path | Purpose |
|------|---------|
| `~/.helm/config.toml` | Provider, model, DB path, default capabilities |
| `~/.helm/audit.log` | HMAC-chained audit log (line-delimited JSON) |
| `~/.helm/skills/` | Skill markdown files (versioned) |
| `~/.helm/helm.db` | SQLite database (episodes, steps, capability grants) |
| `~/.helm/mcp-servers.toml` | MCP server configs (v1.1, not yet implemented) |
| `~/.helm/user_profile.toml` | Learned user preferences (v1.3, not yet implemented) |

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
| Where is budget tracking? | `crates/helm-agent/src/budget.rs` |
