# HELMOPS

> A Linux operations console and a general AI agent, with local evidence, exact approvals and recoverable changes.

HELMOPS observes Linux hosts (services, disks, containers, logs, certificates,
backups, databases you register), explains findings with evidence, and turns
them into reviewed plans. It changes nothing unless you approve the exact
operation. The same binary also runs a general agent (`helmops run`) that can
read, search, edit files and run commands inside a workspace, one approved
action at a time.

`helmops` and `helm` are the same program. With no arguments it opens the web
dashboard.

## Install

### Release binary (Linux x86-64 only)

```sh
curl -fsSL https://github.com/Jatin-Mali/helm/releases/latest/download/install.sh | sh
helmops init
```

The installer downloads one asset, verifies its SHA-256 checksum, and replaces
the existing binary only after verification. On any other OS or architecture,
or if verification fails, it stops and prints the source-build steps. It never
modifies an existing install on failure.

### Build from source

```sh
git clone https://github.com/Jatin-Mali/helm.git
cd helm
cargo build --release -p helm-cli
./target/release/helm init
./target/release/helm doctor
```

Requires Rust 1.89 or newer.

## First run

`helmops init` asks for a provider, model and (if needed) an API key. Keys are
stored in `$XDG_CONFIG_HOME/helm/secrets.toml` (mode `0600`); `config.toml`
holds provider and model only. Environment variables work without being saved.

| Type | Path |
|------|------|
| Config | `$XDG_CONFIG_HOME/helm/` or `~/.config/helm/` |
| Data | `$XDG_DATA_HOME/helm/` or `~/.local/share/helm/` |
| Cache | `$XDG_CACHE_HOME/helm/` or `~/.cache/helm/` |

Providers: Ollama (local), Groq, OpenRouter, Gemini, Anthropic, NVIDIA NIM and
any OpenAI-compatible endpoint. See `docs/providers.md`. API providers mean
prompts (redacted) leave your machine. Ollama inference is local only with a
local endpoint and a locally running model; Ollama cloud models use remote inference.

## Operations

```sh
helmops snapshot --profile standard           # typed system snapshot
helmops monitor --domain disks,services       # findings with coverage (what was and was not observed)
helmops monitor --fleet                       # every registered SSH host; unreachable hosts are UNKNOWN, never healthy
helmops explain <finding-id>
helmops troubleshoot "investigate this finding" --from-finding <id>  # reviewed plan
helmops apply-plan <plan-id>                  # approve step by step; changes are journaled
helmops change-set list
helmops audit verify                          # hash chain check, prints head hashes
```

Monitoring reports state what each domain's collector observed. A failed,
filtered, stale or unsupported collector is reported as unknown and never
resolves an incident. `monitor --format json` emits a versioned object; read
findings from `.findings`.

Changes: every approved step is recorded before it runs. File edits keep
encrypted backups and can be restored with `helmops change-set rollback <id>` where
the operation is supported; other operations are labeled "manual recovery".
Details and limits: `docs/troubleshooting.md`.

## Web dashboard

```sh
helmops web                # http://127.0.0.1:9120
```

The dashboard binds to loopback only. On start, the terminal prints a
single-use pairing code (valid 15 minutes); entering it creates an HttpOnly,
SameSite session. Without a session every data route and WebSocket is refused;
the Host and Origin must match, and every action needs the session's CSRF
token. Service stop/restart and rollback are two-step: the dashboard shows the
exact operation, and approving executes only that operation once.

Do not expose it through a proxy or tunnel: there is no multi-user model or
TLS. Restart `helmops web` to pair again.

Webhooks (`/api/pipelines/webhook/{provider}/{service_id}`) use a per-service secret from the
secrets store (`HELM_WEBHOOK_SECRET_<SERVICE_ID>`); GitHub (HMAC-SHA256),
GitLab (token) and generic timestamped HMAC deliveries are verified over the
raw body, and replays are rejected.

Terminal dashboard: `helmops tui`. Background collection: `helmops daemon start`
(status reports a stale heartbeat as stale).

## General agent

```sh
helmops run "find the failing test in this project and fix it" --workspace ./myproject
helmops run --sessions
helmops run --resume <session-id> "continue"
```

Tools: read, search, write (approval), shell (approval), HTTP GET (approval), a
static browser reader (approval). Every write, command and request needs your
approval of that exact call; without a terminal the run stops and executes
nothing. File and shell content is untrusted data and cannot grant itself
authority. Sessions are saved redacted and are resumable; resuming never
replays an unfinished action. Details: `docs/agent.md`.

Agent tasks act on the local machine only.

## Remote hosts

`helmops remote add`, `helmops bootstrap user@host --register-as prod-1`,
`helmops agent install|register|list|test|remove`, `helmops monitor --fleet`.
Remote support is SSH-based observation. See `docs/agent-on-remote.md`.

## Security notes

- Provider keys: `secrets.toml` (0600). The TUI never copies env keys into it.
- Anything persisted or sent to a provider passes through `redact_secrets`; it
  is pattern-based and cannot recognize arbitrary secrets (`docs/data-boundary.md`).
- The audit log is a SHA-256 hash chain, not a MAC: a same-user attacker can
  rewrite it. Record `audit verify` head hashes off-host if you need that
  guarantee.
- `--read-only` and `--dry-run` apply to every execution path. `--sandbox` is
  not available; HELM refuses host execution rather than pretend.
- Unsupported: non-Linux hosts, `helmops run --remote`, JavaScript-driven web
  pages in the agent browser, Jenkins webhooks.

## Docs

[Usage and project presentation guide](docs/PROJECT_GUIDE.md) ·
[Independent verification](docs/VERIFICATION.md) · [Demo tests](docs/demo.md) ·
`docs/providers.md` · `docs/agent.md` · `docs/agent-on-remote.md` ·
`docs/threat-model.md` · `docs/trust-ladder.md` · `docs/data-boundary.md` ·
`docs/troubleshooting.md` · `docs/detector-review-checklist.md` ·
`docs/release-notes-v1.0.md` · `CONTRIBUTING.md`

## License

Apache-2.0.
