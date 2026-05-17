# HELM — Project Promise

This file is a public commitment. It is updated when scope changes; old entries are kept.

## v0.1 — target ship date: 2026-07-27 (12 weeks)

User-visible features:
- `helm "<task>"` runs a natural-language task to completion using an Anthropic API key.
- Three first-class tools: shell execution, filesystem read, filesystem write.
- SQLite-backed episode log of every task.
- ReAct loop with iteration cap and token budget enforcement.
- Apache 2.0 source on GitHub.

Non-goals for v0.1:
- Multi-provider (Anthropic only in v0.1; OpenAI/Ollama in v0.2).
- Browser automation (v0.2).
- Skill learning (v0.2).
- Permission/capability system (v0.2 — v0.1 has a confirm-y/n gate only).
- macOS/Windows.

## History
- 2026-05-05 — v1 evidence gates: added deterministic Supervisor DAG types,
  step retry/failure state, goal-aware EvidenceVerifier, browser prompt-injection
  regression, 25-run deterministic reliability suite, ignored 100-run release
  gate, and ignored live provider matrix for Groq/OpenRouter/Gemini/NVIDIA
  NIM/Ollama.
- 2026-05-05 — v1 hardening pass: fixed current worktree compile breakage,
  hardened browser automation around PinchTab CLI commands, replaced unsafe
  skill-manager file handling with typed errors, added `helm init`, provider
  docs, threat model, troubleshooting docs, install script, and release CI.
- 2026-05-05 — v0.6: reviewed local skills. Added file-backed skill listing,
  show, approve, disable, and basic test flow. Learned skills remain
  user-reviewed; no auto-running unapproved code.
- 2026-05-05 — v0.5: browser control via PinchTab CLI-backed `browser` tool.
  Browser output is marked external-tainted; browser actions expose strict JSON
  schema and support open, snapshot, click, fill/type, keypress, wait,
  screenshot, text extraction, and close.
- 2026-05-05 — v0.4: terminal TUI. Added `helm tui` with chat/session panel,
  tool-call timeline, permission prompt modal, replay/file preview,
  provider/model status, doctor/session panel, session switcher, model selector,
  and command palette.
- 2026-05-05 — v0.3: typed Linux machine-control tools. Added `process`,
  `service`, `package`, `disk`, `network`, and `logs` tools with strict JSON
  schemas and confirmation gates for destructive actions. Agent prompt now
  tells models to prefer typed tools over raw shell for common admin tasks.
- 2026-05-05 — v0.2.0: complete TUI rebuild. Replaced vim-modal multi-panel TUI
  with a Claude Code / Codex-style always-insert single-pane chat REPL.
  tokio::select! event loop over terminal events (spawn_blocking channel),
  background agent task (tokio::spawn + mpsc), 100ms tick, collapsible Ctrl+T
  sidebar for last 10 tool calls, inline permission prompts (no modal overlays).
- 2026-05-05 — v0.2: capability, taint, and audit security base. Tool calls now
  map to explicit capabilities, dangerous capabilities require grants, external
  taint requires fresh approval for privileged operations, and each allow/deny is
  recorded in a hash-chained SQLite audit log. Added `helm permissions` and
  `helm audit` commands.
- 2026-05-05 — v0.1.5: three real-world bug fixes: unified shell.run capability
  (merged shell.exec/shell.shell with DB migration v5), extended path allowlist
  with /mnt and /media plus symlink-from-HOME rule, and rolling context trimmer
  to prevent Groq parse failures past ~15k tokens with DB migration v6
  (total_turns_summarized column).
- 2026-05-05 — v0.1.4: final tool-call hardening. Five layers in one shot:
  (L1) universal parser handles native, XML-tag, `<function=NAME>`, Pythonic, and
  bare-JSON formats; (L2) JSON Schema validator rejects bad inputs before dispatch;
  (L3) corrective retry (up to 2 attempts) injects schema hint on InvalidInput;
  (L4) post-condition verifier checks fs_write paths for existence, non-empty, and
  unexpanded shell syntax; (L5) provider quirks registry auto-detects Groq, Ollama,
  and OpenRouter behaviour from provider name + model string. Schema migrated to v3
  (corrections_used, format_recovery_used, response_format_log). `helm doctor` now
  shows a `[quirks]` section; `helm replay` shows correction and format-recovery
  counters. Integration test `groq_hardened_loop` added (ignored without key).
- 2026-05-04 — v0.1.3: shell tool gains mode (exec/shell), redirect_stdout_to,
  stdin. System prompt updated to teach the model when to use each. Closes
  the regression where models wrote literal "$(date)" into files via fs_write.
- 2026-05-04 — v0.1.2 shipped multi-provider support with Groq default, Gemini, OpenAI-compatible providers, doctor diagnostics, and live Groq testing.
- 2026-05-04 — v0.1 promise written.
