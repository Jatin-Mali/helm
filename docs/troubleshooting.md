# Troubleshooting

## `helm` opens the dashboard and shows no findings

Collect a fresh snapshot:

```sh
helm monitor
helm monitor --watch --interval 60s
```

Inside the dashboard, `F5` refreshes the current monitor snapshot.

## Installer returns 404

This fork expects release assets from `Jatin-Mali/helm`.

If `curl -fsSL https://github.com/Jatin-Mali/helm/releases/latest/download/install.sh | sh`
fails, either:

- the latest release asset is not published yet for your architecture, or
- you are testing from source and should build locally instead.

Source build path:

```sh
git clone https://github.com/Jatin-Mali/helm.git
cd helm
cargo build --release -p helm-cli
./target/release/helm init
./target/release/helm doctor
./target/release/helm
```

## The dashboard says `llm api` and you expected local inference

The status bar reflects provider class, not where the host runs.

- `llm local` means local model inference, usually Ollama
- `llm api` means prompts leave the machine for provider inference

To switch to local inference:

```sh
ollama pull qwen3:4b
helm init --force --provider ollama --model qwen3:4b
helm
```

## A follow-up command is suggested but not executed

That is expected. Dashboard, monitor, diagnose, and troubleshoot flows are
read-only until you explicitly open an apply flow.

```sh
helm explain <finding-id>
helm troubleshoot --from-finding <finding-id>
helm apply-plan <plan-id>
```

## Groq rate limits

Run live tests serially:

```sh
GROQ_API_KEY=$GROQ_API_KEY cargo test --workspace -- --ignored --test-threads=1
```

## ARM64 release binary missing

If the installer says the release asset is unavailable for your architecture,
use the source-build path above. The installer now prints the exact commands.

## Ollama model missing

```sh
ollama pull qwen3:4b
helm models
helm
```

## Browser tool missing

Install and start PinchTab:

```sh
pinchtab health
pinchtab nav https://example.com --snap
```

## Permission denied

Grant only the capability needed:

```sh
helm permissions grant shell.shell --scope once
helm permissions list
```

## An apply operation was interrupted or restoration failed

Inspect the saved change set before approving another attempt:

```sh
helm change-set show <change-set-id>
helm change-set rollback <change-set-id>
```

Apply records its steps and pre-change snapshot before executing actions. An
unfinished execution is marked interrupted when the next executor opens that
database; HELM does not automatically replay it.

Automatic restoration supports regular-file edits with one link, up to 8 MiB.
It preserves bytes, owner/group, permissions and whether the file existed.
Symlink targets, special files, set-id files and extended attributes or ACLs
require manual handling. Concurrent external writes are outside this guarantee;
restoration refuses content or metadata that differs from the applied change.

Recovery material is encrypted in the database. Its `HELM_BACKUP_KEY` is stored
in the configured secrets store (`$XDG_CONFIG_HOME/helm/secrets.toml`, normally
`~/.config/helm/secrets.toml`). Keep the matching database and key together when
backing up HELM. Deleting or changing that key makes older backups unusable;
HELM refuses restoration if the key is missing or the backup fails authentication.

Arbitrary shell commands have no automatic inverse. A mixed change set may
restore its file edits but still report incomplete restoration because shell
actions require manual recovery. Legacy records without verified backup data
also require manual recovery.

## Monitoring reports unknown health or retains an old incident

Coverage is reported separately from findings. `successful` means the collector
completed its supported probes; `failed`, `unsupported`, `filtered`, `stale` and
`unknown` do not establish healthy state. Snapshots older than five minutes are
stale. Older saved snapshots without coverage metadata remain unverified.
Filtering selects detector domains; a previously observed incident may remain
visible as last-known evidence until recovery is verified.

Automatic resolution requires fresh successful comparable observations, the same
host and boot identity, and an enabled detector. After a collection gap, two
successful observations establish comparison again. Checks without complete
coverage remain unknown; this currently includes backup, cron, load, firewall
and pipeline coverage. Custom/configuration checks do not automatically resolve
without their own observation evidence.

Monitor JSON now returns a versioned object rather than a bare findings array.
Read `.findings` for the previous array and `.coverage` for domain status; the
object also includes `schema_version`, snapshot identity, time and collector
errors. Consumers of the old JSON format must select the `findings` member.

## Pairing the web console

`helm web --port 8765` listens only on loopback. Open the exact printed
`http://127.0.0.1:8765` address and enter the single-use pairing code from the
terminal. The code and browser session expire after 15 minutes; restart
`helm web` to issue a new code. Restarting also invalidates existing sessions.
The code is never included in a URL or in anonymous HTML. Cookies are
HttpOnly and SameSite=Strict; this HTTP console must stay on loopback.

API reads and WebSockets require the paired session. POST requests also need
the session's CSRF token and a matching Origin. Service stop/restart and file
rollback first return an exact review; confirming consumes that review once,
bound to this browser session, host, working directory and operation. Global
CLI grants do not authorize web mutations. Service operations use the durable
execution journal but cannot be automatically reversed. File rollback retains
the documented encrypted-backup and newer-edit checks. Read-only, dry-run and
unsupported sandbox restrictions apply to web actions too.

Webhook ingress uses separate per-service signature verification rather than
browser cookies; missing webhook secrets disable ingestion.
