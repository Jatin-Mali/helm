# HELM — Product Roadmap

## Final Goal
**A fully autonomous, self-learning Linux operations agent that runs headless on any machine, controls it completely, improves from its own episodes, and can be deployed across multiple machines — open source, privacy-first, no cloud required.**

---

## Current State (as of 2026-05-06)

| Phase | Feature | Status |
|-------|---------|--------|
| v0.1 | ReAct loop, shell/fs tools, SQLite episodes | ✅ Done |
| v0.2 | Capability model, taint model, audit log | ✅ Done |
| v0.3 | process/service/package/disk/network/logs/browser tools | ✅ Done |
| v0.4 | TUI (ratatui+crossterm) | ✅ Done |
| v0.5 | Browser via PinchTab, injection guard | ✅ Done |
| v0.6 | Skills library, GC, helm skills CLI | ✅ Done |
| v0.7 | Supervisor DAG, FSM, Evidence verifier | ✅ Done |
| v0.8 | install.sh, helm init, docs, release CI | ✅ Done |
| v1.0 RC | 100-run suite, security hardening | 🔄 In Progress |
| v1.0 | Public release | ⬜ Next |

---

## Explicit Rules

### DO
- Finish the 100-run suite before tagging v1.0.0-rc1
- Keep each phase's exit criteria as the gate — don't merge until met
- Add features only to their earliest version — no forward-creep
- Use the audit chain (`helm audit verify`) to validate all writes
- Write tests before marking any phase done
- Keep AGENTS.md updated whenever a new file is added

### DO NOT
- Do not add v1.1+ features before v1.0 ships publicly
- Do not implement fine-tuning, weight updates, or self-modifying agent code (v4.0+ research territory)
- Do not add Windows/macOS support before v2.0
- Do not build a hosted SaaS before v4.0
- Do not add community/marketplace skill sharing — use decentralized skill exchange (v3.1) only
- Do not use `codex/` — it is an unrelated OpenAI reference and not part of HELM
- Do not break the 5-layer security guarantee: capability gate → taint check → confirmation → audit → verifier

---

## Phase Details

### v1.0 RC — Release Candidate *(current)*
**Remaining:**
- Run 100-run deterministic suite: `cargo test deterministic_100_run -- --ignored`
- Tag `v1.0.0-rc1`

**Exit:** All 265 tests pass, 100-run suite passes, tag pushed.

---

### v1.0 — Public Release
**What:** GitHub release, honest README, `curl | sh` install, demo GIF, HN post.
**Build:** Polish docs, add CONTRIBUTING.md, set up GitHub Discussions, record 90-second demo.
**Exit:** 100 stars, 5 issues from strangers, 0 critical security issues in first 72h.

---

### v1.1 — Git Tool + MCP Client
**Git tool** (`crates/helm-tools/src/git.rs`):
- Operations: clone, status, diff, add, commit, push, pull, log, branch, checkout
- Capability: `git.read`, `git.write`
- Input: path + operation + args; output: structured result
- Exit: `helm "show me what changed in this repo today"` works without raw shell

**MCP client** (`crates/helm-mcp/`):
- New crate; implement MCP client spec
- Config: `~/.helm/mcp-servers.toml`
- Auto-register MCP server tools into ToolRegistry
- Exit: add Gmail MCP server, `helm "draft a reply to my latest email"` works

---

### v1.2 — Memory & Plan Cache
**Graph memory** (`crates/helm-memory/src/graph.rs`):
- SQLite tables: `nodes`, `edges`, `node_embeddings` (sqlite-vec)
- Background indexer: extract entities from tool inputs/outputs post-episode
- `helm gc` command: prune nodes with TTL + decay (low weight, no recent use)
- Exit: `helm "everything you've done involving /etc/nginx"` returns ranked history

**Plan caching** (`crates/helm-agent/src/plan_cache.rs`): ⭐ NOVEL
- Store successful task plans keyed by goal embedding
- New task: if cosine similarity > threshold, reuse cached plan with param substitution
- Verifier still runs; planner is skipped entirely
- Exit: 5th run of a recurring task uses 0 planner tokens

**Procedural memory** (`crates/helm-memory/src/procedures.rs`):
- Nightly job: cluster recent successful episodes by goal-embedding similarity
- Synthesize prose "procedure" templates via planner LLM
- Inject top-K procedures into planner context at task start
- Exit: repeat tasks complete in measurably fewer iterations

---

### v1.3 — Skill Learning + User Learning
**Voyager-style skill learning** (`crates/helm-memory/src/skill_learner.rs`): ⭐ NOVEL
- Watch episodes; when ≥3 use similar tool sequences, propose a new parameterized skill
- TUI review screen: name, code, required capabilities, gold examples from triggering episodes
- Approve → installed in `~/.helm/skills/`; future similar tasks invoke skill directly
- Skill versioning: new version must pass all gold examples or is rejected + rolled back
- Exit: after 50 episodes, ≥3 user-extracted skills exist with successful re-invocations

**Meta-learning / model routing** (`crates/helm-agent/src/model_router.rs`): ⭐ NOVEL
- Track per-provider+model: format reliability, avg iterations, verifier correction rate, token cost/success
- `helm models stats` shows profile per model
- Planner queries: "best model for this task type" → model id
- Exit: routing-eligible tasks auto-pick the right model; dashboard shows per-model stats

**User-style learning** (`crates/helm-memory/src/user_profile.rs`): ⭐ NOVEL
- Observer: after each episode, lightweight LLM call extracts ≤3 preference signals
- Merge into `~/.helm/user_profile.toml` with confidence weights; decay unused
- Inject relevant slices into planner system prompt per task
- Exit: after 100 episodes, `helm profile show` displays learned preferences; output noticeably reflects user style

---

### v1.4 — Concurrency + Sandbox
**Cancellation** (`crates/helm-agent/src/cancel.rs`):
- `tokio_util::CancellationToken` per step; `tokio::select!` on every tool call
- Save partial state to memory on cancel; episode marked `cancelled`
- TUI: dedicated cancel-current-step key binding
- Exit: mid-task Ctrl+C cancels current step only, preserves episode

**Bubblewrap sandbox** (`helm-cli/src/sandbox.rs`):
- `--sandbox` flag; detect `bwrap` at runtime
- Construct flags from `~/.helm/policy.toml` filesystem + network policy
- Exit: `helm --sandbox "run downloaded script"` cannot read outside sandbox even if script tries

---

### v1.5 — SSH/Remote Target ⭐ NOVEL
**What:** `helm --remote prod-1 "..."` runs agent against a remote machine.

**Three modes:**
- `just-shell`: tools execute via SSH transparently; no daemon on remote
- `agent-on-remote`: HELM daemon on target; send goals to it via gRPC
- `bootstrap`: auto-installs HELM on target on first contact, upgrades to agent-on-remote

**Build:** `Target` enum (Local, Ssh{host}, AgentOnRemote{host,port}); tools dispatch based on target; rely on `~/.ssh/config` — do not roll own key management.

**Exit:** `helm --remote my-vps "show top 10 processes by memory"` works against any reachable SSH host.

---

### v2.0 — Multi-Agent + Disagreement Protocol
**Plan DAG with deterministic supervisor** (extend `crates/helm-agent/src/supervisor.rs`):
- Goals decompose to DAG; ready nodes schedule in parallel
- Supervisor: Rust state machine (not LLM); retry with backoff; replan on threshold failure
- Token/time/tool-call budgets per step and per task
- Exit: parallel sub-agents run against 3 hosts simultaneously

**Sub-agent specialization** (`crates/helm-agent/src/roles.rs`):
- `Role` enum: Triager (cheap model), Planner (frontier), Executor (per-step), Verifier, Retro (post-task)
- `ProviderRouter` picks configured model per role
- Exit: `helm doctor` shows which model is assigned to each role

**Disagreement protocol**: ⭐ NOVEL
- When verifier and executor disagree: spawn third independent agent (different model)
- Third agent reviews both positions + ground truth, rules
- Only triggers when `*.write` capability involved (cost gate)
- Exit: induced disagreement (mock executor lies) → correctly resolved by third agent

**v2.0 release:** Blog post naming the 8 novel features. "What HELM does that nobody else does."

---

### v2.5 — Interface Expansion
**Notification system** (`crates/helm-notify/`):
- Channels: libnotify (desktop), Telegram, Slack DM, email
- `--notify` flag per task; per-channel config in `~/.helm/config.toml`

**Scheduled tasks** (`crates/helm-schedule/`):
- `helm schedule "every morning at 8am, ..."` → systemd-timer-backed
- Results via notification system; visible in `helm episodes`

**Web UI** (local, `helm-web/`):
- Plain SSR at `localhost:8765` with bearer token auth
- No SPA. Markdown rendering, file diffs, tool output — nothing more.

**Voice** (optional feature flag):
- whisper.cpp STT (whisper-tiny, push-to-talk via Ctrl+Space)
- Piper TTS for agent responses
- Exit: voice-typed tasks work; model runs on CPU at ≥5x realtime

---

### v3.0 — Proactivity + Autonomous Mode
**Signal watchers** (`crates/helm-watch/`):
- File changes, disk %, OOM events, new emails (via MCP), git commits, calendar events
- Each signal maps to configurable agent trigger

**Suggestion engine:**
- Periodic background analysis of episodes + system state
- Suggestions appear in TUI; user approves/dismisses/"never suggest"
- Default: very conservative; user opts in to higher frequency

**Autonomous mode**: ⭐ NOVEL
- Opt-in; skill must have ≥3 gold examples + capability marked `auto:true`
- Maps signals → skills → automatic execution without prompting
- Comprehensive audit trail; `helm undo last` reverts last autonomous action
- Exit: disk hits 90% → cleanup skill runs, notification shows what was done

---

### v3.1 — Skill Exchange ⭐ NOVEL
- `helm skill share <name>` → signed JSON manifest (with gold examples, capabilities, origin)
- `helm skill install <url>` → TUI review screen shows exact code before installing
- No central marketplace — decentralized, user-to-user
- Exit: install a skill from URL, inspect manifest, approve, re-invoke successfully

---

### v3.2–v3.4 — Multi-Machine + Team
**Multi-machine memory sync:**
- CRDT-based sync; HELM-hosted relay or self-hosted relay binary
- Encrypted in transit + at rest; full data export command
- Default off; opt-in

**Team mode:**
- Per-user memory partitions; team policy file; RBAC

---

### v4.0 — Cloud Control Plane (Monetization)
- Hosted relay for multi-machine sync + team + audit retention + RBAC + web UI
- Free product: the daemon. Paid product: coordination at scale.
- Apache 2.0 open core forever. CLA from contributors.
- Only build when there are customers asking for it.

---

## Novel Features (Moat — As of May 2026)

| # | Feature | Phase |
|---|---------|-------|
| 1 | Source-taint capability tokens (prompt injection solved at type level) | v0.2 ✅ |
| 2 | SSH-native multi-target operation | v1.5 |
| 3 | Plan caching by goal embedding (repeat tasks = 0 planner tokens) | v1.2 |
| 4 | Voyager-style skill learning from your own episodes | v1.3 |
| 5 | User-style learning without fine-tuning | v1.3 |
| 6 | Disagreement protocol with third-agent triangulation | v2.0 |
| 7 | Decentralized skill exchange (no marketplace attack surface) | v3.1 |
| 8 | Autonomous mode gated on gold-example skill quality | v3.0 |

---

## Critical Path (do in order, no skipping)

1. ✅ v0.1–v0.8 complete
2. 🔄 100-run suite → tag v1.0.0-rc1
3. v1.0 public release (polish, HN post)
4. v1.1 git tool + MCP client
5. v1.2 graph memory + plan cache + procedural memory
6. v1.3 skill learning + meta-learning + user-style learning
7. v1.4 cancellation + sandbox
8. v1.5 SSH/remote
9. v2.0 multi-agent + disagreement protocol + public release #2
10. v2.5 notifications + scheduler + voice + web UI
11. v3.0 signal watchers + autonomous mode
12. v3.1 skill exchange
13. v3.2–v4.0 multi-machine + team + cloud

**If you add v1.1+ features before v1.0 ships, you ship nothing. Resist.**

---

## What Will Never Be In HELM

- GUI computer-use (fragile; your hardware can't run vision models at scale)
- Windows/macOS before v2.0
- Fine-tuning or weight updates of any model
- Self-modification of agent source code
- Agent-to-agent negotiation across machines (research-grade, 2027+)
- Centralized skill marketplace
- SaaS before v4.0
