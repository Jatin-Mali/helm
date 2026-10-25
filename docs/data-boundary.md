# Data Boundary

HELM enforces a hard boundary between local state, external inputs, and
approved mutations. Monitoring is useful only if the product is honest about
what it saw, where that data came from, and when commands leave the machine.

## Default Boundary

When you run `helm`, you land in the dashboard. That default path is read-only.

Read-only surfaces:

- `helm`
- `helm snapshot`
- `helm monitor`
- `helm diagnose`
- dashboard refresh, evidence views, and read-only follow-up checks

These flows may inspect system state and call local or remote providers for
reasoning, but they do not mutate the host.

## Provider Boundary

HELM supports two provider classes:

- **local model**: inference stays on the machine, for example Ollama
- **API provider**: prompts leave the machine, for example Anthropic, Gemini,
  Groq, OpenRouter, or Nvidia NIM

The dashboard status bar and `helm trust-report` must always make that boundary
explicit. Local monitoring data does not imply local inference.

## Taint Boundary

Every piece of data in HELM carries a `TaintLevel`.

| Level | Source | Write Allowed? |
|-------|--------|----------------|
| `Local` | user input, local files, local collectors | Yes, after approval |
| `External` | browser output, SSH, MCP, HTTP, remote tools | No direct write escalation |

Taint is propagated through `Tainted<T>`. External content cannot silently
become an approved local mutation.

## Mutation Boundary

Mutation starts only when the user leaves monitor/troubleshoot flows and opens
an apply flow.

Required before mutation:

1. a stored finding or explicit user problem
2. a troubleshooting plan
3. exact command preview
4. expected effect on this host
5. blast radius and rollback note
6. explicit approval

No dashboard surface may hide that transition.

## Protected Local State

HELM local state is sensitive and must stay redacted in persistence and trace
output:

- `$XDG_CONFIG_HOME/helm/secrets.toml`
- `$XDG_CONFIG_HOME/helm/.secrets.toml.lock`
- `$XDG_DATA_HOME/helm/helm.db`
- `$XDG_DATA_HOME/helm/logs/helm.log`

`fs_read` denies these paths by default.

## Audit Boundary

Important state transitions must be auditable:

- snapshot stored
- findings generated
- plan rendered
- approval granted or denied
- command executed
- verification result

Use:

```bash
helm trust-report
helm audit verify
```

to inspect boundary-related trust signals.

### What the audit chain does and does not prove

New events are chained with SHA-256 over a canonical, length-prefixed encoding (`v2:` hashes).
Rows written by earlier versions use a 64-bit FNV hash; `helm audit verify` counts them and
labels them legacy. They still detect accidental edits, but anyone can recompute them.

The chain is **unkeyed**. Whoever can write `helm.db` (the same OS user, or root) can rewrite
history and recompute every hash; a key stored beside the database would not change that. To
detect rewriting or truncation, copy the `head[...]` hashes printed by `helm audit verify` to a
place the host cannot modify (a remote log, a printed note) and compare later. Stronger
guarantees need externally anchored audit, which HELM does not provide.

### Redaction limits

`redact_secrets` removes recognisable provider keys, bearer/basic credentials, URL passwords,
`password=`/`token:`-style assignments (quoted or bare), private-key blocks and HELM's own
protected paths at the persistence, trace, export and provider-prompt boundaries. A credential
with no recognisable marker cannot be found by pattern; keep such values out of evidence.
Provider keys belong only in `secrets.toml`; `helm config set` and the TUI refuse
credential-looking keys and values.
