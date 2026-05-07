# CLAUDE.md — Token Efficiency Rules for HELM

> These rules override all defaults. Read before every session.

---

## 1. RTK — Always Active

All shell commands route through RTK automatically via hook. Never invoke rtk manually for normal commands.

Meta commands (call directly):
```
rtk gain              # token savings analytics
rtk gain --history    # command history with savings
rtk discover          # find missed optimization opportunities
```

---

## 2. Search Before Reading

Never read a file cold. Always grep first to find exact lines.

```bash
grep -n "fn build_provider" helm-cli/src/main.rs    # find line number first
grep -rn "ModalState::" helm-cli/src/tui.rs         # locate symbol
grep -rn "impl Tool for" crates/helm-tools/src/     # find all implementations
```

Then read only the specific line range:
```
Read file_path offset=N limit=50
```

---

## 3. Output Rules

- **No prose summaries.** State what changed + what's next. One sentence each.
- **No printing full files** unless explicitly requested.
- **No multi-paragraph explanations.** If the code is clear, say nothing.
- **No `I have updated...` / `I have implemented...`** — just show the diff target.
- **Code comments:** only when WHY is non-obvious. Never explain WHAT.
- **Artifacts** (code, configs): write to files. Never paste inline unless it's ≤10 lines.

---

## 4. Tool Hierarchy

| Task | Use |
|------|-----|
| Read a file you will edit | `Read` |
| Find a symbol or pattern | `Bash` with grep |
| Edit existing file | `Edit` (not Write) |
| Create new file | `Write` |
| Large output (>20 lines) | Spawn `Explore` subagent |
| Complex multi-file research | Spawn `Explore` subagent |
| Implementation planning | Spawn `Plan` subagent |
| Never | `cat`, `head`, `tail`, `echo >` via Bash |

---

## 5. Parallel Tool Calls

When tool calls are independent, fire all in one response block. Never sequence what can be parallelized.

```
Good:  Read(file_a) + Read(file_b) + Bash(grep ...) — one message
Bad:   Read(file_a) → then Read(file_b) → then grep
```

---

## 6. Context Management

**Compact triggers** (suggest `/compact` after):
- Any task that modified ≥3 files
- After a full phase of work (feature complete)
- When context exceeds 60%
- After any multi-file refactor

**Clear triggers** (suggest `/clear` after):
- Switching to a completely different subsystem
- After tagging a release
- When starting a new phase

**After `/compact`:** context-mode knowledge base is preserved. Use `ctx purge` only if you want a fresh start.

---

## 7. Subagent Rules

| When | Which |
|------|-------|
| Finding files/symbols across codebase | `Explore` |
| Designing implementation before coding | `Plan` |
| Large open-ended research | `Explore` |
| Never | General-purpose for simple grep tasks |

Subagents are expensive. Use direct tools for anything answerable in 1-3 tool calls.

---

## 8. HELM-Specific Shortcuts

**Always check AGENTS.md before exploring.** Every crate, key file, and grep target is indexed there.

**Before any edit to a provider:** check `quirks.rs` for model-specific overrides.

**Before any tool capability change:** check `capability.rs` for the 10 defined capabilities.

**Before adding a new tool:** register in `ToolRegistry` (`crates/helm-tools/src/registry.rs`).

**Run gate before any commit:**
```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

**Full RC gate:**
```bash
cargo test deterministic_100_run -- --ignored
```

---

## 9. Security Invariants — Never Break

1. Capability gate checked before every tool call
2. Taint level propagated through `Tainted<T>` — never strip without explicit user action
3. External content (browser output, SSH, MCP) always tagged `TaintLevel::External`
4. `*.write` capabilities blocked on `TaintLevel::External` tainted inputs
5. Audit log append-only; HMAC chain must verify after any episode

---

## 10. What Not To Do

- Do not add features beyond the current phase's scope
- Do not implement fine-tuning, weight updates, or self-modifying code (ever)
- Do not touch `codex/` — it is an unrelated OpenAI reference
- Do not use `src/` at repo root — it is an empty test host
- Do not break backward compatibility of the audit log schema
- Do not skip `cargo clippy` before committing
- Do not use `--no-verify` on git hooks
- Do not add Windows/macOS-specific code before v2.0
