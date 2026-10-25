# General agent

`helmops run "<task>"` runs a bounded tool-using agent against a workspace
directory on this machine. The same runtime backs the assistant in `helmops tui`.

```sh
helmops run "list the TODO comments and summarize them" --workspace ./project
helmops run --sessions                   # saved sessions
helmops run --resume sess-… "continue"   # new process, same transcript
helmops run "..." --json                 # JSON event lines + final session record
```

Options: `--max-steps` (20), `--timeout-secs` (600), `--budget-usd` with
`--input-price` and `--output-price` (USD per million tokens). HELM never guesses
prices: a budget without prices is a usage error, and a hard budget stops
retries after a failed request whose charge is unknown. `--read-only` and
`--dry-run` apply to the agent's mutations.

## Tools

| Tool | Approval | Notes |
|------|----------|-------|
| `fs_read`, `search` | none | Regular files with one link inside the workspace; output bounded and redacted |
| `fs_write` | exact call, once | Encrypted backup first; journaled; one link, no symlinks |
| `shell` | exact call, once | **Broad OS authority** as your user, not confined to the workspace; the approval prompt shows the exact command |
| `http` | exact call, once | GET only; private/loopback/link-local destinations, credentials in URLs and cross-origin redirects are refused |
| `browser` | exact call, once | Static page reader through a headless Chromium (needs `chromium`/`google-chrome` installed); JavaScript and sub-resource network are disabled |

HELM state (database, secrets, config, sessions) is unreadable to every tool.
Content returned by tools, files or web pages is data: it cannot add tools,
widen scope, change grants, or approve anything.

## Approvals

Each approval covers one exact call (tool, arguments, workspace) for 15
minutes and is consumed by use. In a terminal you are prompted; without a
terminal the run stops as `waiting_for_approval`, nothing executes, and the
command exits non-zero. Resume it interactively with `--resume`. The `--yes`
flag is not honored by `run`. Ctrl-C cancels cleanly (session saved as
`cancelled`); a second Ctrl-C terminates the process.

## Sessions

One file per session in `<data dir>/agent-sessions/` (mode 0600): redacted
transcript, tool calls and results, approvals with decisions, evidence
references, usage and the terminal outcome (`completed`, `failed`, `cancelled`,
`waiting_for_approval`). Resuming removes tool calls that never produced a
result from the history, so nothing is replayed; the model must propose the
action again and you approve it again. Completion summaries cite tool results;
a task that ran no tool is never reported as having made changes.

## Recovery

File edits made through `fs_write` and shell steps recorded in the change-set
journal can be listed with `helmops change-set list` and restored with
`helmops change-set rollback <id>` where the operation is a supported file
change. Shell side effects are not reversible by HELM and are labeled manual
recovery. See `docs/troubleshooting.md`.

## Not supported

Remote execution, JavaScript-driven pages, sandbox confinement (`--sandbox` is
refused), multi-agent delegation.
