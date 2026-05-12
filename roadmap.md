# HELM Feature Inventory and Implementation Strategy

## PART 1 — The complete feature inventory

Organized by surface. Every line is a feature you'll need. "Best-in-class reference" tells you who already nailed each one so you copy the right thing — never invent what's been solved. Ship-rank: MUST = required for v1.0, SHOULD = required for v2.0, COULD = post v2.0, NICE = nice-to-have anytime.

**Ship-rank legend:** MUST = required for v1.0; SHOULD = required for v2.0; COULD = post v2.0; NICE = nice-to-have anytime.

### A. Installation & first-run

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| A1 | Single curl \| sh install (~10MB static Rust binary, no runtime) |  | Crush, OpenCode | MUST |
| A2 | Native package distributions: brew, aur, nix, apt, scoop, choco, mise |  | OpenCode (best coverage) | SHOULD |
| A3 | helm with no args → starts TUI in current directory; if no config, runs onboarding |  | Crush, OpenCode | MUST |
| A4 | First-run onboarding wizard: 5 questions (provider key, default model, tools to enable, capability defaults, telemetry opt-in) |  | Cowork onboard, Crush login | MUST |
| A5 | helm init writes ~/.helm/config.toml and ./AGENTS.md if in project dir |  | OpenCode init, Claude Code /init | MUST |
| A6 | helm upgrade self-update (with rollback to previous binary on failure) |  | Claude Code | SHOULD |
| A7 | helm uninstall cleans config, db, logs (with --keep-data flag) |  | None — be the first | NICE |
| A8 | Auto-detect provider from env vars on first launch (already done) |  | Codex CLI | DONE |
| A9 | OAuth login flow for providers that support it (Anthropic, Z.AI, OpenCode Zen) |  | Crush login, OpenCode /connect | SHOULD |
| A10 | helm doctor system check with green/red status per subsystem (already done, extend) |  | Claude Code /doctor | DONE-extend |

### B. CLI surface — top-level commands

| # | Command | Purpose | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| B1 | helm | Open TUI in cwd | OpenCode/Crush convention | MUST |
| B2 | helm "<task>" | Run task non-interactive, print result, exit | OpenCode -p | MUST |
| B3 | helm run <task> [-q] [-f json] [--no-stream] | Like B2 with options | OpenCode | MUST |
| B4 | helm init [--minimal] | Init config + AGENTS.md | OpenCode | MUST |
| B5 | helm doctor [--json] | System check | Claude Code | DONE |
| B6 | helm models [--provider X] [--installed] | List models with capabilities | Claude Code /model | DONE-extend |
| B7 | helm episodes [--limit N] [--outcome X] [--since DATE] | List past episodes | None — novel | DONE-extend |
| B8 | helm replay <id> | Replay a past episode transcript | Aider /commit, OpenCode export | DONE-extend |
| B9 | helm export <id> [--format md\|json] | Export episode to file | OpenCode export | SHOULD |
| B10 | helm import <file-or-url> | Import shared episode/skill | OpenCode import | COULD |
| B11 | helm permissions {list,grant,revoke,reset} | Manage capabilities | None — novel | DONE-extend |
| B12 | helm audit {tail,verify,export} | Audit log operations | Cowork OTel (we beat them) | DONE-extend |
| B13 | helm skills {list,show,run,edit,delete,share,install,verify} | Skill management | Cowork Skills, OpenClaw skills | SHOULD |
| B14 | helm config {get,set,edit,validate,path} | Config management | OpenCode config | MUST |
| B15 | helm mcp {add,list,remove,test,run} | MCP server management | Claude Code mcp | SHOULD |
| B16 | helm sessions {list,delete,export,resume} | Session/conversation management | OpenCode sessions, Crush | SHOULD |
| B17 | helm stats [--since DATE] | Token usage, cost, success rate | OpenCode stats, Crush | SHOULD |
| B18 | helm undo [N] | Undo last N agent-applied changes | OpenCode /undo, Crush snapshots | SHOULD |
| B19 | helm redo [N] | Redo undone changes | OpenCode /redo | SHOULD |
| B20 | helm schedule {add,list,remove} | Scheduled tasks | None — novel for ops | COULD (v2.5) |
| B21 | helm watch {add,list,remove} | Signal watchers | None — novel | COULD (v3.0) |
| B22 | helm remote {add,list,test,remove} | Remote target management | None — novel | SHOULD (v1.5) |
| B23 | helm sync {push,pull,status,reset} | Multi-machine memory sync | None — novel | COULD (v3.2) |
| B24 | helm tunnel | Reverse tunnel for mobile dispatch | Tailscale-style | COULD (v2.5) |
| B25 | helm serve [--host X] [--port N] | Headless daemon mode | OpenCode serve | SHOULD |
| B26 | helm tui --attach <host:port> | Attach TUI to remote daemon | OpenCode attach | COULD |
| B27 | helm version, helm help, helm completion {bash,zsh,fish} | Standard CLI | All | MUST |

### C. CLI flags (top-level + run)

| # | Flag | Purpose | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| C1 | --provider <id> | Override provider for one run | OpenCode | DONE |
| C2 | --model <id> | Override model | OpenCode | DONE |
| C3 | --system-prompt <text> and --append-system-prompt <text> | System prompt control | Claude Code | SHOULD |
| C4 | --max-iterations <n>, --max-tokens <n>, --max-time <duration> | Budget overrides | None — best practice | DONE |
| C5 | --remote <name-or-host> | SSH/remote target | None — novel | SHOULD (v1.5) |
| C6 | --sandbox | Bubblewrap sandbox | Cowork sandbox | SHOULD (v1.4) |
| C7 | --yes, --yolo, --dangerously-skip-permissions | Auto-approve permissions | Crush --yolo, Claude Code | MUST |
| C8 | --read-only | Plan mode — no writes | OpenCode plan, Claude Code /plan | MUST |
| C9 | -p <task>, --print | Non-interactive output to stdout | OpenCode | DONE |
| C10 | -f json\|md\|text | Output format | OpenCode -f | SHOULD |
| C11 | --no-stream | Disable token streaming | All | SHOULD |
| C12 | --quiet, --verbose, --debug | Log levels | All | DONE |
| C13 | --db-path, --config, --config-dir | Override paths | OpenCode | DONE |
| C14 | --worktree <name> | Run in named git worktree | Claude Code worktree | COULD |
| C15 | --resume [<id>] or --continue | Resume last/specific session | Claude Code, Aider | SHOULD |
| C16 | --notify | Send notification on completion | None — novel | COULD (v2.5) |
| C17 | --log-file <path> | Override log location | DONE | DONE |
| C18 | --no-color, --color always\|auto\|never | Color control | All | SHOULD |

### D. TUI — layout regions

| # | Region | Detail | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| D1 | Header bar (1 line) | logo · provider/model · session name · running state · time · token meter · cost meter | Crush header, Claude Code | MUST |
| D2 | Chat scroll area | Speaker labels (you/helm), inline tool summaries, markdown rendered, code blocks with left rule | Claude Code best, Crush close 2nd | MUST |
| D3 | Tool inline summaries | One-line ✓ ran 'ls ~' (47 lines, 12ms) collapsed; expandable to full I/O | Claude Code | MUST |
| D4 | Live diff display | When agent edits file: side-by-side or unified diff inline, syntax-highlighted | Crush diff_style auto/stacked | SHOULD |
| D5 | Multiline input | Default 1 line, grows up to ~7; placeholder text in dim | Claude Code | MUST |
| D6 | Footer hint strip | Context-aware key hints (Enter send · Ctrl+T tools · …) | Claude Code | MUST |
| D7 | Right sidebar (toggleable) | Tool history list with status icons + truncated I/O | Codex/Crush ctrl+p panel | MUST |
| D8 | Modal overlay | Centered, bordered; for tool details, permission prompts, config editors | Crush dialog system | MUST |
| D9 | Plan panel | When in plan mode, show DAG progress: pending/running/done steps | None — novel (we already have DAG) | SHOULD |
| D10 | Cost ticker | Running USD cost for the session; turn red over budget | Crush cost display | SHOULD |
| D11 | Status bar (bottom-left) | mode (chat/plan/yolo) · model · vim-state | Claude Code TUI | SHOULD |
| D12 | Toast notifications | Transient bottom-right (file saved, audit verified, etc.) | None established — invent | NICE |
| D13 | Background-task indicator | Spinner for any non-blocking tasks (sync, watcher, etc.) | None | COULD |

### E. TUI — modes

| # | Mode | Behavior | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| E1 | Chat (default) | Read freely, write/exec ask permission | Claude Code default | MUST |
| E2 | Plan | Read-only; agent proposes a plan, no writes execute | OpenCode plan, Claude Code /plan | MUST |
| E3 | Auto-accept | Tools execute without prompts; capabilities still enforced | Claude Code auto-accept | MUST |
| E4 | YOLO | All permissions auto-accepted (gated behind --yolo + warning) | Crush --yolo | SHOULD |
| E5 | Replay (read-only) | View past episode in TUI as if live | None — novel | SHOULD |
| E6 | Focus | Hide chrome/sidebars/toasts, just chat + input | Claude Code focus | NICE |
| E7 | Cycle modes with Shift+Tab (visual indicator in footer) |  | All | MUST |

### F. TUI — keybinds

| # | Key | Action | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| F1 | Enter | Send | All | MUST |
| F2 | Shift+Enter / Ctrl+J / \\<Enter> | Newline | Claude Code (3 ways) | MUST |
| F3 | Ctrl+G | Open external editor ($EDITOR) for prompt | Claude Code, OpenCode editor | SHOULD |
| F4 | Ctrl+T | Toggle tool sidebar | Codex pattern | MUST |
| F5 | Ctrl+H | History panel | Claude Code /resume | MUST |
| F6 | Ctrl+L | Clear current chat (in-session only, doesn't delete from db) | Claude Code /clear | MUST |
| F7 | Ctrl+P | Command palette (fuzzy search every command) | Crush ctrl+p | MUST |
| F8 | Ctrl+R | Replay last assistant message (re-render) | None | NICE |
| F9 | Ctrl+C | Cancel current step (or quit if idle, with confirm) | Claude Code | MUST |
| F10 | Ctrl+D (empty input) | Quit | Aider | MUST |
| F11 | Ctrl+X Ctrl+K | Kill all background agents | Claude Code (recently rebound) | SHOULD |
| F12 | Esc | Close modal → close sidebar selection → close history | Claude Code | MUST |
| F13 | ↑/↓ (empty input) | Cycle previous prompts | All | MUST |
| F14 | ↑/↓ (sidebar focused) | Move tool selection | All | MUST |
| F15 | Tab (input focused) | Autocomplete: slash command, @-file, /skill, model name | Claude Code, OpenCode | MUST |
| F16 | ? | Help overlay | Crush, Claude Code | MUST |
| F17 | / (line start) | Slash command picker | Claude Code | MUST |
| F18 | @ (line start or mid-line) | File/dir picker (fuzzy) | Claude Code, OpenCode | MUST |
| F19 | # (line start) | Add note to memory ("remember: …") | Claude Code # | SHOULD |
| F20 | ! (line start) | Bash mode — run shell command in current cwd | Claude Code ! | SHOULD |
| F21 | Ctrl+B | Toggle bash mode persistently | Claude Code Alt+B | NICE |
| F22 | Mouse: scroll, click, copy-on-select | Standard | OpenCode mouse=true | SHOULD |
| F23 | Ctrl+O | Toggle verbose transcript view | Claude Code Ctrl+O | SHOULD |
| F24 | All keybinds remappable via ~/.helm/keybindings.json |  | Claude Code keybindings.json | SHOULD |
| F25 | /vim | toggle vim-mode for input | Claude Code /vim | NICE |

### G. TUI — slash commands

These are typed in the input, prefix /. Many duplicate top-level helm commands but operate on current session.

| # | Command | Purpose | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| G1 | /help | List all commands | All | MUST |
| G2 | /clear | Clear visible conversation (db row stays) | Claude Code | MUST |
| G3 | /compact [hint] | Summarize conversation to reclaim context | Claude Code, OpenCode | SHOULD |
| G4 | /new or /session new | New session, fresh context | OpenCode /new | MUST |
| G5 | /resume [id] | Resume past session | Claude Code | MUST |
| G6 | /sessions | Picker | Claude Code, OpenCode | MUST |
| G7 | /model [id] | Switch model mid-session (warn cache loss) | Claude Code | MUST |
| G8 | /effort {low,medium,high,xhigh} | Reasoning effort | Claude Code | SHOULD |
| G9 | /mode {chat,plan,auto,yolo} | Switch mode without keybind | Claude Code Shift-Tab eq | SHOULD |
| G10 | /init | Generate AGENTS.md for current dir | OpenCode init, Claude Code | MUST |
| G11 | /agents | Show role/sub-agent assignments | OpenCode | SHOULD |
| G12 | /tools | List loaded tools and their capabilities | None — novel for us | MUST |
| G13 | /skills | List skills, run /skill <name> to invoke | Cowork Skills | SHOULD |
| G14 | /mcp | Manage MCP servers | Claude Code | SHOULD |
| G15 | /permissions | Open permissions modal | None — novel | MUST |
| G16 | /audit | Tail audit log inline | None — novel | SHOULD |
| G17 | /undo [N] /redo [N] | Revert agent edits | OpenCode | SHOULD |
| G18 | /diff [path] | Show diff of last edits | OpenCode | SHOULD |
| G19 | /cost /usage /stats | Cost & usage panel | Claude Code, Crush | SHOULD |
| G20 | /share | Generate shareable link/file of conversation | OpenCode /share | NICE |
| G21 | /export [format] | Export to file | OpenCode | SHOULD |
| G22 | /config [key=val] | Edit config inline | Crush | NICE |
| G23 | /theme [name] | Switch theme | Claude Code, OpenCode | NICE |
| G24 | /keybindings | Edit keybinding file | Claude Code | NICE |
| G25 | /doctor | System check inline | Claude Code | MUST |
| G26 | /recap | Summarize what's happened so far | Claude Code | NICE |
| G27 | /btw <question> | Side-channel question without derailing main thread | Claude Code (March 2026 hit) | SHOULD |
| G28 | /think /thinking | Toggle visible chain-of-thought | None — novel for our agent | NICE |
| G29 | /remote <name> | Switch active target | None — novel (v1.5) | SHOULD |
| G30 | /quit /exit /q | Exit | OpenCode | MUST |
| G31 | /btw, custom user commands at ~/.helm/commands/<name>.md with YAML frontmatter |  | Claude Code commands, OpenCode | SHOULD |

### H. Permissions — fine-grained surface

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| H1 | Three-tier system: read=auto, exec=prompt-once, write=prompt-each |  | Claude Code | DONE |
| H2 | Capability TTL (15m / 1h / 24h / always / once) |  | Cowork-style scopes | DONE |
| H3 | Source-taint check at every escalation |  | None — your moat | DONE |
| H4 | Per-pattern shell allowlist (e.g., Bash(git status:*)) |  | Claude Code permissions.allow | SHOULD |
| H5 | Per-domain network allowlist for browser/http tools |  | Cowork allowedManagedDomainsOnly | SHOULD |
| H6 | Per-path read/write allowlist (defaults: $HOME, /tmp, /mnt/*, /media/*, /etc, /var/log, /proc) |  | Claude Code allowManagedReadPathsOnly | DONE |
| H7 | .helmignore file (like .gitignore but for tool reach) |  | Crush .crushignore | SHOULD |
| H8 | TUI permission modal: shows tool, exact input, taint, asks Allow once / Allow 15m / Allow always / Deny |  | All | MUST |
| H9 | "Why am I being asked?" link in modal explains capability + taint |  | None — novel teaching surface | NICE |
| H10 | Per-skill auto-approval flag (auto:true) gated by gold examples |  | None — novel (v3.0 autonomous) | COULD |
| H11 | Confirmation policy file (policy.toml): which actions need confirmation |  | None | DONE |
| H12 | Capability inheritance to sub-agents (least privilege; sub-agent gets a subset) |  | None — novel | SHOULD (v2.0) |
| H13 | Audit-log filter for "deny" events visible in helm doctor |  | None | SHOULD |

### I. Audit — beyond what Cowork does

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| I1 | HMAC-chained append-only log |  | None — your moat | DONE |
| I2 | helm audit verify walks chain, reports breaks with line number |  | None | DONE |
| I3 | OpenTelemetry traces option (matches Cowork's offering — but enabled by default) |  | Cowork OTel | SHOULD |
| I4 | Audit export to JSON / CSV / SIEM-compatible JSONL |  | Cowork (premium feature) | SHOULD |
| I5 | Capture rule: never log API keys, secrets, sensitive file contents (regex redaction) |  | None — your safety story | MUST |
| I6 | Audit query: helm audit grep <regex>, helm audit since <date> |  | None | SHOULD |
| I7 | Per-episode audit slice: helm audit --episode <id> |  | None | SHOULD |
| I8 | Anomaly hints: agent ran for >2× usual duration, sudden spike in capability use |  | None — novel | COULD |

### J. Memory — episodic, graph, procedural

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| J1 | Episode log per task with full step transcripts (already done) |  | All have a version of this | DONE |
| J2 | Graph memory: nodes (paths, urls, processes, services, packages), edges (modified, depends-on, etc.) |  | None do this well; closest is Manus long memory | SHOULD (v1.2) |
| J3 | Embedding index (sqlite-vec) over node descriptions and goal strings |  | Manus, Comet have analogues | SHOULD (v1.2) |
| J4 | Procedural memory: nightly clustering → injectable templates |  | None — novel | COULD (v1.2) |
| J5 | Plan cache: cosine match → reuse plan, skip planner |  | None — novel & high impact | COULD (v1.2) |
| J6 | TTL + decay + helm gc |  | None | SHOULD |
| J7 | helm memory query "<question>" to ask the memory directly |  | Manus instruction retention | COULD |
| J8 | Memory export/import for full portability |  | None — your privacy story | SHOULD |
| J9 | Context-window compaction (already done in v0.1.5) |  | Claude Code /compact | DONE |
| J10 | Per-project context files: AGENTS.md, HELM.md, CLAUDE.md (read on cd) |  | Crush AGENTS.md, Claude Code CLAUDE.md, OpenCode AGENTS.md | MUST |
| J11 | #-mode quick memory: # remember: prefer journalctl over systemctl status |  | Claude Code # | SHOULD |
| J12 | Memory namespacing per project / target / user |  | None — novel | COULD |

### K. Skills — beyond a marketplace

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| K1 | Built-in skills bundled (docker-restart, git-status, nginx-deploy starter set) | Loaded via include_str! at compile time; auto-registered as SkillTool at agent startup | Cowork built-in Skills | SHIPPED (v1.5) |
| K2 | Skill manifest format: name, description, schema, code, gold examples, capabilities, version | skill.toml format with JSON Schema input validation | Cowork Skill format | SHIPPED (v1.5) |
| K3 | Voyager-style auto-extraction from successful episodes |  | None — your moat | COULD (v1.3) |
| K4 | Skill versioning + gold-example regression tests |  | None — novel | COULD (v1.3) |
| K5 | Decentralized skill exchange (signed manifest URL) |  | None — your moat | COULD (v3.1) |
| K6 | Skill review modal: full code visible, capabilities listed, signature checked |  | None — your safety story | COULD (v3.1) |
| K7 | Skill execution sandboxed (separate Python venv per skill) |  | None | COULD |
| K8 | helm skills run <name> --input '{…}' standalone invocation | --dry-run prints resolved shell commands; live path executes via SkillTool | None | SHIPPED (v1.5) |
| K9 | Custom user-written skills in ~/.helm/skills/<name>/ (skill.toml + script) |  | OpenClaw skills, Cowork | SHOULD |
| K10 | Disable specific skills per project via .helmignore |  | None | NICE |

### L. Tools (MUST already done; expanding for v1.x+)

| # | Tool | Capability | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| L1-9 | shell, fs_read, fs_write, process, service, package, network, disk, logs, browser |  | Multiple | DONE |
| L10 | git | git.read, git.write | OpenCode/Crush git tool | SHOULD (v1.1) |
| L11 | env | env.read | None — minor | DONE |
| L12 | firewall | firewall.read/write | None | COULD |
| L13 | cron | cron.read/write | None | COULD |
| L14 | http (generic GET/POST with allowlist) | net.http | OpenCode/Crush http | SHOULD |
| L15 | grep / glob | (fast file search via ripgrep + ignore-aware) | OpenCode grep/glob, Crush | SHOULD |
| L16 | edit | (LSP-aware diff edits, language servers attached) | Crush LSP, OpenCode edit | COULD (post-v2) |
| L17 | LSP integration | for code-aware ops | OpenCode/Crush | COULD |
| L18 | docker (container ops) | docker.read/write | None — novel for ops | COULD |
| L19 | kubernetes (kubectl wrap) | k8s.read/write | None | COULD |
| L20 | systemd-machine (containerd, machinectl) |  | None | COULD |
| L21 | journalctl | with structured filters | None | DONE-extend |
| L22 | aws/gcp/azure cli wrappers |  | None | COULD |
| L23 | ssh tool | (run on remote without full daemon) | None — your moat | SHOULD (v1.5) |
| L24 | scp / rsync wrappers |  | None | COULD |
| L25 | mcp_call | (proxy to any registered MCP server tool) | Claude Code | SHOULD (v1.1) |

### M. Multi-agent + supervisor

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| M1 | Plan DAG (typed Rust struct) |  | None — your design | DONE |
| M2 | Deterministic FSM supervisor (NOT LLM) |  | None — your moat | DONE |
| M3 | Roles: Triager / Planner / Executor / Verifier / Retro |  | Cowork sub-agents, Computer 19-model orchestration | DONE |
| M4 | Per-role model assignment (ProviderRouter) |  | Computer | DONE |
| M5 | Parallel sub-agent execution on independent DAG nodes |  | Cowork sub-agents, Manus Wide Research | SHOULD (v2.0) |
| M6 | Cross-step shared memory scratchpad |  | Manus | SHOULD (v2.0) |
| M7 | Disagreement protocol (third-agent triangulation) |  | None — your moat | COULD (v2.0) |
| M8 | Verifier with structured Evidence enum |  | None — your moat | DONE |
| M9 | Budget enforcement per step + per task |  | OpenCode max | DONE |
| M10 | Replan-on-failure threshold |  | None | DONE-extend |
| M11 | Sub-agent capability inheritance (least privilege) |  | None | SHOULD (v2.0) |
| M12 | Plan visualization in TUI |  | Cowork plan view | SHOULD |
| M13 | Task tree / TODO tool — agent-managed checklist |  | OpenCode todo tool, Manus | SHOULD |

### N. Providers + model surface

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| N1 | Provider registry: anthropic, openai, gemini, groq, openrouter, nvidia-nim, ollama, generic openai-compat |  | OpenCode (75+) | DONE |
| N2 | Auto-detect from env (already done) |  | All | DONE |
| N3 | Per-role + per-model routing |  | Computer | DONE |
| N4 | Tool-call format recovery (already done) |  | None — novel | DONE |
| N5 | Model-quirks table |  | None — novel | DONE |
| N6 | "Best model for task" router |  | None — your meta-learning | COULD (v1.3) |
| N7 | Cost tracking per provider/model |  | Crush, OpenCode | SHOULD |
| N8 | Per-provider rate-limit handling with friendly errors |  | OpenCode fallback | SHOULD |
| N9 | Streaming token output (when provider supports it) |  | All | SHOULD |
| N10 | OAuth login for providers that support it |  | OpenCode /connect | COULD |
| N11 | Local-first model option (Ollama default) |  | Crush, OpenCode | DONE |
| N12 | Model picker UI (Ctrl+P → "Switch Model") |  | Crush ctrl+p | MUST |

### O. Observability

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| O1 | Structured logs via tracing to ~/.helm/helm.log |  | DONE | DONE |
| O2 | Per-iteration debug events with token counts |  | DONE | DONE |
| O3 | OpenTelemetry exporter (OTLP) | helm.episode/plan/provider_call/tool_call spans; otel feature flag; HELM_TELEMETRY_ENDPOINT env override | Cowork OTel | SHIPPED (v1.5) |
| O4 | Prometheus metrics endpoint when serving |  | None — novel for our category | COULD |
| O5 | helm doctor --json for monitoring scrape |  | DONE | DONE |
| O6 | Token-counter heartbeat in TUI |  | Crush, Claude Code | SHOULD |
| O7 | Cost meter per session |  | Crush | SHOULD |
| O8 | Session recap on resume ("last time you …") |  | Claude Code recap | SHOULD |
| O9 | helm stats daily/weekly rollups |  | OpenCode stats | SHOULD |
| O10 | Failure mode telemetry: provider errors, parse failures, validation rejects |  | None | SHOULD |

### P. Hooks & extensibility

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| P1 | Lifecycle hooks: PreToolUse, PostToolUse, PrePlan, PostPlan, OnEpisodeStart/End | All 6 hook stages wired; shell commands fire with HELM_* env vars | Claude Code hooks, Crush hooks | SHIPPED (v1.5) |
| P2 | Hook config in ~/.helm/hooks.toml (matchers + commands) |  | Claude Code | SHOULD |
| P3 | Hook env vars: HELM_TOOL, HELM_INPUT, HELM_EPISODE_ID, HELM_TARGET, HELM_CWD |  | Claude Code envs | SHOULD |
| P4 | User custom commands: ~/.helm/commands/<name>.md with frontmatter |  | Claude Code, OpenCode | SHOULD |
| P5 | User custom skills: ~/.helm/skills/<name>/skill.toml |  | OpenClaw, Cowork | SHOULD |
| P6 | User custom agents (roles): ~/.helm/agents/<name>.toml |  | OpenCode agents, Crush coordinator | COULD |
| P7 | User custom modes: ~/.helm/modes/<name>.toml |  | OpenCode modes | NICE |
| P8 | Plugin manifest with declared capabilities |  | Claude Code plugins | COULD |
| P9 | Tool plugin SDK (Rust trait + dynamic loader, or stdio JSON-RPC) |  | Crush MCP, OpenClaw | COULD |
| P10 | First-party hook example: prettier-on-edit, lint-on-write |  | Claude Code | NICE |

### Q. Networking & remote — your moat

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| Q1 | --remote <host> SSH-mode |  | None — your moat | SHOULD (v1.5) |
| Q2 | --remote agent-on-remote mode (NDJSON over SSH) | Line-delimited JSON event stream over SSH stdout; see docs/agent-on-remote.md | None | SHIPPED (v1.5) |
| Q3 | helm bootstrap <host> auto-install |  | None | COULD (v1.5) |
| Q4 | Tailscale-aware target resolution |  | None | NICE |
| Q5 | Multi-target broadcast (run same task on N hosts) |  | None — novel | COULD (v2.0) |
| Q6 | Per-target audit log (separate file per host) | Lazily-opened per-host SQLite shards under ~/.helm/audit/<host>.db with independent HMAC chains | None | SHIPPED (v1.5) |
| Q7 | helm tunnel reverse-tunnel daemon |  | None | COULD (v2.5) |
| Q8 | Mobile dispatch PWA |  | Cowork Dispatch | COULD (v2.5) |
| Q9 | Bearer token auth on serve mode |  | OpenCode OPENCODE_SERVER_PASSWORD | SHOULD |
| Q10 | mTLS option for production deployments |  | None | NICE |

### R. Notifications, scheduling, proactivity

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| R1 | Desktop notifications (libnotify) |  | OpenClaw equivalents | COULD (v2.5) |
| R2 | Telegram channel |  | OpenClaw best | COULD (v2.5) |
| R3 | Slack DM |  | OpenClaw | COULD (v2.5) |
| R4 | Email |  | OpenClaw | COULD (v2.5) |
| R5 | Per-task --notify flag |  | None | COULD |
| R6 | Scheduled tasks via systemd-timer |  | OpenClaw cron, Cowork Dispatch | COULD (v2.5) |
| R7 | Signal watchers: file/email/disk/load/oom |  | OpenClaw signal triggers | COULD (v3.0) |
| R8 | Suggestion engine |  | None — novel | COULD (v3.0) |
| R9 | Autonomous mode (gold-example skills only) |  | None — your moat | COULD (v3.0) |
| R10 | helm undo last-autonomous revert |  | None | COULD (v3.0) |

### S. Sharing & collaboration (decentralized only)

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| S1 | helm share session <id> → signed JSON file or short URL |  | OpenCode /share | NICE |
| S2 | helm skill share → signed manifest file or URL |  | None — your moat | COULD (v3.1) |
| S3 | helm skill install <url> with signature verification + review modal |  | None — your safety | COULD (v3.1) |
| S4 | Multi-machine sync (CRDT, opt-in, encrypted) |  | None — your differentiation | COULD (v3.2) |
| S5 | Self-hosted relay binary helm-relay |  | None | COULD (v3.2) |
| S6 | Anthropic-style hosted relay (paid tier) |  | None — monetization | COULD (v4.0) |
| S7 | Per-user / team RBAC |  | Cowork RBAC | COULD (v3.4) |

### T. Voice + extended interfaces (post v2)

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| T1 | whisper.cpp STT (push-to-talk Ctrl+Space) |  | None — novel for terminal agents | COULD (v2.1) |
| T2 | Piper TTS for replies |  | None | NICE |
| T3 | Wake word (post v2) |  | OpenClaw on Mac/iOS/Android | NICE (v3+) |
| T4 | Web UI (local SSR, bearer token) |  | OpenCode browser dashboard, Cowork | COULD (v2.4) |
| T5 | Read-only public share page (one-time URL) |  | OpenCode share | NICE |
| T6 | iOS / Android dispatch app |  | Cowork Dispatch, OpenClaw | COULD (v2.5) |

### U. Quality of life (the small stuff that matters)

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| U1 | Themes (dark/light/dim/solarized/dracula/custom) |  | Claude Code /theme, OpenCode | SHOULD |
| U2 | Diff style: auto/stacked/side-by-side |  | Crush diff_style | SHOULD |
| U3 | Mouse support: click, scroll, copy-on-select |  | OpenCode, Crush | SHOULD |
| U4 | Auto-update with notify mode (don't auto-install if installed via package manager) |  | OpenCode autoupdate | SHOULD |
| U5 | Snapshots (git-based) for every agent file edit; helm undo walks them |  | OpenCode snapshots | SHOULD |
| U6 | Per-project worktree mode |  | Claude Code --worktree | COULD |
| U7 | Sessions auto-save every N seconds |  | Crush | SHOULD |
| U8 | Crash-resume: if helmd dies mid-task, next start offers to resume |  | None — novel | SHOULD |
| U9 | "Last assistant text always shown" (already done in v0.1.1) |  | Critical UX | DONE |
| U10 | Smart context-low warning (transient footer toast) |  | Claude Code | SHOULD |
| U11 | Easter egg: helm hatch → terminal pet (like Claude Code /buddy) |  | Claude Code /buddy | NICE |
| U12 | Status bar shows git branch + dirty state when in repo |  | Crush | NICE |
| U13 | Session naming: auto-named by first goal, renamable via /rename |  | Claude Code | SHOULD |
| U14 | helm in a non-empty dir asks "use this as project context?" first time |  | OpenCode init | SHOULD |
| U15 | TUI degrades gracefully on small terminals (80×24 minimum) |  | All | MUST |
| U16 | Synchronized output mode for terminals that don't auto-detect |  | Claude Code env var | SHOULD |
| U17 | CJK / wide-char safe rendering |  | Claude Code | SHOULD |
| U18 | helm completion {bash,zsh,fish} for tab completion of subcommands |  | Standard | MUST |
| U19 | XDG-compliant paths: $XDG_CONFIG_HOME/helm, $XDG_DATA_HOME/helm, $XDG_STATE_HOME/helm |  | Linux best practice | SHOULD |
| U20 | First-run privacy disclosure: explicit yes/no on telemetry |  | Best practice | MUST |

### V. Documentation deliverables

| # | Feature | Details | Best-in-class reference | Ship-rank |
|---|---|---|---|---|
| V1 | README.md with the honest "use HELM if / use Cowork if" table |  | None — your differentiator | MUST |
| V2 | Cheat sheet page (every command, flag, keybind in scannable tables) |  | Claude Code cheat sheet | SHOULD |
| V3 | ADR (Architecture Decision Records) directory |  | Best practice | SHOULD |
| V4 | 90-second demo GIF/video on README |  | Cowork, OpenCode | MUST |
| V5 | docs/ site (mdBook or Astro) |  | Crush, OpenCode | SHOULD |
| V6 | CONTRIBUTING.md, code of conduct, security policy |  | Best practice | MUST |
| V7 | THREAT_MODEL.md — the prompt-injection story |  | None — your differentiator | SHOULD |
| V8 | Release notes / /release-notes command |  | Claude Code /release-notes | NICE |
| V9 | helm tour interactive in-TUI tour for first-runners |  | None — novel | COULD |

## PART 2 — Implementation strategy per cluster

I'm not going to write 200 implementation paragraphs. I'm going to give you the strategy per cluster of features — because they share infrastructure. Every cluster lists: (a) what to build first, (b) what depends on what, (c) the gotcha to watch.

### Cluster 1 — TUI shell (D, E, F, G, U)

**Strategy.** Build the layout once, harden it, then layer slash commands on top. The mistake everyone makes is writing slash commands before the layout settles, then having to rip them out.
**Order.** D1-D8 layout → F1-F12 essential keybinds → G1-G10 essential slash commands → D9-D13 advanced regions → F13-F25 advanced keybinds → G11-G31 advanced slash commands → U1-U20 polish.
**Gotcha.** Don't use ratatui::TestBackend only — it can't catch terminal-specific render bugs. Add a manual smoke pass (one screenshot from each of: gnome-terminal, alacritty, kitty, tmux, screen, ssh).

### Cluster 2 — Permissions, audit, taint (H, I)

Already mostly built. The expansions to plan now:

- H4-H7 (allowlists + .helmignore): one new module, ~300 lines, two days.
- H8-H9 (TUI permission modal with "why am I being asked"): two days, depends on Cluster 1's modal system.
- H12 (capability inheritance to sub-agents): coupled to Cluster 6 (multi-agent); build them together in v2.0.
- I3-I8 (OTel + redaction + audit query): one module each. The redaction (I5) is the most important — implement it before any beta release.

**Gotcha.** Redaction must be on by default and impossible to fully disable. Even with --debug, secrets are redacted in log output (only present in encrypted on-disk audit if user opts in).

### Cluster 3 — Memory & learning (J, K)

**Order.** J10 (project context files) first — it's two days of work and unblocks everyone using HELM in a project. Then J2-J3 (graph + embeddings), then J4-J5 (procedural + plan cache, your novel features), then K1 (built-in skills), then K3 (skill auto-extraction).
**Gotcha.** Embedding model choice is a hidden trap. Don't bind to a hosted embedding API — that violates your privacy story. Use a small local embedding model (bge-small or similar via Ollama) by default; allow OpenAI-style for users who don't care.

### Cluster 4 — Tools (L)

Already done for v1.0 set. The v1.1+ set (git, http, grep/glob, edit, mcp_call) — build them as one batch in two weeks. Each is ~150 lines and follows the same Tool trait pattern.
**Gotcha.** grep/glob should wrap ripgrep (with ignore crate semantics so .gitignore/.helmignore are respected), not reimplement search. Don't reinvent.

### Cluster 5 — Providers (N)

Already in good shape. The remaining work:

- N6 (best-model router): coupled to meta-learning; build with v1.3.
- N9 (streaming): backbone work — implement once, propagate to TUI rendering. Two weeks.
- N10 (OAuth): per-provider; not blocking; ship as users request.

**Gotcha.** Streaming + tool-call recovery + correction retry interact in nasty ways. When you stream and the model emits a malformed tool call mid-stream, you can't undo what's already shown. Buffer through the stream, validate on completion, redact and re-emit if recovery is needed.

### Cluster 6 — Multi-agent (M)

Already partially done (DAG + supervisor + roles). The v2.0 work:

- M5 (parallel sub-agents): hardest piece. Requires careful tokio task supervision + shared scratchpad design + capability inheritance.
- M7 (disagreement protocol): build last; it's an additive layer over (M5).
- M11 (capability inheritance): coupled with H12.
- M13 (TODO tool): borrow OpenCode's pattern — global state per session, two tools (todo_read, todo_write), TUI displays as checklist.

**Gotcha.** Parallel sub-agents share the audit log. Use a per-sub-agent prefix in audit entries so the chain stays verifiable but provenance is clear.

### Cluster 7 — Hooks & extensibility (P)

**Order.** P4 (custom commands) first — it's the easiest extension surface and users want it. Then P1-P3 (lifecycle hooks). Then P5 (custom skills, depends on Cluster 3 skill schema). Then P6-P9 (custom agents/modes/plugins) — these are post v2.0.
**Gotcha.** Hooks are shell commands. Treat them as code execution — they need their own capability gate (hook.run). Default-deny; user explicitly enables.

### Cluster 8 — Remote & networking (Q)

**Order.** Q1 (just-shell SSH) first — biggest impact, simplest implementation. Q2 (agent-on-remote) shipped in v1.5 as NDJSON-over-SSH (line-delimited JSON over SSH stdout/stdin); gRPC remains a v2.0 follow-up. Q3 (bootstrap) last — depends on Q2.
**Gotcha.** SSH key management. Do NOT roll your own. Read ~/.ssh/config, defer to the user's ssh-agent, never store keys in HELM's data dir.

### Cluster 9 — Notifications/scheduling/proactivity (R)

Defer all of this past v2.0. When you build it, do it in this order: R1 (libnotify) → R6 (systemd-timer scheduling) → R5 (--notify flag) → R7 (file/disk watchers) → R8 (suggestions) → R9 (autonomous).
**Gotcha.** Autonomous mode is the highest-risk feature in the entire roadmap. Triple-gate it: gold examples + auto:true on capability + opt-in flag in config + explicit helm autonomous enable confirmation. Anything less and you ship a Cowork-class CVE.

### Cluster 10 — Sharing (S)

Defer to v3+. When you build it, S2-S3 (signed skill manifest + review) is the only one that matters. S4-S7 are post-MVP for monetization.

### Cluster 11 — Voice / Web / Mobile (T)

Defer everything past v2.0. T4 (web UI) is the natural first one when you build them — it doubles as the dispatch backend.

### Cluster 12 — Quality-of-life (U) and Docs (V)

Don't batch this. Sprinkle U items across every release — each release picks 3-5 from U to ship. V1 (README) and V4 (demo) are MUST for v1.0; the rest of V can wait.

# HELM — Product Roadmap (canonical, rev 2026-05-08)

## Final Goal
A self-hosted, self-learning Linux operations agent that runs headless on
any machine, controls it completely under explicit user permission,
improves from its own episodes, and can be deployed across multiple
machines — open source, privacy-first, no cloud required.

## Audience
Developers and sysadmins who live in tmux, run at least one VPS or server,
distrust cloud agents touching their files, and want one auditable binary.
Not a replacement for Cowork. Not a clone of OpenClaw. The Linux operator's
agent.

## Versioning convention
- v0.x  Pre-alpha foundation work (mostly complete)
- v1.0  Public release. Open source on GitHub. README is honest.
- v1.x  Polish + must-have features for the audience
- v2.0  Differentiation release. The novel features ship.
- v2.5  Interface expansion (notification, scheduler, voice, web)
- v3.x  Proactivity + multi-machine
- v4.0  Hosted control plane + monetization

---

## Current state (2026-05-08)
| Phase | Status |
|-------|--------|
| v0.1 ReAct loop, shell/fs tools, SQLite episodes | ✅ |
| v0.2 Capabilities, taint, audit log, TUI v1 | ✅ |
| v0.3 process/service/package/disk/network/logs/browser tools | ✅ |
| v0.4 TUI v2 (Claude Code-style single-pane) | ✅ |
| v0.5 Skills library, GC, helm skills CLI | ✅ |
| v0.6 Supervisor DAG, FSM, Evidence verifier | ✅ |
| v0.7 install.sh, helm init, docs, release CI | ✅ |
| v0.8 100-run suite, security hardening, RC | 🔄 |

---

## Explicit rules

### DO
- Finish current phase before starting next; gate on exit criteria.
- Write tests before marking done; clippy + fmt + test green at every merge.
- Update AGENTS.md when adding files, ROADMAP.md when changing scope.
- Use audit chain for every write — `helm audit verify` clean before tagging.
- Public PROJECT_PROMISE.md updated on every release with date.

### DO NOT
- Do not add v1.1+ features before v1.0 ships publicly.
- Do not implement fine-tuning or self-modifying agent code (research, post-v4).
- Do not add Windows / macOS before v2.0.
- Do not build a hosted SaaS before v4.0.
- Do not centralize a skill marketplace — decentralized exchange only (v3.1).
- Do not add a feature labeled MUST-NOT (the GUI computer-use, vision agent, …).

---

## v1.0 — Public release  *(target: 4 weeks from RC)*

**Scope freeze.** Beyond what is here, nothing.

**MUST features (gating):**
- A1, A3, A4, A5, A10 (install + onboarding + init + doctor)
- B1-B7, B11, B14, B27 (top-level commands: tui, run, init, doctor, models, episodes, permissions, config, version/help/completion)
- C1-C4, C7-C13, C18 (essential flags)
- D1-D8, D11 (TUI layout + status bar)
- E1-E3, E7 (chat / plan / auto-accept modes + Shift+Tab cycle)
- F1-F18 (essential keybinds, including @ and / pickers)
- G1-G10, G12, G15, G25, G30 (essential slash commands)
- H1-H3, H6, H8, H11 (permissions, taint, allowlists, modal, policy)
- I1, I2, I5 (audit chain, verify, redaction)
- J1, J9, J10 (episodes, compaction, project context files)
- L1-L11, L21 (tools shipped in v0.x)
- M1-M4, M8-M10 (DAG, supervisor, roles, verifier, budgets, replan — single-agent only)
- N1-N5, N7, N11, N12 (providers, routing, recovery, quirks, cost, ollama, model picker)
- O1, O2, O5, O6 (logs, debug events, doctor JSON, token meter)
- U15, U18, U19, U20 (small terminals, completion, XDG, telemetry consent)
- V1, V4, V6 (README, demo, contributing)

**Exit criteria.**
- 100-run deterministic suite green.
- `cargo test --workspace --all-targets`, clippy, fmt all green.
- README has the "use HELM if / use Cowork if" honest comparison table.
- 90-second demo recorded.
- 100 GitHub stars, 5 issues from strangers, 0 critical security issues
  in first 72 hours.

---

## v1.1 — Git, MCP, sessions, themes, snapshots  *(target: 6 weeks from v1.0)*

Designed as the first "you actually want to use this daily" release.

- B8, B9, B16 (replay, export, sessions list/delete/export/resume)
- B15 (mcp add/list/remove/test/run)
- C5 stub, C15 (resume / continue flag)
- F19, F20 (# memory mode, ! bash mode)
- G3, G4, G5, G7, G14, G18, G19 (compact, new, resume, model, mcp, diff, cost/usage/stats)
- L10 (git tool), L14 (http tool), L15 (grep/glob via ripgrep), L25 (mcp_call)
- N9 (streaming output)
- U1, U5, U7, U13 (themes, snapshots, auto-save, session naming)
- B18, B19, G17 (undo / redo via snapshots)
- O7, O8, O9 (cost meter, recap, stats rollup)

**Exit:** users can connect a Gmail MCP server and the agent drafts replies
from natural language; users can `helm "show me what changed today"` in a
repo without raw shell; users can resume a stalled session and undo a bad
edit.

---

## v1.2 — Memory, plan cache, project context  *(target: 8 weeks from v1.1)*

The first "self-learning" release. Three of the eight novel features ship.

- J2, J3 (graph memory + embeddings)
- J4 (procedural memory)  ⭐ NOVEL
- J5 (plan caching by goal embedding)  ⭐ NOVEL
- J6 (TTL + decay + helm gc)
- J7, J8 (memory query, memory export/import)
- J11 (# quick memory) — already partially in 1.1; complete here
- N6 stub (best-model-for-task — full version in v1.3)
- O3 (OpenTelemetry exporter)
- I4, I6, I7 (audit export, query, episode-slice)

**Exit:** running a recurring task (e.g., "find my biggest log files") for
the 5th time uses 0 planner tokens — verified by `helm replay`. Memory
graph survives a `helm gc` cleanly.

---

## v1.3 — Skill learning, model routing, user-style learning  *(target: 6 weeks from v1.2)*

Three more novel features. This is when HELM stops being "another agent" and
becomes "the agent that adapts."

- K1 (~30 built-in ops skills bundled)
- K2 (skill manifest format finalized)
- K3 (Voyager-style auto-extraction)  ⭐ NOVEL
- K4 (skill versioning + gold-example regression)
- K8, K9 (skill run, custom user skills)
- N6 (best-model-for-task router, full version)  ⭐ NOVEL
- User-style learning module (writes ~/.helm/user_profile.toml)  ⭐ NOVEL
- O10 (failure-mode telemetry)

**Exit:** after 50 episodes of real use, ≥3 user-extracted skills exist
with passing gold examples; `helm profile show` displays learned style
preferences; routing dashboard shows per-model success rates.

---

## v1.4 — Cancellation, sandbox, hooks, custom commands  *(target: 4 weeks from v1.3)*

The "extensibility lands" release.

- Cancellation token threaded through every tool call
- C6 (--sandbox via bubblewrap, opt-in)
- P1, P2, P3 (lifecycle hooks)
- P4 (custom commands at ~/.helm/commands/<name>.md)
- P5 (custom skills, formalized)
- H4, H5, H7 (per-pattern shell allowlist, per-domain net allowlist, .helmignore)
- H9 (TUI "why am I being asked" link)
- F22, F24 (mouse, remappable keybinds)
- G22, G23, G24 (config / theme / keybindings inline)
- U2, U3, U6, U10, U16, U17 (diff style, mouse, worktree, context-low warning, sync output, CJK)

**Exit:** mid-task Ctrl+C cancels current step only; `--sandbox` provably
restricts agent reach; user-written hooks fire on lifecycle events; user
custom commands work from / picker.

---

## v1.5 — SSH/remote target  ⭐ NOVEL  *(target: 6 weeks from v1.4)*

The single biggest differentiating release. **Lead the v1.5 announcement
with this — it's the demo nobody else can match.**

- Q1 (just-shell SSH mode)
- Q2 (agent-on-remote: NDJSON over SSH — shipped; gRPC deferred to v2.0)
- Q3 (bootstrap install)
- Q6 (per-target audit log)
- B22, C5, G29 (helm remote add/list/test, --remote flag, /remote slash)
- L23, L24 (ssh tool, scp/rsync wrappers)
- Q9 (bearer auth on serve)
- B25, B26 (helm serve, helm tui --attach)

**Exit:** `helm --remote prod-1 "find why nginx leaks memory and patch it"`
runs end-to-end against any reachable Linux host. Per-target audit chain
verifies. `helm tui --attach prod-1:8765` works.

---

## v2.0 — Multi-agent + disagreement + public release #2  *(target: 8 weeks from v1.5)*

Last two of the eight novel features. After this, you have the moat.

- M5 (parallel sub-agents on independent DAG nodes)
- M6 (cross-step shared scratchpad)
- M7 (disagreement protocol)  ⭐ NOVEL
- M11 (capability inheritance to sub-agents)
- M12, M13 (plan visualization in TUI, TODO tool)
- H12 (sub-agent capability inheritance, formalized)
- Q5 (multi-target broadcast: same task to N hosts)
- T4 (web UI — local SSR, bearer auth)
- Public blog post: "What HELM does that nobody else does"

**Exit:** parallel sub-agents run against 3 hosts simultaneously; mock
verifier-executor disagreement is correctly resolved by third agent;
`/agents` shows role assignments; web UI at localhost:8765 renders the
same conversation as the TUI.

---

## v2.5 — Notifications, scheduling, voice  *(deferred until v2.0 has 1k stars)*

- R1-R5 (notifications + --notify flag)
- R6 (scheduled tasks via systemd-timer)
- T1, T2 (whisper.cpp STT push-to-talk, Piper TTS)
- T6 (mobile dispatch PWA)
- Q7 (helm tunnel)
- Q8 (mobile dispatch wired)

---

## v3.0 — Proactivity + autonomous mode  ⭐ NOVEL

Don't ship until v2.5 has been used by ≥1k people for ≥3 months. Autonomous
mode is the most dangerous feature in the entire product.

- R7 (signal watchers)
- R8 (suggestion engine)
- R9 (autonomous mode, gold-example-gated)  ⭐ NOVEL
- R10 (helm undo last-autonomous)

---

## v3.1 — Decentralized skill exchange  ⭐ NOVEL

- S2, S3 (signed skill manifest, install with review)
- S1 (share session for support purposes)

---

## v3.2-3.4 — Multi-machine + team

- S4 (CRDT-based memory sync, opt-in, encrypted)
- S5 (self-hosted relay binary)
- S7 (per-user partitions + RBAC)

---

## v4.0 — Cloud control plane (monetization)

- S6 (hosted relay)
- Web UI extended for team
- Audit retention as a service
- Apache 2.0 open core forever; CLA from contributors

---

## Novel features (the moat — May 2026)

| # | Feature | First ships |
|---|---------|-------------|
| 1 | Source-taint capability tokens | v0.2 ✅ |
| 2 | SSH-native multi-target | v1.5 |
| 3 | Plan caching by goal embedding | v1.2 |
| 4 | Voyager-style skill learning from your own episodes | v1.3 |
| 5 | Per-model routing via observed success rates | v1.3 |
| 6 | User-style learning without fine-tuning | v1.3 |
| 7 | Disagreement protocol (third-agent triangulation) | v2.0 |
| 8 | Decentralized skill exchange | v3.1 |
| 9 | Autonomous mode gated on gold-example skills | v3.0 |

---

## Hard non-goals (do not build, do not let users beg you into building)

- GUI vision-based computer-use
- Windows / macOS before v2.0
- Centralized skill marketplace
- Fine-tuning or weight modification of any model
- Self-modifying agent source code
- Hosted SaaS before v4.0
- Anything that violates the 5-layer security guarantee
  (capabilities → taint → confirmation → audit → verifier)

---

## Critical path (the only path)

1. ✅ v0.1–v0.7 done.
2. 🔄 v0.8 100-run suite → tag v1.0.0-rc1.
3. v1.0 public release with feature set above.
4. v1.1 git + MCP + sessions + snapshots.
5. v1.2 memory + plan cache (3 novel features).
6. v1.3 skill learning + routing + user-style (3 more novel features).
7. v1.4 cancellation + sandbox + hooks + extensibility.
8. v1.5 SSH/remote (the demo that wins users).
9. v2.0 multi-agent + disagreement (last 2 novel features) + public release #2.
10. v2.5+ deferred until usage justifies.

If you add a v1.1+ feature to v1.0, you ship nothing. Resist.
