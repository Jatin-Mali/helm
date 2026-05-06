//! Language model provider implementations for HELM.

pub mod anthropic;
pub mod gemini;
pub mod mock;
pub mod ollama;
pub mod openai_compat;
pub mod provider;
pub mod quirks;

pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use mock::MockProvider;
pub use ollama::OllamaProvider;
pub use openai_compat::{OpenAiCompatProvider, OpenAiCompatProviderBuilder};
pub use provider::{ChatRequest, ChatResponse, Provider, StopReason, ToolSchema, Usage};
pub use quirks::{ExpectedFormat, ProviderQuirks, quirks_for};
