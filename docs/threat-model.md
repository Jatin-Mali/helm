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

## Non-Goals For v1

- GUI desktop control.
- IoT control.
- Perfect prevention of model reasoning mistakes.
- Cryptographic remote attestation.
