# HELM — Canonical Product Roadmap
# rev 2026-05-13

---

## Product Thesis

HELM is a trust-bound Linux operations assistant for diagnostics, documentation,
and approved changes. It earns adoption by being useful without write access
first, then earns permission for scoped, reversible, audited actions.

It is not an autonomous server manager. It is not a replacement for sed, awk,
or shell scripts. It is the assistant that sits between the operator and the
terminal — reading, explaining, planning, and acting only when explicitly told
to, with a full audit trail, rollback path, and blast-radius estimate shown
before anything destructive runs.

---

## Versioning Convention

- v0.x  Pre-alpha foundation (complete)
- v1.0  Public release — open source on GitHub
- v1.x  Trust-first hardening and real operator value
- v2.0  Governed multi-agent verification release
- v2.5  Interface expansion (notifications, scheduler, voice)
- v3.x  Proactivity and multi-machine
- v4.0  Hosted control plane and monetization

---

## Audience

### Primary (ship for these people)
- Self-hosters managing Proxmox, Docker, Compose, nginx, Grafana stacks
- Solo Linux operators who maintain servers between other work
- Homelab users who lose context between sessions
- Cautious SREs who want read-only incident support and better runbooks
- Small teams with stale documentation and repeated handoff problems

### Not Primary Yet
- People who want full autopilot
- Users unwilling to install terminal tools
- High-compliance enterprises needing central policy systems
- Teams expecting a SaaS control plane

---

## Why Users Still Won't Choose This — Brutal Honest Audit

Before shipping anything, every angle of rejection must be named. This section
exists so that no objection is a surprise and every release directly closes
one of these gaps.

### Trust objections
1. "LLMs will eventually screw up. It's autocomplete with confidence."
2. "I don't know what it's doing under the hood."
3. "Who takes the blame when it deletes something?"
4. "I've been burned by 'helpful automation' before."
5. "I'd only trust this inside a VM with nothing real."
6. "It acts like a junior engineer who rushes without understanding."
7. "It talks like it knows everything, then gets it wrong."
8. "I can't audit what it actually did."
9. "I have no rollback if it makes a bad change."
10. "It doesn't understand my system's context."

### Privacy and data objections
11. "Data can't stay local if you're sending prompts to an API."
12. "My server configs and log contents are sensitive."
13. "I don't want my infrastructure topology sent to Anthropic."
14. "Even Ollama models are large, noisy, and energy-expensive."
15. "I don't trust telemetry defaults."

### Utility objections
16. "Why not just use Claude directly in a browser?"
17. "I already have Hermes / Aider / OpenCode."
18. "sed/awk/grep does what I need without a 10MB binary."
19. "The setup cost is not worth it for occasional tasks."
20. "I'm faster doing it myself than explaining it to an agent."
21. "It doesn't know my specific service configs."
22. "It hallucinates package names and command flags."
23. "One wrong restart and my homelab is down for the night."
24. "It's too clunky to justify for simple tasks."
25. "There's no killer feature I can't get elsewhere."

### Category objections
26. "The Kubernetes Helm name conflict causes confusion immediately."
27. "r/sysadmin will remove your posts as self-promotion."
28. "This is vibe-coding for sysadmins. It creates spaghetti, not fixes."
29. "The AI agent category is already crowded and I'm tired of it."
30. "Open source agents all abandon maintenance after 6 months."
31. "No Windows, no macOS — too narrow to recommend to my team."
32. "If it breaks prod, no one will believe me that an AI did it."
33. "The self-hosted community is deeply skeptical of cloud-adjacent tools."
34. "I can't show my security team an AI with shell access."

### Every one of these is addressed in the roadmap below.

---

## How Each Objection Is Closed

| Objection cluster | Closed by |
|---|---|
| Rushes / junior engineer | Plan-first default, evidence report before action, verifier role |
| Can't audit | HMAC audit chain, audit verify command, per-episode audit slice |
| No rollback | Change sets, snapshots, helm rollback |
| Doesn't understand my system | AGENTS.md context file, graph memory, per-project context |
| API sends my data | Explicit local-vs-API boundary messaging, Ollama default option |
| Setup cost too high | Single curl install, diagnose works immediately with no config |
| Why not Claude/Hermes directly | Read-only diagnose mode, structured evidence, Rust binary speed, audit chain, remote multi-target |
| sed/awk is faster | True for single commands. HELM is for sessions: correlate logs + service state + config + history across many commands |
| Name conflict | Binary rename or alias before v2.0 marketing push |
| Spaghetti / vibe-coding | Deterministic FSM supervisor, not LLM-directed execution; verifier checks evidence before marking done |
| Category fatigue | Lead with read-only diagnose — a concrete job, not "AI agent" |
| Security team won't approve | Capability gates, taint system, per-path allowlist, policy.toml, audit export |

---

## Positioning: What To Lead With

**Do lead with:**
- Read-only diagnostics that cannot write anything
- Evidence before every action: what it read, what it found, what it assumes
- Approved, auditable, bounded changes
- Recoverable operations with rollback
- Runbooks and handoff docs generated from real system state

**Do not lead with:**
- AI server automation
- Remote patching at scale
- YOLO mode
- Multi-agent autonomy
- Broad shell access
- MCP as a headline feature

**Safe public examples:**
```sh
helm diagnose "what is eating my disk?"
helm --read-only "summarize failing services and recent journal errors"
helm runbook generate --dry-run
helm handoff
```

**Unsafe examples — do not put in README or demos:**
```sh
helm --remote prod-1 "patch nginx and restart"
helm --yes "fix my server"
```

---

## Hard Non-Goals (permanent)

- GUI vision-based computer-use
- Full autopilot server management without human approval
- Hosted SaaS before strong local trust and usage exist
- Centralized skill marketplace
- Fine-tuning or weight modification of any model
- Self-modifying agent source code
- Windows / macOS before v2.0
- Any action that bypasses capability gates, taint checks, redaction, or audit integrity

---

## PART 1 — Complete Feature Inventory

Ship-rank legend: MUST = v1.0 required; SHOULD = v2.0 required; COULD = post v2.0; NICE = anytime; DONE = shipped; SHIPPED(vX) = shipped in that version.

### A. Installation & First-Run

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| A1 | Single curl \| sh install, ~10MB static Rust binary, no runtime | | Crush, OpenCode | MUST |
| A2 | Package distributions: brew, aur, nix, apt, scoop, choco, mise | | OpenCode | SHOULD |
| A3 | helm with no args starts TUI in cwd; no config triggers onboarding | | Crush, OpenCode | MUST |
| A4 | First-run wizard: provider key, default model, tools to enable, capability defaults, telemetry opt-in | | Cowork, Crush | MUST |
| A5 | helm init writes ~/.helm/config.toml and ./AGENTS.md | | OpenCode, Claude Code | MUST |
| A6 | helm upgrade self-update with rollback to previous binary on failure | | Claude Code | SHOULD |
| A7 | helm uninstall cleans config, db, logs with --keep-data flag | | None — novel | NICE |
| A8 | Auto-detect provider from env vars on first launch | | Codex CLI | DONE |
| A9 | OAuth login for providers that support it | | Crush, OpenCode | SHOULD |
| A10 | helm doctor system check with green/red per subsystem | | Claude Code | DONE-extend |

### B. CLI Surface — Top-Level Commands

| # | Command | Purpose | Reference | Rank |
|---|---|---|---|---|
| B1 | helm | Open TUI in cwd | OpenCode/Crush | MUST |
| B2 | helm "\<task\>" | Non-interactive task, print result, exit | OpenCode -p | MUST |
| B3 | helm run \<task\> [-q] [-f json] [--no-stream] | Like B2 with options | OpenCode | MUST |
| B4 | helm init [--minimal] | Init config + AGENTS.md | OpenCode | MUST |
| B5 | helm doctor [--json] | System check | Claude Code | DONE |
| B6 | helm models [--provider X] [--installed] | List models with capabilities | Claude Code | DONE-extend |
| B7 | helm episodes [--limit N] [--outcome X] [--since DATE] | List past episodes | Novel | DONE-extend |
| B8 | helm replay \<id\> | Replay past episode transcript | Aider, OpenCode | DONE-extend |
| B9 | helm export \<id\> [--format md\|json] | Export episode to file | OpenCode | SHOULD |
| B10 | helm import \<file-or-url\> | Import shared episode/skill | OpenCode | COULD |
| B11 | helm permissions {list,grant,revoke,reset} | Manage capabilities | Novel | DONE-extend |
| B12 | helm audit {tail,verify,export} | Audit log operations | Cowork OTel | DONE-extend |
| B13 | helm skills {list,show,run,edit,delete,share,install,verify} | Skill management | Cowork, OpenClaw | SHOULD |
| B14 | helm config {get,set,edit,validate,path} | Config management | OpenCode | MUST |
| B15 | helm mcp {add,list,remove,test,run} | MCP server management | Claude Code | SHOULD |
| B16 | helm sessions {list,delete,export,resume} | Session management | OpenCode, Crush | SHOULD |
| B17 | helm stats [--since DATE] | Token usage, cost, success rate | OpenCode, Crush | SHOULD |
| B18 | helm undo [N] | Undo last N agent-applied changes | OpenCode, Crush | SHOULD |
| B19 | helm redo [N] | Redo undone changes | OpenCode | SHOULD |
| B20 | helm schedule {add,list,remove} | Scheduled tasks | Novel | COULD (v2.5) |
| B21 | helm watch {add,list,remove} | Signal watchers | Novel | COULD (v3.0) |
| B22 | helm remote {add,list,test,remove} | Remote target management | Novel | SHOULD (v1.5) |
| B23 | helm sync {push,pull,status,reset} | Multi-machine memory sync | Novel | COULD (v3.2) |
| B24 | helm tunnel | Reverse tunnel for mobile dispatch | Tailscale-style | COULD (v2.5) |
| B25 | helm serve [--host X] [--port N] | Headless daemon mode | OpenCode | SHOULD |
| B26 | helm tui --attach \<host:port\> | Attach TUI to remote daemon | OpenCode | COULD |
| B27 | helm version, helm help, helm completion {bash,zsh,fish} | Standard CLI | All | MUST |
| B28 | helm diagnose "\<question\>" | Safe read-only entrypoint, cannot write | Novel | SHIPPED (v1.6) |
| B29 | helm runbook generate [--dry-run] | Generate runbook from system state | Novel | MUST (v1.8) |
| B30 | helm handoff | Generate handoff doc for next operator | Novel | MUST (v1.8) |
| B31 | helm rollback [\<changeset-id\>] | Rollback supported change sets | Novel | MUST (v1.7) |
| B32 | helm trust-report | Show provider mode, storage, sandbox, audit status | Novel | SHIPPED (v1.6) |
| B33 | helm changeset {list,show,rollback} | Inspect and manage change sets | Novel | MUST (v1.7) |

### C. CLI Flags

| # | Flag | Purpose | Reference | Rank |
|---|---|---|---|---|
| C1 | --provider \<id\> | Override provider for one run | OpenCode | DONE |
| C2 | --model \<id\> | Override model | OpenCode | DONE |
| C3 | --system-prompt and --append-system-prompt | System prompt control | Claude Code | SHOULD |
| C4 | --max-iterations, --max-tokens, --max-time | Budget overrides | Novel | DONE |
| C5 | --remote \<name-or-host\> | SSH/remote target | Novel | SHOULD (v1.5) |
| C6 | --sandbox | Bubblewrap sandbox | Cowork | SHOULD (v1.4) |
| C7 | --yes, --yolo, --dangerously-skip-permissions | Auto-approve | Crush, Claude Code | MUST |
| C8 | --read-only | Plan mode, no writes | OpenCode, Claude Code | MUST |
| C9 | -p \<task\>, --print | Non-interactive stdout | OpenCode | DONE |
| C10 | -f json\|md\|text | Output format | OpenCode | SHOULD |
| C11 | --no-stream | Disable token streaming | All | SHOULD |
| C12 | --quiet, --verbose, --debug | Log levels | All | DONE |
| C13 | --db-path, --config, --config-dir | Override paths | OpenCode | DONE |
| C14 | --worktree \<name\> | Named git worktree | Claude Code | COULD |
| C15 | --resume [\<id\>] or --continue | Resume last/specific session | Claude Code, Aider | SHOULD |
| C16 | --notify | Notify on completion | Novel | COULD (v2.5) |
| C17 | --log-file \<path\> | Override log location | | DONE |
| C18 | --no-color, --color always\|auto\|never | Color control | All | SHOULD |
| C19 | --dry-run | Print exact commands/actions, no execution | Novel | SHIPPED (v1.6) |
| C20 | --evidence | Show evidence report before any action | Novel | SHIPPED (v1.6) |

### D. TUI — Layout Regions

| # | Region | Detail | Reference | Rank |
|---|---|---|---|---|
| D1 | Header bar (1 line) | logo · provider/model · session name · state · time · token meter · cost meter | Crush, Claude Code | MUST |
| D2 | Chat scroll area | Speaker labels, inline tool summaries, markdown, code blocks with left rule | Claude Code | MUST |
| D3 | Tool inline summaries | One-line ✓ ran 'ls ~' (47 lines, 12ms) collapsed; expandable | Claude Code | MUST |
| D4 | Live diff display | Side-by-side or unified diff inline, syntax-highlighted | Crush | SHOULD |
| D5 | Multiline input | Default 1 line, grows to ~7; dim placeholder | Claude Code | MUST |
| D6 | Footer hint strip | Context-aware key hints | Claude Code | MUST |
| D7 | Right sidebar (toggleable) | Tool history list with status icons + truncated I/O | Codex/Crush | MUST |
| D8 | Modal overlay | Centered, bordered; for tool details, permission prompts, config | Crush | MUST |
| D9 | Plan panel | DAG progress when in plan mode: pending/running/done | Novel | SHOULD |
| D10 | Cost ticker | Running USD cost; turns red over budget | Crush | SHOULD |
| D11 | Status bar (bottom-left) | mode · model · vim-state | Claude Code | SHOULD |
| D12 | Toast notifications | Transient bottom-right | Novel | NICE |
| D13 | Background-task indicator | Spinner for non-blocking tasks | None | COULD |
| D14 | Evidence panel | Before permission prompt: shows sources, findings, assumptions, blast radius | Novel | SHIPPED (v1.6) |

### E. TUI — Modes

| # | Mode | Behavior | Reference | Rank |
|---|---|---|---|---|
| E1 | Chat (default) | Read freely, write/exec ask permission | Claude Code | MUST |
| E2 | Plan | Read-only; agent proposes plan, no writes execute | OpenCode, Claude Code | MUST |
| E3 | Auto-accept | Tools execute without prompts; capabilities still enforced | Claude Code | MUST |
| E4 | YOLO | All permissions auto-accepted, gated behind --yolo + warning | Crush | SHOULD |
| E5 | Replay (read-only) | View past episode in TUI as if live | Novel | SHOULD |
| E6 | Focus | Hide chrome/sidebars/toasts, just chat + input | Claude Code | NICE |
| E7 | Cycle modes with Shift+Tab (visual indicator in footer) | | All | MUST |
| E8 | Diagnose (read-only locked) | Cannot call write tools at the OS level | Novel | SHIPPED (v1.6) |

### F. TUI — Keybinds

| # | Key | Action | Reference | Rank |
|---|---|---|---|---|
| F1 | Enter | Send | All | MUST |
| F2 | Shift+Enter / Ctrl+J / \\\<Enter\> | Newline | Claude Code | MUST |
| F3 | Ctrl+G | Open external editor ($EDITOR) for prompt | Claude Code, OpenCode | SHOULD |
| F4 | Ctrl+T | Toggle tool sidebar | Codex | MUST |
| F5 | Ctrl+H | History panel | Claude Code | MUST |
| F6 | Ctrl+L | Clear current chat (db row stays) | Claude Code | MUST |
| F7 | Ctrl+P | Command palette (fuzzy search every command) | Crush | MUST |
| F8 | Ctrl+R | Replay last assistant message | None | NICE |
| F9 | Ctrl+C | Cancel current step (or quit if idle, with confirm) | Claude Code | MUST |
| F10 | Ctrl+D (empty input) | Quit | Aider | MUST |
| F11 | Ctrl+X Ctrl+K | Kill all background agents | Claude Code | SHOULD |
| F12 | Esc | Close modal → close sidebar → close history | Claude Code | MUST |
| F13 | ↑/↓ (empty input) | Cycle previous prompts | All | MUST |
| F14 | ↑/↓ (sidebar focused) | Move tool selection | All | MUST |
| F15 | Tab (input focused) | Autocomplete: slash command, @-file, /skill, model name | Claude Code, OpenCode | MUST |
| F16 | ? | Help overlay | Crush, Claude Code | MUST |
| F17 | / (line start) | Slash command picker | Claude Code | MUST |
| F18 | @ (line start or mid-line) | File/dir picker (fuzzy) | Claude Code, OpenCode | MUST |
| F19 | # (line start) | Add note to memory | Claude Code | SHOULD |
| F20 | ! (line start) | Bash mode — run shell command in cwd | Claude Code | SHOULD |
| F21 | Ctrl+B | Toggle bash mode persistently | Claude Code | NICE |
| F22 | Mouse: scroll, click, copy-on-select | Standard | OpenCode | SHOULD |
| F23 | Ctrl+O | Toggle verbose transcript view | Claude Code | SHOULD |
| F24 | All keybinds remappable via ~/.helm/keybindings.json | | Claude Code | SHOULD |
| F25 | /vim | Toggle vim-mode for input | Claude Code | NICE |

### G. TUI — Slash Commands

| # | Command | Purpose | Reference | Rank |
|---|---|---|---|---|
| G1 | /help | List all commands | All | MUST |
| G2 | /clear | Clear visible conversation (db row stays) | Claude Code | MUST |
| G3 | /compact [hint] | Summarize conversation to reclaim context | Claude Code, OpenCode | SHOULD |
| G4 | /new or /session new | New session, fresh context | OpenCode | MUST |
| G5 | /resume [id] | Resume past session | Claude Code | MUST |
| G6 | /sessions | Picker | Claude Code, OpenCode | MUST |
| G7 | /model [id] | Switch model mid-session | Claude Code | MUST |
| G8 | /effort {low,medium,high,xhigh} | Reasoning effort | Claude Code | SHOULD |
| G9 | /mode {chat,plan,auto,yolo,diagnose} | Switch mode | Claude Code | SHOULD |
| G10 | /init | Generate AGENTS.md for current dir | OpenCode, Claude Code | MUST |
| G11 | /agents | Show role/sub-agent assignments | OpenCode | SHOULD |
| G12 | /tools | List loaded tools and capabilities | Novel | MUST |
| G13 | /skills | List skills, run /skill \<name\> | Cowork | SHOULD |
| G14 | /mcp | Manage MCP servers | Claude Code | SHOULD |
| G15 | /permissions | Open permissions modal | Novel | MUST |
| G16 | /audit | Tail audit log inline | Novel | SHOULD |
| G17 | /undo [N] /redo [N] | Revert agent edits | OpenCode | SHOULD |
| G18 | /diff [path] | Show diff of last edits | OpenCode | SHOULD |
| G19 | /cost /usage /stats | Cost & usage panel | Claude Code, Crush | SHOULD |
| G20 | /share | Generate shareable file of conversation | OpenCode | NICE |
| G21 | /export [format] | Export to file | OpenCode | SHOULD |
| G22 | /config [key=val] | Edit config inline | Crush | NICE |
| G23 | /theme [name] | Switch theme | Claude Code, OpenCode | NICE |
| G24 | /keybindings | Edit keybinding file | Claude Code | NICE |
| G25 | /doctor | System check inline | Claude Code | MUST |
| G26 | /recap | Summarize what has happened so far | Claude Code | NICE |
| G27 | /btw \<question\> | Side-channel question without derailing main thread | Claude Code | SHOULD |
| G28 | /think /thinking | Toggle visible chain-of-thought | Novel | NICE |
| G29 | /remote \<name\> | Switch active target | Novel (v1.5) | SHOULD |
| G30 | /quit /exit /q | Exit | OpenCode | MUST |
| G31 | Custom user commands at ~/.helm/commands/\<name\>.md with YAML frontmatter | | Claude Code, OpenCode | SHOULD |
| G32 | /diagnose "\<question\>" | Run read-only diagnose inline | Novel | SHIPPED (v1.6) |
| G33 | /evidence | Show evidence report for last action | Novel | SHIPPED (v1.6) |
| G34 | /runbook generate | Generate runbook inline | Novel | SHOULD (v1.8) |
| G35 | /changeset | Show current change set before commit | Novel | SHOULD (v1.7) |

### H. Permissions — Fine-Grained Surface

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| H1 | Three-tier system: read=auto, exec=prompt-once, write=prompt-each | | Claude Code | DONE |
| H2 | Capability TTL (15m / 1h / 24h / always / once) | | Cowork | DONE |
| H3 | Source-taint check at every escalation | | Novel — moat | DONE |
| H4 | Per-pattern shell allowlist (e.g., Bash(git status:*)) | | Claude Code | SHOULD |
| H5 | Per-domain network allowlist for browser/http tools | | Cowork | SHOULD |
| H6 | Per-path read/write allowlist | | Claude Code | DONE |
| H7 | .helmignore file (like .gitignore but for tool reach) | | Crush | SHOULD |
| H8 | TUI permission modal: tool, exact input, taint, blast radius, Allow once / Allow 15m / Allow always / Deny | | All | MUST |
| H9 | "Why am I being asked?" link in modal explains capability + taint | | Novel | NICE |
| H10 | Per-skill auto-approval flag (auto:true) gated by gold examples | | Novel | COULD |
| H11 | Confirmation policy file (policy.toml) | | Novel | DONE |
| H12 | Capability inheritance to sub-agents (least privilege) | | Novel | SHOULD (v2.0) |
| H13 | Audit-log filter for deny events visible in helm doctor | | None | SHOULD |
| H14 | Diagnose mode capability lock: write tools cannot be registered in diagnose mode | | Novel | SHIPPED (v1.6) |

### I. Audit

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| I1 | HMAC-chained append-only log | | Novel — moat | DONE |
| I2 | helm audit verify walks chain, reports breaks with line number | | Novel | DONE |
| I3 | OpenTelemetry traces option (OTLP) | | Cowork | SHOULD |
| I4 | Audit export to JSON / CSV / SIEM-compatible JSONL | | Cowork | SHOULD |
| I5 | Capture rule: never log API keys, secrets, sensitive file contents | | Novel | MUST |
| I6 | Audit query: helm audit grep \<regex\>, helm audit since \<date\> | | Novel | SHOULD |
| I7 | Per-episode audit slice: helm audit --episode \<id\> | | Novel | SHOULD |
| I8 | Anomaly hints: agent ran for >2× usual duration, spike in capability use | | Novel | COULD |
| I9 | Per-changeset audit slice showing exact mutations in order | | Novel | MUST (v1.7) |

### J. Memory — Episodic, Graph, Procedural

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| J1 | Episode log per task with full step transcripts | | All | DONE |
| J2 | Graph memory: nodes (paths, urls, processes, services, packages), edges | | Manus | SHOULD (v1.2) |
| J3 | Embedding index (sqlite-vec) over node descriptions and goal strings | | Manus, Comet | SHOULD (v1.2) |
| J4 | Procedural memory: nightly clustering → injectable templates | | Novel | COULD (v1.2) |
| J5 | Plan cache: cosine match → reuse plan, skip planner | | Novel | COULD (v1.2) |
| J6 | TTL + decay + helm gc | | None | SHOULD |
| J7 | helm memory query "\<question\>" | | Manus | COULD |
| J8 | Memory export/import | | Novel | SHOULD |
| J9 | Context-window compaction (done in v0.1.5) | | Claude Code | DONE |
| J10 | Per-project context files: AGENTS.md, HELM.md, CLAUDE.md | | Crush, Claude Code | MUST |
| J11 | # quick memory | | Claude Code | SHOULD |
| J12 | Memory namespacing per project / target / user | | Novel | COULD |

### K. Skills

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| K1 | Built-in skills bundled (docker-restart, git-status, nginx-deploy starter) | Loaded via include_str! at compile time | Cowork | SHIPPED (v1.5) |
| K2 | Skill manifest format: name, description, schema, code, gold examples, capabilities, version | skill.toml | Cowork | SHIPPED (v1.5) |
| K3 | Voyager-style auto-extraction from successful episodes | | Novel — moat | COULD (v1.3) |
| K4 | Skill versioning + gold-example regression tests | | Novel | COULD (v1.3) |
| K5 | Decentralized skill exchange (signed manifest URL) | | Novel — moat | COULD (v3.1) |
| K6 | Skill review modal: code visible, capabilities listed, signature checked | | Novel | COULD (v3.1) |
| K7 | Skill execution sandboxed (separate Python venv per skill) | | None | COULD |
| K8 | helm skills run \<name\> --input '{…}' | --dry-run available | Novel | SHIPPED (v1.5) |
| K9 | Custom user-written skills in ~/.helm/skills/\<name\>/ | | OpenClaw, Cowork | SHOULD |
| K10 | Disable specific skills per project via .helmignore | | None | NICE |

### L. Tools

| # | Tool | Capability | Reference | Rank |
|---|---|---|---|---|
| L1-9 | shell, fs_read, fs_write, process, service, package, network, disk, logs, browser | | Multiple | DONE |
| L10 | git | git.read, git.write | OpenCode/Crush | SHOULD (v1.1) |
| L11 | env | env.read | None | DONE |
| L12 | firewall | firewall.read/write | None | COULD |
| L13 | cron | cron.read/write | None | COULD |
| L14 | http (generic GET/POST with allowlist) | net.http | OpenCode/Crush | SHOULD |
| L15 | grep / glob via ripgrep | ignore-aware | OpenCode, Crush | SHOULD |
| L16 | edit (LSP-aware diff edits) | | Crush, OpenCode | COULD (post-v2) |
| L17 | LSP integration | | OpenCode/Crush | COULD |
| L18 | docker (container ops) | docker.read/write | Novel | COULD |
| L19 | kubernetes (kubectl wrap) | k8s.read/write | None | COULD |
| L20 | systemd-machine (containerd, machinectl) | | None | COULD |
| L21 | journalctl with structured filters | | Novel | DONE-extend |
| L22 | aws/gcp/azure cli wrappers | | None | COULD |
| L23 | ssh tool (run on remote without full daemon) | | Novel — moat | SHOULD (v1.5) |
| L24 | scp / rsync wrappers | | None | COULD |
| L25 | mcp_call (proxy to any registered MCP server tool) | | Claude Code | SHOULD (v1.1) |
| L26 | docker/compose read-only diagnostics | | Novel | MUST (v1.9) |
| L27 | proxmox read-only query | | Novel | SHOULD (v1.9) |
| L28 | backup verification tool | | Novel | SHOULD (v1.9) |

### M. Multi-Agent + Supervisor

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| M1 | Plan DAG (typed Rust struct) | | Novel | DONE |
| M2 | Deterministic FSM supervisor (NOT LLM) | | Novel — moat | DONE |
| M3 | Roles: Triager / Planner / Executor / Verifier / Retro | | Cowork | DONE |
| M4 | Per-role model assignment (ProviderRouter) | | Computer | DONE |
| M5 | Parallel sub-agent execution on independent DAG nodes | | Cowork, Manus | SHOULD (v2.0) |
| M6 | Cross-step shared memory scratchpad | | Manus | SHOULD (v2.0) |
| M7 | Disagreement protocol (third-agent triangulation) | | Novel — moat | COULD (v2.0) |
| M8 | Verifier with structured Evidence enum | | Novel — moat | DONE |
| M9 | Budget enforcement per step + per task | | OpenCode | DONE |
| M10 | Replan-on-failure threshold | | None | DONE-extend |
| M11 | Sub-agent capability inheritance (least privilege) | | Novel | SHOULD (v2.0) |
| M12 | Plan visualization in TUI | | Cowork | SHOULD |
| M13 | Task tree / TODO tool | | OpenCode, Manus | SHOULD |

### N. Providers + Model Surface

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| N1 | Provider registry: anthropic, openai, gemini, groq, openrouter, nvidia-nim, ollama, generic openai-compat | | OpenCode | DONE |
| N2 | Auto-detect from env | | All | DONE |
| N3 | Per-role + per-model routing | | Computer | DONE |
| N4 | Tool-call format recovery | | Novel | DONE |
| N5 | Model-quirks table | | Novel | DONE |
| N6 | "Best model for task" router | | Novel | COULD (v1.3) |
| N7 | Cost tracking per provider/model | | Crush, OpenCode | SHOULD |
| N8 | Per-provider rate-limit handling with friendly errors | | OpenCode | SHOULD |
| N9 | Streaming token output | | All | SHOULD |
| N10 | OAuth login for providers that support it | | OpenCode | COULD |
| N11 | Local-first model option (Ollama default) | | Crush, OpenCode | DONE |
| N12 | Model picker UI (Ctrl+P → Switch Model) | | Crush | MUST |

### O. Observability

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| O1 | Structured logs via tracing to ~/.helm/helm.log | | | DONE |
| O2 | Per-iteration debug events with token counts | | | DONE |
| O3 | OpenTelemetry exporter (OTLP) | | Cowork | SHIPPED (v1.5) |
| O4 | Prometheus metrics endpoint when serving | | Novel | COULD |
| O5 | helm doctor --json for monitoring scrape | | | DONE |
| O6 | Token-counter heartbeat in TUI | | Crush, Claude Code | SHOULD |
| O7 | Cost meter per session | | Crush | SHOULD |
| O8 | Session recap on resume | | Claude Code | SHOULD |
| O9 | helm stats daily/weekly rollups | | OpenCode | SHOULD |
| O10 | Failure mode telemetry: provider errors, parse failures, validation rejects | | None | SHOULD |

### P. Hooks & Extensibility

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| P1 | Lifecycle hooks: PreToolUse, PostToolUse, PrePlan, PostPlan, OnEpisodeStart/End | All 6 stages wired | Claude Code, Crush | SHIPPED (v1.5) |
| P2 | Hook config in ~/.helm/hooks.toml | | Claude Code | SHOULD |
| P3 | Hook env vars: HELM_TOOL, HELM_INPUT, HELM_EPISODE_ID, HELM_TARGET, HELM_CWD | | Claude Code | SHOULD |
| P4 | User custom commands: ~/.helm/commands/\<name\>.md with frontmatter | | Claude Code, OpenCode | SHOULD |
| P5 | User custom skills: ~/.helm/skills/\<name\>/skill.toml | | OpenClaw, Cowork | SHOULD |
| P6 | User custom agents (roles): ~/.helm/agents/\<name\>.toml | | OpenCode, Crush | COULD |
| P7 | User custom modes: ~/.helm/modes/\<name\>.toml | | OpenCode | NICE |
| P8 | Plugin manifest with declared capabilities | | Claude Code | COULD |
| P9 | Tool plugin SDK (Rust trait + dynamic loader, or stdio JSON-RPC) | | Crush MCP, OpenClaw | COULD |
| P10 | First-party hook examples: prettier-on-edit, lint-on-write | | Claude Code | NICE |

### Q. Networking & Remote

| # | Feature | Details | Reference | Rank |
|---|---|---|---|---|
| Q1 | --remote \<host\> SSH-mode | | Novel — moat | SHOULD (v1.5) |
| Q2 | --remote agent-on-remote mode (NDJSON over SSH) | Line-delimited JSON over SSH stdout | Novel | SHIPPED (v1.5) |
| Q3 | helm bootstrap \<host\> auto-install | | Novel | COULD (v1.5) |
| Q4 | Tailscale-aware target resolution | | None | NICE |
| Q5 | Multi-target broadcast (same task to N hosts) | | Novel | COULD (v2.0) |
| Q6 | Per-target audit log (separate SQLite shard per host) | | Novel | SHIPPED (v1.5) |
| Q7 | helm tunnel reverse-tunnel daemon | | None | COULD (v2.5) |
| Q8 | Mobile dispatch PWA | | Cowork | COULD (v2.5) |
| Q9 | Bearer token auth on serve mode | | OpenCode | SHOULD |
| Q10 | mTLS option for production deployments | | None | NICE |

### R. Notifications, Scheduling, Proactivity

| # | Feature | Rank |
|---|---|---|
| R1 | Desktop notifications (libnotify) | COULD (v2.5) |
| R2 | Telegram channel | COULD (v2.5) |
| R3 | Slack DM | COULD (v2.5) |
| R4 | Email | COULD (v2.5) |
| R5 | Per-task --notify flag | COULD |
| R6 | Scheduled tasks via systemd-timer | COULD (v2.5) |
| R7 | Signal watchers: file/email/disk/load/oom | COULD (v3.0) |
| R8 | Suggestion engine | COULD (v3.0) |
| R9 | Autonomous mode (gold-example skills only, heavily gated) | COULD (v3.0) |
| R10 | helm undo last-autonomous revert | COULD (v3.0) |

### S. Sharing & Collaboration

| # | Feature | Rank |
|---|---|---|
| S1 | helm share session → signed JSON file or short URL | NICE |
| S2 | helm skill share → signed manifest | COULD (v3.1) |
| S3 | helm skill install \<url\> with signature verification | COULD (v3.1) |
| S4 | Multi-machine sync (CRDT, opt-in, encrypted) | COULD (v3.2) |
| S5 | Self-hosted relay binary helm-relay | COULD (v3.2) |
| S6 | Hosted relay (paid tier) | COULD (v4.0) |
| S7 | Per-user / team RBAC | COULD (v3.4) |

### T. Voice + Extended Interfaces

| # | Feature | Rank |
|---|---|---|
| T1 | whisper.cpp STT push-to-talk | COULD (v2.1) |
| T2 | Piper TTS for replies | NICE |
| T3 | Wake word | NICE (v3+) |
| T4 | Web UI (local SSR, bearer token) | COULD (v2.4) |
| T5 | Read-only public share page | NICE |
| T6 | iOS / Android dispatch app | COULD (v2.5) |

### U. Quality of Life

| # | Feature | Rank |
|---|---|---|
| U1 | Themes (dark/light/dim/solarized/dracula/custom) | SHOULD |
| U2 | Diff style: auto/stacked/side-by-side | SHOULD |
| U3 | Mouse support | SHOULD |
| U4 | Auto-update with notify mode | SHOULD |
| U5 | Snapshots for every agent file edit; helm undo walks them | SHOULD |
| U6 | Per-project worktree mode | COULD |
| U7 | Sessions auto-save every N seconds | SHOULD |
| U8 | Crash-resume: next start offers to resume | SHOULD |
| U9 | Last assistant text always shown | DONE |
| U10 | Smart context-low warning | SHOULD |
| U11 | Easter egg: helm hatch → terminal pet | NICE |
| U12 | Status bar shows git branch + dirty state | NICE |
| U13 | Session naming: auto-named by first goal, renamable | SHOULD |
| U14 | helm in non-empty dir asks "use as project context?" first time | SHOULD |
| U15 | TUI degrades gracefully on 80×24 | MUST |
| U16 | Synchronized output mode for terminals that don't auto-detect | SHOULD |
| U17 | CJK / wide-char safe rendering | SHOULD |
| U18 | helm completion {bash,zsh,fish} | MUST |
| U19 | XDG-compliant paths | SHOULD |
| U20 | First-run privacy disclosure: explicit yes/no on telemetry | MUST |

### V. Documentation

| # | Feature | Rank |
|---|---|---|
| V1 | README with honest "use HELM if / don't use HELM if" table | MUST |
| V2 | Cheat sheet (every command, flag, keybind) | SHOULD |
| V3 | ADR directory | SHOULD |
| V4 | 90-second demo GIF/video on README | MUST |
| V5 | docs/ site (mdBook or Astro) | SHOULD |
| V6 | CONTRIBUTING.md, code of conduct, security policy | MUST |
| V7 | THREAT_MODEL.md — prompt-injection story | SHOULD |
| V8 | Release notes command | NICE |
| V9 | helm tour interactive in-TUI tour | COULD |
| V10 | Trust ladder documentation: Level 0–4 explained | SHIPPED (v1.6) |
| V11 | Local-vs-API data boundary documentation (honest) | SHIPPED (v1.6) |
| V12 | Safe example gallery: what to try first | SHIPPED (v1.6) |

---

## PART 2 — Release Plan

### Current State (v0.x — Complete)

| Phase | What shipped | Status |
|---|---|---|
| v0.1 | ReAct loop, shell/fs tools, SQLite episodes | ✅ |
| v0.2 | Capabilities, taint, audit log, TUI v1 | ✅ |
| v0.3 | process/service/package/disk/network/logs/browser tools | ✅ |
| v0.4 | TUI v2 (Claude Code-style single-pane) | ✅ |
| v0.5 | Skills library, GC, helm skills CLI | ✅ |
| v0.6 | Supervisor DAG, FSM, Evidence verifier | ✅ |
| v0.7 | install.sh, helm init, docs, release CI | ✅ |
| v0.8 | 100-run suite, security hardening, RC | 🔄 in progress |

---

### v1.0 — Public Release *(4 weeks from RC)*

**Scope freeze. Nothing beyond what is listed here.**

**What ships:**
- A1, A3, A4, A5, A10 — install + onboarding + init + doctor
- B1-B7, B11, B14, B27 — core CLI commands
- C1-C4, C7-C13, C18 — essential flags
- D1-D8, D11 — TUI layout + status bar
- E1-E3, E7 — chat / plan / auto-accept + Shift+Tab cycle
- F1-F18 — essential keybinds
- G1-G10, G12, G15, G25, G30 — essential slash commands
- H1-H3, H6, H8, H11 — permissions, taint, allowlists, modal, policy
- I1, I2, I5 — audit chain, verify, redaction
- J1, J9, J10 — episodes, compaction, project context files
- L1-L11, L21 — all tools shipped in v0.x
- M1-M4, M8-M10 — DAG, supervisor, roles, verifier, budgets, replan (single-agent)
- N1-N5, N7, N11, N12 — providers, routing, quirks, cost, ollama, model picker
- O1, O2, O5, O6 — logs, debug events, doctor JSON, token meter
- U15, U18, U19, U20 — small terminals, completion, XDG, telemetry consent
- V1, V4, V6 — README, demo, contributing

**Exit criteria:**
- 100-run deterministic suite green
- cargo test --workspace --all-targets, clippy, fmt all green
- README has honest "use HELM if / don't use HELM if" table
- 90-second demo recorded
- No critical security issues
- `helm --read-only "check my disk"` works on a fresh install in under 60 seconds

---

### v1.1 — Git, MCP, Sessions, Themes, Snapshots *(6 weeks from v1.0)*

First "daily use" release.

- B8, B9, B16 — replay, export, sessions list/delete/export/resume
- B15 — mcp add/list/remove/test/run
- C15 — resume / continue flag
- F19, F20 — # memory mode, ! bash mode
- G3, G4, G5, G7, G14, G17, G18, G19 — compact, new, resume, model, mcp, undo, diff, cost
- L10 — git tool
- L14 — http tool
- L15 — grep/glob via ripgrep
- L25 — mcp_call
- N9 — streaming output
- U1, U5, U7, U13 — themes, snapshots, auto-save, session naming
- B18, B19 — undo / redo via snapshots
- O7, O8, O9 — cost meter, recap, stats

**Exit:** Users can connect Gmail MCP and draft replies. Users can `helm "show me what changed today"` in a repo. Users can resume a stalled session and undo a bad edit.

---

### v1.2 — Memory, Plan Cache, Project Context *(8 weeks from v1.1)*

First self-learning release.

- J2, J3 — graph memory + embeddings
- J4 — procedural memory ⭐ NOVEL
- J5 — plan caching by goal embedding ⭐ NOVEL
- J6 — TTL + decay + helm gc
- J7, J8 — memory query, memory export/import
- J11 — # quick memory (complete)
- N6 stub — best-model-for-task
- O3 — OpenTelemetry exporter
- I4, I6, I7 — audit export, query, episode-slice

**Exit:** Running a recurring task for the 5th time uses 0 planner tokens, verified by `helm replay`. Memory graph survives `helm gc`.

---

### v1.3 — Skill Learning, Model Routing, User-Style Learning *(6 weeks from v1.2)*

Three more novel features. HELM starts adapting.

- K1 — ~30 built-in ops skills
- K2 — skill manifest format finalized
- K3 — Voyager-style auto-extraction ⭐ NOVEL
- K4 — skill versioning + gold-example regression
- K8, K9 — skill run, custom user skills
- N6 — best-model-for-task full router ⭐ NOVEL
- User-style learning module (writes ~/.helm/user_profile.toml) ⭐ NOVEL
- O10 — failure-mode telemetry

**Exit:** After 50 episodes of real use, ≥3 user-extracted skills exist with passing gold examples.

---

### v1.4 — Cancellation, Sandbox, Hooks, Custom Commands *(4 weeks from v1.3)*

Extensibility lands.

- Cancellation token threaded through every tool call
- C6 — --sandbox via bubblewrap
- P1, P2, P3 — lifecycle hooks
- P4 — custom commands
- P5 — custom skills formalized
- H4, H5, H7 — per-pattern shell allowlist, per-domain net allowlist, .helmignore
- H9 — "Why am I being asked?" link
- F22, F24 — mouse, remappable keybinds
- U2, U3, U6, U10, U16, U17 — diff style, mouse, worktree, context-low warning, sync output, CJK

**Exit:** Mid-task Ctrl+C cancels current step only. `--sandbox` provably restricts agent reach. User hooks fire on lifecycle events.

---

### v1.5 — SSH / Remote Target ⭐ NOVEL *(6 weeks from v1.4)*

Biggest differentiating release. Lead the announcement with this.

- Q1 — just-shell SSH mode
- Q2 — agent-on-remote NDJSON over SSH (already shipped, stabilize)
- Q3 — bootstrap install
- Q6 — per-target audit log
- B22, C5, G29 — helm remote add/list/test, --remote flag, /remote slash
- L23, L24 — ssh tool, scp/rsync wrappers
- Q9 — bearer auth on serve
- B25, B26 — helm serve, helm tui --attach

**Exit:** `helm --remote prod-1 "find why nginx leaks memory"` runs end-to-end against any reachable Linux host. Per-target audit chain verifies. `helm tui --attach prod-1:8765` works.

---

### v1.6 — Trust Baseline *(SHIPPED 2026-05-13)*

Make HELM safe to try without granting write access. This closes the biggest
adoption blocker from early Reddit feedback.

**What shipped:**
- B28 — `helm diagnose "<question>"` as the main safe entrypoint, cannot call write tools
- C19 — global `--dry-run` for normal agent runs
- C20 — `--evidence` flag shows full evidence report before any action
- B32 — `helm trust-report` showing provider mode, storage, sandbox, permissions, audit, secret status
- D14 — Evidence panel in TUI before permission prompts
- E8 — Diagnose mode (write tools cannot be registered)
- H14 — Diagnose mode capability lock at OS level
- G32, G33 — /diagnose and /evidence slash commands
- V10 — Trust ladder docs: Level 0 (read-only diagnose) → Level 1 (dry-run) → Level 2 (local approved) → Level 3 (remote approved) → Level 4 (governed automation)
- V11 — Honest local-vs-API data boundary docs
- V12 — Safe example gallery in README
- TUI status bar overhaul — elapsed time, better mode colors
- Tool cells with duration badges and success/failure icons
- Toast system with Success/Error/Info variants
- Welcome screen for empty chat state
- Mode-specific input placeholder text

**Evidence report must include:**
- Inspected sources (which files, logs, commands read)
- Findings (what it found)
- Assumptions (what it inferred but did not verify)
- Uncertainty (what it doesn't know)
- Proposed actions (exactly what it would do)
- Blast radius (what could be affected)
- Rollback/snapshot status (is this reversible?)
- Exact tool calls or commands it would run

**Plan mode becomes default behavior for writes.** Auto-accept must feel like
unlocking something. Plan mode should feel like the safe normal.

**Exit criteria:**
- `helm diagnose` provably cannot call write capabilities (enforced at OS level, not prompt level)
- `--dry-run` performs no file, service, package, shell, or remote mutation
- Risky actions show evidence before permission prompt
- Denied actions leave zero state changes
- `helm trust-report` accurately reflects provider mode, paths, sandbox, audit status

---

### v1.7 — Recoverable Change Execution *(4 weeks from v1.6)*

Make approved actions bounded, understandable, and reversible.

**What ships:**
- Change sets: grouping file writes, commands, service actions, package actions, remote actions into a single reviewable unit
- Mandatory pre-change snapshots or backups for supported mutations
- B31 — `helm rollback` for supported change sets
- B33 — `helm changeset {list,show,rollback}`
- I9 — per-changeset audit slice
- G35 — /changeset slash command
- Blast-radius display for local and remote actions in TUI
- Idempotency checks before repeated operations
- "How to stop/reverse this" guidance for every risky plan
- Change-set views showing: files affected, services affected, commands run, outputs, audit event IDs, rollback availability

**Exit criteria:**
- Users can inspect exactly what changed after any agent run
- Supported file/config changes can be restored to pre-run state
- Repeated runs detect existing state and do not compound changes silently
- Unsupported rollback cases are clearly labeled before execution

---

### v1.8 — Runbooks and Handoff Docs *(4 weeks from v1.7)*

Deliver value even when users never allow writes. The main objection
"I can't trust it near my server" becomes irrelevant when the most useful
thing it does requires only read access.

**What ships:**
- B29 — `helm runbook generate`
- B30 — `helm handoff`
- `helm docs update --dry-run`
- Notion-friendly Markdown output
- Architecture summaries for Docker, Compose, systemd, nginx, Proxmox, Grafana, InfluxDB
- Diff-first docs updates (never overwrites without showing diff)
- G34 — /runbook generate slash command

**Runbook output includes:**
- Service inventory
- Ports and listeners
- Config paths
- Common commands for this system
- Backup/restore notes
- Recent failures or risks observed
- Handoff summary for the next operator

**Exit criteria:**
- HELM can summarize an environment with read-only access
- Generated docs are useful without allowing any mutation
- Docs updates never overwrite without showing a diff first

---

### v1.9 — Self-Hosted Ops Pack *(4 weeks from v1.8)*

Focus on the users who showed real demand in the Reddit threads.

**What ships:**
- L26 — Docker and Compose diagnostics
- L27 — Proxmox read-only diagnostics
- L28 — Backup verification checks
- InfluxDB health checks
- Grafana/MCP workflow examples
- Update planning without automatic update execution
- Skills for real self-hosted tasks:
  - "are my backups healthy?"
  - "which containers need attention?"
  - "summarize this Proxmox host"
  - "draft a Grafana dashboard plan"
  - "what changed in my Compose stack since last week?"

**Exit criteria:**
- Users can run useful self-hosted diagnostics with read-only access
- Update plans show risk and rollback guidance before any action
- Integration docs include safe setup and no-broad-secret-access defaults

---

### v2.0 — Governed Multi-Agent *(8 weeks from v1.9)*

Use multi-agent as verification, not autonomy. The disagreement protocol
catches the "junior engineer who rushes" failure mode before it reaches
production.

**What ships:**
- M5 — Parallel sub-agents on independent DAG nodes
- M6 — Cross-step shared memory scratchpad
- M7 — Disagreement protocol: if Executor and Verifier disagree, third-agent triangulation decides ⭐ NOVEL
- M11 — Capability inheritance (sub-agents cannot inherit broader caps than parent)
- H12 — Sub-agent capability inheritance formalized
- M12, M13 — Plan visualization in TUI, TODO tool
- Q5 — Multi-target broadcast (read-only first)
- T4 — Web UI (local SSR, bearer auth, local observability surface only)
- Public blog post: "How HELM uses multi-agent as a safety net, not an automation engine"

**Constraints:**
- Parallel writes are off by default; require explicit config
- Multi-agent review must reduce risk, not increase blast radius
- Risky changes require verifier approval even in multi-agent mode
- Web UI is observability and control, not an autonomous interface

**Exit criteria:**
- Verifier-executor disagreement is correctly resolved by third agent
- Sub-agents provably cannot inherit broader capabilities than parent
- /agents shows role assignments and current state
- Web UI renders same conversation as TUI

---

### v2.5+ — Deferred Automation

Defer until usage proves trust. Do not build until HELM has been used by
≥1,000 people for ≥3 months.

- R1-R5 — Notifications + --notify flag
- R6 — Scheduled tasks via systemd-timer
- T1, T2 — whisper.cpp STT, Piper TTS
- T6 — Mobile dispatch PWA
- Q7, Q8 — helm tunnel, mobile dispatch

---

### v3.0 — Proactivity + Autonomous Mode ⭐ NOVEL

Do not ship until v2.5 has been used by ≥1,000 people for ≥3 months.
Autonomous mode is the most dangerous feature in the roadmap.

Triple-gate requirement:
1. Gold examples passing for the target skill
2. `auto:true` explicitly set on the capability in policy.toml
3. `helm autonomous enable` confirmation with full warning

- R7 — Signal watchers (file, disk, load, OOM)
- R8 — Suggestion engine
- R9 — Autonomous mode, gold-example-gated ⭐ NOVEL
- R10 — helm undo last-autonomous revert

---

### v3.1 — Decentralized Skill Exchange ⭐ NOVEL

- S2, S3 — Signed skill manifest, install with review modal
- S1 — Share session for support purposes

---

### v3.2–3.4 — Multi-Machine + Team

- S4 — CRDT-based memory sync, opt-in, encrypted
- S5 — Self-hosted relay binary
- S7 — Per-user partitions + RBAC

---

### v4.0 — Cloud Control Plane

- S6 — Hosted relay
- Web UI extended for team
- Audit retention as a service
- Apache 2.0 open core forever; CLA from contributors

---

## Novel Features — The Moat

| # | Feature | First Ships |
|---|---|---|
| 1 | Source-taint capability tokens | v0.2 ✅ |
| 2 | Diagnose mode: write-locked at tool registration level | v1.6 |
| 3 | Evidence report: inspected sources, assumptions, blast radius | v1.6 |
| 4 | Plan caching by goal embedding | v1.2 |
| 5 | Voyager-style skill learning from your own episodes | v1.3 |
| 6 | Per-model routing via observed success rates | v1.3 |
| 7 | User-style learning without fine-tuning | v1.3 |
| 8 | SSH-native multi-target operations | v1.5 |
| 9 | Governed runbook and handoff doc generation | v1.8 |
| 10 | Disagreement protocol (third-agent triangulation) | v2.0 |
| 11 | Decentralized skill exchange | v3.1 |
| 12 | Autonomous mode gated on gold-example skills | v3.0 |

---

## Name and Distribution Risk

The `helm` binary name collides with Kubernetes Helm in the DevOps audience.
This is a real adoption blocker for the sysadmin/SRE market.

**Resolution required before v2.0 marketing push:**
- Option A: Rename the project and binary entirely
- Option B: Keep the repository name but ship a non-conflicting binary alias (e.g., `helmop`, `hlm`, `hx`)
- Option C: Rename to something that positions it clearly (e.g., `opshelm`, `syshelm`)

**Decision must be made before v1.6 ships.**

---

## Explicit Rules

### DO
- Make read-only diagnostics the safest and first experience
- Show evidence, assumptions, blast radius, and rollback status before risky actions
- Gate every release on fmt, clippy, tests, and release build
- Keep docs honest about API model data boundaries
- Keep every mutation auditable and, where supported, recoverable
- Prefer narrow idempotent actions over broad repair attempts
- Finish current phase before starting next; gate on exit criteria
- Update AGENTS.md when adding files, ROADMAP.md when changing scope

### DO NOT
- Market HELM as autonomous server automation
- Make remote mutation the first demo
- Hide uncertainty or assumptions
- Imply API-backed runs are fully local
- Build autonomous scheduled repair before trust UX is proven
- Weaken redaction, capability gates, taint checks, or audit integrity
- Add v1.1+ features before v1.0 ships publicly
- Add Windows / macOS before v2.0
- Build a hosted SaaS before v4.0
- Centralize a skill marketplace

---

## Critical Path

1. ✅ v0.1–v0.7 done
2. 🔄 v0.8 100-run suite → tag v1.0.0-rc1
3. v1.0 public release
4. v1.1 git + MCP + sessions + snapshots
5. v1.2 memory + plan cache
6. v1.3 skill learning + routing + user-style
7. v1.4 cancellation + sandbox + hooks + extensibility
8. v1.5 SSH/remote (the demo that wins users)
9. v1.6 trust baseline (the release that earns skeptics)
10. v1.7 recoverable change execution
11. v1.8 runbooks and handoff docs
12. v1.9 self-hosted ops pack
13. v2.0 governed multi-agent
14. v2.5+ deferred until usage justifies

**If you add a v1.1+ feature to v1.0, you ship nothing. Resist.**
