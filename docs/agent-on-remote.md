# Remote hosts

HELM observes remote Linux hosts over SSH. It does **not** run the general agent
on a remote host, and `helm run --remote` is refused. The earlier NDJSON
agent-on-remote transport no longer exists.

## Register a host

```sh
helmops remote add prod-1 --host prod-1.example.com --user ops   # see --help for all options
helmops remote test prod-1                                       # ssh prod-1 true
helmops bootstrap ops@prod-1.example.com --register-as prod-1    # install HELM on the host
helmops agent list | test | remove                               # installed monitoring agents
```

HELM never stores SSH keys. It uses your `ssh` configuration and agent.

## Fleet monitoring

```sh
helmops monitor --fleet [--json]
```

Each host is collected over SSH with a 5 s connect timeout and a 60 s budget,
running `helmops monitor --profile quick --format json` on the host. Results
carry the host's own identity and a local observation time.

- A host that cannot be reached is **UNKNOWN**. HELM never substitutes the local
  machine's data and never reports an unreachable host as healthy.
- If a previous observation exists it is shown labeled **STALE** with its age.
  Last-known state is kept in `fleet-state.json` under the data directory, keyed
  by the registered remote name (renaming a remote forgets its history).
- Remote commands are quoted as one argument each; a hostile remote name or path
  cannot inject shell syntax.
- The remote must run a HELM version whose `monitor --format json` emits the
  versioned report (`schema_version`, `findings`, `coverage`). Reports without a
  `findings` field are treated as errors.

## Limits

- Observation only. Changes on a remote host are not supported by the web or TUI
  action paths.
- Audit events for fleet runs are written on the machine running `monitor --fleet`.
- A remote that is actually the same machine as another registered remote is not
  detected at runtime; compare host identities in the output.
