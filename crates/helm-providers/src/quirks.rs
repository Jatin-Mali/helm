//! Provider/model quirks registry — per-model behaviour overrides for the ReAct loop.

/// Expected tool-call wire format for a given provider/model combination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedFormat {
    /// Provider returns structured `tool_use` content blocks.
    Native,
    /// Model emits `<tool_name>{…}</tool_name>` XML tags in text.
    XmlTag,
    /// Model emits `<function=NAME>{…}</function>` tags in text.
    FunctionTag,
    /// Model emits Pythonic `[name(k="v")]` syntax in text.
    Pythonic,
    /// Model emits bare JSON `{"name":…,"parameters":…}` in text.
    BareJson,
    /// Unknown — try all formats.
    Unknown,
}

/// Behaviour overrides for a specific provider + model combination.
#[derive(Debug, Clone)]
pub struct ProviderQuirks {
    /// Override `temperature` in every `ChatRequest` (provider may reject non-zero).
    pub force_temperature: Option<f32>,
    /// Appended verbatim to the system prompt (e.g., format instructions).
    pub system_prompt_addendum: Option<String>,
    /// Hint for the parser about what wire format to expect.
    pub expected_format: ExpectedFormat,
    /// Human-readable note surfaced in `helm doctor`.
    pub user_note: Option<String>,
}

impl Default for ProviderQuirks {
    fn default() -> Self {
        Self {
            force_temperature: None,
            system_prompt_addendum: None,
            expected_format: ExpectedFormat::Native,
            user_note: None,
        }
    }
}

/// Returns the known quirks for a given provider name and model string.
///
/// Falls back to `ProviderQuirks::default()` when no specific entry is known.
pub fn quirks_for(provider: &str, model: &str) -> ProviderQuirks {
    let model_lc = model.to_ascii_lowercase();
    let provider_lc = provider.to_ascii_lowercase();

    // Groq — hosted inference; most models work fine with native tool calls but
    // some open-weight fine-tunes only support temperature=0.
    if provider_lc.contains("groq")
        && (model_lc.contains("llama")
            || model_lc.contains("mixtral")
            || model_lc.contains("gemma"))
    {
        return ProviderQuirks {
            force_temperature: Some(0.0),
            system_prompt_addendum: None,
            expected_format: ExpectedFormat::Native,
            user_note: Some(
                "Groq open-weight models require temperature=0 for deterministic tool use."
                    .to_owned(),
            ),
        };
    }

    // Ollama — local inference; models vary widely but most need explicit format
    // instructions in the system prompt and emit XML-style tags.
    if provider_lc.contains("ollama") {
        let addendum = Some(
            "\nWhen calling a tool, respond ONLY with a JSON object: \
             {\"name\": \"<tool>\", \"parameters\": {…}}. No prose before or after."
                .to_owned(),
        );
        if model_lc.contains("mistral") || model_lc.contains("mixtral") {
            return ProviderQuirks {
                force_temperature: Some(0.0),
                system_prompt_addendum: addendum,
                expected_format: ExpectedFormat::BareJson,
                user_note: Some(
                    "Mistral/Mixtral via Ollama emits bare-JSON tool calls; temperature fixed to 0."
                        .to_owned(),
                ),
            };
        }
        return ProviderQuirks {
            force_temperature: Some(0.0),
            system_prompt_addendum: addendum,
            expected_format: ExpectedFormat::BareJson,
            user_note: Some(
                "Ollama models use bare-JSON tool calls; temperature fixed to 0.".to_owned(),
            ),
        };
    }

    // OpenRouter — pass-through for many models; open-weight models may need hints.
    if provider_lc.contains("openrouter") && model_lc.contains("hermes") {
        return ProviderQuirks {
            force_temperature: Some(0.0),
            system_prompt_addendum: Some(
                "\nUse native tool_use blocks for all tool calls.".to_owned(),
            ),
            expected_format: ExpectedFormat::Native,
            user_note: Some("Hermes models on OpenRouter prefer native tool_use.".to_owned()),
        };
    }

    // Anthropic / Gemini / NvidiaNim — full native tool call support, no overrides needed.
    ProviderQuirks::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_quirks_for_anthropic() {
        let q = quirks_for("anthropic", "claude-3-5-sonnet-20241022");
        assert_eq!(q.expected_format, ExpectedFormat::Native);
        assert!(q.force_temperature.is_none());
        assert!(q.system_prompt_addendum.is_none());
    }

    #[test]
    fn groq_llama_forces_temperature_zero() {
        let q = quirks_for("groq", "llama3-70b-8192");
        assert_eq!(q.force_temperature, Some(0.0));
        assert!(q.user_note.is_some());
    }

    #[test]
    fn ollama_returns_bare_json_format() {
        let q = quirks_for("ollama", "mistral:7b");
        assert_eq!(q.expected_format, ExpectedFormat::BareJson);
        assert!(q.system_prompt_addendum.is_some());
    }

    #[test]
    fn unknown_provider_returns_default() {
        let q = quirks_for("my_custom_provider", "some-model");
        assert_eq!(q.expected_format, ExpectedFormat::Native);
        assert!(q.force_temperature.is_none());
    }

    #[test]
    fn openrouter_hermes_gets_native_hint() {
        let q = quirks_for("openrouter", "nousresearch/hermes-3-llama");
        assert_eq!(q.expected_format, ExpectedFormat::Native);
        assert!(q.system_prompt_addendum.is_some());
    }
}
