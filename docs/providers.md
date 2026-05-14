# Providers

HELM resolves providers in this order: CLI flags, `HELM_PROVIDER`, config file,
environment auto-detection, then Ollama fallback.

If you built HELM from source, replace `helm` below with
`./target/release/helm`.

Default usage after setup:

```sh
helm init
helm            # opens the dashboard
helm doctor
helm trust-report
```

## Groq

```sh
export GROQ_API_KEY=gsk_...
helm init --force --provider groq --model openai/gpt-oss-20b
helm doctor
helm
```

## OpenRouter

```sh
export OPENROUTER_API_KEY=sk-or-...
helm init --force --provider openrouter
helm doctor
helm
```

## Gemini

```sh
export GOOGLE_API_KEY=...
# GEMINI_API_KEY is also accepted
helm init --force --provider gemini --model gemini-2.5-flash
helm doctor
helm
```

`helm` prefers `GOOGLE_API_KEY` as the default Gemini env var, but also accepts
`GEMINI_API_KEY` for compatibility.

## NVIDIA NIM

```sh
export NVIDIA_API_KEY=...
helm init --force --provider nvidia-nim
helm doctor
helm
```

## Ollama

```sh
ollama pull qwen3:4b
helm init --force --provider ollama --model qwen3:4b
helm doctor
helm
```

Ollama works best with tool-capable models. `llama3.2:1b` is too small for
reliable agent tasks.

## Local vs API boundary

- Ollama is shown in the dashboard as `llm local`
- API providers are shown as `llm api`

That boundary is also reported by `helm trust-report`.
