# Troubleshooting

## The model wrote `$(date)` literally

Use a tool-capable model and prefer shell redirection:

```sh
HELM_MODEL=llama-3.3-70b-versatile helm "create /tmp/hello.txt with the output of date and uname -a"
```

## Groq rate limits

Run live tests serially:

```sh
GROQ_API_KEY=$GROQ_API_KEY cargo test --workspace -- --ignored --test-threads=1
```

## Ollama model missing

```sh
ollama pull qwen3:4b
./target/release/helm models
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
./target/release/helm permissions grant shell.shell --scope once
./target/release/helm permissions list
```
