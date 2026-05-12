# Providers

HELM resolves providers in this order: CLI flags, `HELM_PROVIDER`, config file,
environment auto-detection, then Ollama fallback.

If you built HELM from source, replace `helm` below with
`./target/release/helm`.

## Groq

```sh
export GROQ_API_KEY=gsk_...
helm init --force --provider groq --model openai/gpt-oss-20b
helm doctor
```

## OpenRouter

```sh
export OPENROUTER_API_KEY=sk-or-...
helm init --force --provider openrouter
helm doctor
```

## Gemini

```sh
export GOOGLE_API_KEY=...
# GEMINI_API_KEY is also accepted
helm init --force --provider gemini --model gemini-2.5-flash
helm doctor
```

`helm` prefers `GOOGLE_API_KEY` as the default Gemini env var, but also accepts
`GEMINI_API_KEY` for compatibility.

## NVIDIA NIM

```sh
export NVIDIA_API_KEY=...
helm init --force --provider nvidia-nim
helm doctor
```

## Ollama

```sh
ollama pull qwen3:4b
helm init --force --provider ollama --model qwen3:4b
helm doctor
```

Ollama works best with tool-capable models. `llama3.2:1b` is too small for
reliable agent tasks.
