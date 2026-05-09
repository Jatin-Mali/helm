# Providers

HELM resolves providers in this order: CLI flags, `HELM_PROVIDER`, config file,
environment auto-detection, then Ollama fallback.

## Groq

```sh
export GROQ_API_KEY=gsk_...
./target/release/helm init --force --provider groq --model llama-3.3-70b-versatile
./target/release/helm doctor
```

## OpenRouter

```sh
export OPENROUTER_API_KEY=sk-or-...
./target/release/helm init --force --provider openrouter
./target/release/helm doctor
```

## Gemini

```sh
export GOOGLE_API_KEY=...
./target/release/helm init --force --provider gemini --model gemini-2.0-flash
./target/release/helm doctor
```

## NVIDIA NIM

```sh
export NVIDIA_API_KEY=...
./target/release/helm init --force --provider nvidia-nim
./target/release/helm doctor
```

## Ollama

```sh
ollama pull qwen3:4b
./target/release/helm init --force --provider ollama --model qwen3:4b
./target/release/helm doctor
```

Ollama works best with tool-capable models. `llama3.2:1b` is too small for
reliable agent tasks.
