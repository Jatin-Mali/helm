# Contributing to HELM

## Before You Start

Read `AGENTS.md` — it indexes every crate, key file, and symbol so you never have to explore from scratch.

Read `roadmap.md` — it defines what belongs in each phase. Do not add v1.1+ features before v1.0 ships.

## Gate (must pass before every commit)

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

## Project Structure

```
helm/
├── crates/
│   ├── helm-core/      # shared types: capability, taint, message, error
│   ├── helm-providers/ # LLM backends (Anthropic, Groq, Gemini, Ollama, ...)
│   ├── helm-tools/     # tool implementations (shell, fs, service, browser, ...)
│   ├── helm-memory/    # SQLite episodes + skills library
│   └── helm-agent/     # ReAct loop, Supervisor DAG, budget, parser
├── helm-cli/           # binary: CLI subcommands + TUI
└── tests/              # integration and e2e tests
```

## Adding a Tool

1. Create `crates/helm-tools/src/<name>.rs` implementing the `Tool` trait
2. Declare the required `Capability` variant (see `helm-core/src/capability.rs`)
3. Register in `crates/helm-tools/src/registry.rs`
4. Add a row to the tools table in `AGENTS.md`
5. Write at least one integration test

## Adding a Provider

1. Implement `Provider` trait in `crates/helm-providers/src/<name>.rs`
2. Add `quirks_for()` entry in `crates/helm-providers/src/quirks.rs` for any known model quirks
3. Add the provider variant to `ProviderChoice` in `helm-cli/src/main.rs`
4. Wire it into `build_provider()` and `interactive_init()`
5. Add a row to `docs/providers.md`

## Security Invariants — Never Break

1. Capability gate checked before every tool call
2. `TaintLevel::External` propagated through `Tainted<T>` — never stripped without explicit user action
3. External content (browser, SSH, MCP) always tagged `TaintLevel::External`
4. `*.write` capabilities blocked on `TaintLevel::External` inputs
5. Audit log append-only; HMAC chain must verify after any episode

## Commit Style

One subject line, imperative mood, 72 chars max. Body only when the why is non-obvious.

```
feat(tools): add git tool with clone/status/diff/commit
fix(providers): handle Groq 429 with exponential backoff
```

## Pull Requests

- One logical change per PR
- All gate checks pass
- Tests added or updated
- `AGENTS.md` updated if you added a new file

## What Not To Do

- No v1.1+ features before v1.0 ships publicly
- No fine-tuning, weight updates, or self-modifying code
- No Windows/macOS-specific code before v2.0
- No `--no-verify` on git hooks
- Do not touch `codex/` or `src/` at repo root — both are placeholders
