# Threat Model

HELM is a Linux-first local machine-control agent. Its main risk is that model
output or external content can request dangerous local actions.

## Assets

- User files and secrets.
- Shell and sudo capability.
- Browser session state.
- Provider API keys.
- Audit and episode database.

## Trust Boundaries

- User prompt: trusted by default.
- Tool output: tool-tainted.
- Browser, web, email, downloads: external-tainted.
- LLM provider response: untrusted until parsed and validated.

## Controls

- Tool inputs are JSON-schema validated before execution.
- Capabilities gate dangerous actions.
- External-tainted context requires fresh approval for privileged actions.
- Tool calls are recorded in a hash-chained audit log.
- Browser content is external-tainted.
- File tools enforce allowlist and denylist path checks.
- Shell has explicit `exec` and `shell` modes.

## API Key Storage

Provider API keys are stored locally in `~/.helm/secrets.toml` (Unix mode
0600, parent directory 0700, atomic write via temp-file + rename). The file is
never world-readable; HELM refuses to load it if permissions are wider than
0600 and refuses to write below a world-writable parent. Keys are held in a
`Secret` newtype that suppresses debug output and is only exposed as a plain
string at the HTTP boundary or explicit `helm secrets get` output.

Resolution order: CLI `--api-key` flag → `~/.helm/secrets.toml` → environment variable.

Environment variables are not silently imported into the secrets store by the
TUI. They remain session-scoped unless you explicitly save them with
`helm secrets set`, `helm secrets import-env`, `helm init`, or the
authentication modal after a provider failure.

HELM v1.0 does not encrypt secrets at rest. For a Linux user home, 0600 file
mode plus a 0700 parent directory is the v1.0 security boundary.

**v1.5 note:** OS keyring integration (libsecret / GNOME Keyring / KDE KWallet)
is planned but not yet implemented.

## Non-Goals For v1

- GUI desktop control.
- IoT control.
- Perfect prevention of model reasoning mistakes.
- Cryptographic remote attestation.
