# HELM v1.0 Release Notes

## Persistent Provider Keys

HELM now stores provider API keys in `~/.helm/secrets.toml` with Unix mode
0600. This fixes the old env-only setup where a fresh terminal lost access to
the configured provider.

Migration for existing users:

```sh
helm secrets import-env
```

That imports any provider keys present in the current shell, including
`GROQ_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`, `GEMINI_API_KEY`,
`OPENROUTER_API_KEY`, `NVIDIA_API_KEY`, and `OPENAI_API_KEY`.

Use `helm secrets list` to show stored key names. Values are never printed
unless explicitly requested with `helm secrets get <NAME>`.
