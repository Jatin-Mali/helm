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
