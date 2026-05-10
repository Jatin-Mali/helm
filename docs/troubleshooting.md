# Troubleshooting

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
```

## The model wrote `$(date)` literally

Use a tool-capable model and prefer shell redirection:

```sh
HELM_MODEL=openai/gpt-oss-20b helm "create /tmp/hello.txt with the output of date and uname -a"
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
