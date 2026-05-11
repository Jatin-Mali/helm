//! Shared HELM types used by providers, tools, memory, and the ReAct agent.

pub mod capability;
pub mod error;
pub mod message;
pub mod secret;
pub mod taint;
pub mod validation;

pub use capability::{Capability, GrantScope};
pub use error::{BudgetError, HelmError, MemoryError, ProviderError, ToolError};
pub use message::{ContentBlock, Message, Role};
pub use secret::{RotationPolicy, Secret, SecretError, SecretStore, redact_secrets};
pub use taint::{Taint, Tainted};
pub use validation::{ValidationError, Validator};
