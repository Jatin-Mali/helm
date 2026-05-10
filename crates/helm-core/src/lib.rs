//! Shared HELM types used by providers, tools, memory, and the ReAct agent.

pub mod capability;
pub mod error;
pub mod message;
pub mod redaction;
pub mod secret;
pub mod taint;

pub use capability::{Capability, GrantScope};
pub use error::{BudgetError, HelmError, MemoryError, ProviderError, ToolError};
pub use message::{ContentBlock, Message, Role};
pub use redaction::{redact_content_blocks, redact_text};
pub use secret::Secret;
pub use taint::{Taint, Tainted};
