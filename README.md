# HELM

> The Rust agent for Linux operators.

HELM is a local AI agent for Linux systems work. It can inspect files, run
shell commands, manage services, look at logs, inspect disk usage, and keep a
local audit trail of what it did.

`helm` with no arguments opens the TUI.

## Install

### Release binary

Current tagged releases publish an x86_64 Linux binary plus the installer.

```sh
curl -fsSL https://github.com/Jatin-Mali/helm/releases/latest/download/install.sh | sh
helm init
helm
```

If your architecture does not have a published release asset yet, the installer
fails with source-build instructions instead of a silent 404.

### Build from source

Use this path on ARM64, on forks without release assets, or if you want to test
exact source locally.

```sh
git clone https://github.com/Jatin-Mali/helm.git
cd helm
cargo build --release -p helm-cli
./target/release/helm init
./target/release/helm doctor
./target/release/helm
```

Minimum supported Rust version: `1.85`.

## First Run

`helm init` asks for:

1. provider
2. model
3. API key if the provider needs one
4. optional crash-report consent

Stored keys go into `~/.helm/secrets.toml` with mode `0600`. `config.toml`
stores provider/model settings, not the raw key.

## Provider Examples

### OpenRouter

```sh
export OPENROUTER_API_KEY='sk-or-...'
./target/release/helm init --force --provider openrouter
./target/release/helm doctor
./target/release/helm "Reply with exactly: ok"
```

### Ollama

```sh
ollama pull qwen3:4b
./target/release/helm init --force --provider ollama --model qwen3:4b
./target/release/helm doctor
./target/release/helm
```

## Core Commands

```sh
helm                                  # open the TUI
helm "<task>"                         # run one task
helm run "<task>"                     # explicit one-shot mode
helm init                             # configure provider/model/key
helm doctor                           # verify provider, DB, tools, secrets store
helm models                           # list models for the active provider
helm episodes --limit 10              # recent runs
helm replay <episode_id>              # replay a run
helm audit verify                     # verify audit chain
helm permissions list                 # current grants
helm secrets list                     # stored provider keys
helm config path                      # config file location
helm completion bash                  # shell completion
```

## Security Notes

- Stored provider keys live in `~/.helm/secrets.toml` with mode `0600`.
- Environment variables stay ephemeral unless you explicitly import or save
  them. The TUI does not auto-import env keys into the secrets store anymore.
- Dangerous tools still require capability grants unless you run with explicit
  override flags such as `--yes`.
- HELM keeps local episode and audit state in `~/.helm/`. Treat that directory
  as sensitive local state.

## Use HELM If / Use Something Else If

| Use HELM if... | Use something else if... |
|---|---|
| You want a local Linux operations agent | You want a cloud-hosted managed agent |
| You need shell, services, logs, packages, and disk inspection in one tool | You need team collaboration and SaaS workflows |
| You want local audit and episode history | You do not want any local state at all |
| You prefer a terminal-first interface | You want a browser-only product |

## Docs

- `docs/providers.md`
- `docs/threat-model.md`
- `docs/troubleshooting.md`
- `docs/release-notes-v1.0.md`
- `CONTRIBUTING.md`

## License

Apache-2.0.
