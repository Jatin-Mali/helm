//! Provider pricing lookup.
//!
//! Prices are per-1M tokens in USD. Rates are approximate and should be
//! updated periodically. Rates last verified: 2025-12.

/// Input rate per 1M tokens.
pub enum InputRate {
    GroqFree,
    GroqPaid,
    OpenRouter,
    NvidiaNim,
    OpenAi,
    Gemini,
    Anthropic,
    Ollama, // Local - $0
}

/// Output rate per 1M tokens.
pub enum OutputRate {
    GroqFree,
    GroqPaid,
    OpenRouter,
    NvidiaNim,
    OpenAi,
    Gemini,
    Anthropic,
    Ollama, // Local - $0
}

/// All known provider/model combinations with their pricing in USD per 1M tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pricing {
    pub input_rate: f64,
    pub output_rate: f64,
}

impl Pricing {
    pub const fn new(input_rate: f64, output_rate: f64) -> Self {
        Self {
            input_rate,
            output_rate,
        }
    }
}

/// Default placeholder for unknown providers.
pub const DEFAULT_PRICING: Pricing = Pricing::new(0.0, 0.0);

/// Provider pricing table (USD per 1M tokens).
/// Rates from publicly available pricing as of 2025-12.
pub fn pricing_for(provider: &str, model: &str) -> Pricing {
    let provider_lower = provider.to_lowercase();
    let model_lower = model.to_lowercase();

    // Groq: free tier = $0, paid = $0.20/$0.80 per 1M
    if provider_lower.contains("groq") {
        if model_lower.contains("llama-3.3-70b") || model_lower.contains("mixtral") {
            return Pricing::new(0.20, 0.80);
        }
        return Pricing::new(0.20, 0.80); // Default Groq paid
    }

    // OpenAI: GPT-4o-mini = $0.15/$0.60, GPT-4o = $2.50/$10.00
    if provider_lower.contains("openai") || provider_lower.contains("openrouter") {
        if model_lower.contains("gpt-4o-mini") || model_lower.contains("4o-mini") {
            return Pricing::new(0.15, 0.60);
        }
        if model_lower.contains("gpt-4o") || model_lower.contains("4o") {
            return Pricing::new(2.50, 10.00);
        }
        if model_lower.contains("o1") || model_lower.contains("o3") {
            return Pricing::new(15.00, 60.00);
        }
        // OpenRouter defaults
        return Pricing::new(0.50, 2.00);
    }

    // Nvidia NIM
    if provider_lower.contains("nvidia") {
        return Pricing::new(0.50, 2.00);
    }

    // Anthropic: Claude 4 = $15/$75 for Opus, $3/$15 for Sonnet, $0.80/$4 for Haiku
    if provider_lower.contains("anthropic") || provider_lower.contains("claude") {
        if model_lower.contains("opus") {
            return Pricing::new(15.00, 75.00);
        }
        if model_lower.contains("sonnet") {
            return Pricing::new(3.00, 15.00);
        }
        if model_lower.contains("haiku") {
            return Pricing::new(0.80, 4.00);
        }
        // Default to Sonnet pricing
        return Pricing::new(3.00, 15.00);
    }

    // Gemini: 2.0 Flash = $0.0/$0.10, 2.5 Pro = $1.25/$5.00
    if provider_lower.contains("gemini") {
        if model_lower.contains("flash") {
            return Pricing::new(0.0, 0.10);
        }
        if model_lower.contains("2.5") || model_lower.contains("pro") {
            return Pricing::new(1.25, 5.00);
        }
        return Pricing::new(0.0, 0.10);
    }

    // Ollama: local - free
    if provider_lower.contains("ollama") {
        return Pricing::new(0.0, 0.0);
    }

    // Default: unknown provider
    DEFAULT_PRICING
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groq_pricing_known_model() {
        let p = pricing_for("groq", "llama-3.3-70b-versatile");
        assert!(p.input_rate > 0.0);
    }

    #[test]
    fn anthropic_opus_pricing() {
        let p = pricing_for("anthropic", "claude-opus-4-1-20250805");
        assert_eq!(p.input_rate, 15.00);
        assert_eq!(p.output_rate, 75.00);
    }

    #[test]
    fn unknown_provider_returns_default() {
        let p = pricing_for("unknown", "nonexistent");
        assert_eq!(p, DEFAULT_PRICING);
    }

    #[test]
    fn ollama_is_free() {
        let p = pricing_for("ollama", "qwen2.5");
        assert_eq!(p.input_rate, 0.0);
        assert_eq!(p.output_rate, 0.0);
    }
}
