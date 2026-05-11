//! Typed error definitions shared across HELM crates.

use std::{io, path::PathBuf};

use thiserror::Error;

/// Top-level error type for HELM operations that cross crate boundaries.
#[derive(Debug, Error)]
pub enum HelmError {
    /// Filesystem, process, or other operating-system I/O failed.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    /// JSON serialization or deserialization failed.
    #[error("json error: {0}")]
    Serde(#[from] serde_json::Error),
    /// An LLM provider failed before producing a valid response.
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    /// A tool invocation failed.
    #[error("tool error: {0}")]
    Tool(#[from] ToolError),
    /// The episode memory store failed.
    #[error("memory error: {0}")]
    Memory(#[from] MemoryError),
    /// The run budget was exhausted.
    #[error("budget error: {0}")]
    Budget(#[from] BudgetError),
    /// Input validation failed (e.g., injection attacks detected).
    #[error("validation error: {0}")]
    ValidationFailed(String),
}

/// Error type returned by language model providers.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Required provider configuration was not available.
    #[error("missing configuration: {0}")]
    MissingConfig(String),
    /// The HTTP request failed before a provider response was available.
    #[error("request failed: {0}")]
    Request(String),
    /// The provider returned an unsuccessful HTTP status.
    #[error("provider returned HTTP {status}: {body}")]
    HttpStatus {
        /// Numeric HTTP status code returned by the provider.
        status: u16,
        /// Raw response body returned by the provider.
        body: String,
    },
    /// The provider returned JSON that could not be mapped to HELM types.
    #[error("malformed response: {0}")]
    MalformedResponse(String),
    /// The provider timed out.
    #[error("provider request timed out")]
    Timeout,
    /// The conversation history cannot be represented by the provider wire format.
    #[error("invalid conversation: {reason}")]
    InvalidConversation {
        /// Human-readable reason the provider rejected the local conversation history.
        reason: String,
    },
    /// The mock provider had no remaining scripted responses.
    #[error("mock provider exhausted")]
    Exhausted,
    /// A provider-specific failure that does not fit a narrower variant.
    #[error("{0}")]
    Other(String),
}

/// Error type returned by HELM tools.
#[derive(Debug, Error)]
pub enum ToolError {
    /// Tool input JSON was missing required fields or had invalid types.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// Tool execution exceeded the configured timeout.
    #[error("tool timed out")]
    Timeout,
    /// Tool output exceeded the configured output limit.
    #[error("tool output too large")]
    OutputTooLarge,
    /// Tool execution failed because of an I/O error.
    #[error("io error: {0}")]
    IoError(#[from] io::Error),
    /// The requested path is outside the allowed filesystem roots.
    #[error("path denied: {0}")]
    PathDenied(PathBuf),
    /// A tool-specific failure that does not fit a narrower variant.
    #[error("{0}")]
    Other(String),
}

/// Error type returned by the SQLite-backed episode memory store.
#[derive(Debug, Error)]
pub enum MemoryError {
    /// SQLite returned an error.
    #[error("sqlite error: {0}")]
    Sqlite(String),
    /// JSON serialization or deserialization failed.
    #[error("json error: {0}")]
    Serde(#[from] serde_json::Error),
    /// Schema migration failed or left the database in an unsupported state.
    #[error("migration error: {0}")]
    Migration(String),
    /// A blocking task failed before returning a memory-store result.
    #[error("blocking task failed: {0}")]
    Join(String),
    /// A requested entity was not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// General other error.
    #[error("error: {0}")]
    Other(String),
    /// Invalid input provided.
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

/// Error returned when a ReAct run exceeds its configured budget.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// The run used the maximum allowed number of ReAct iterations.
    #[error("maximum iterations reached")]
    MaxIterations,
    /// The run consumed the maximum allowed provider input tokens.
    #[error("maximum input tokens reached")]
    MaxInputTokens,
    /// The run consumed the maximum allowed provider output tokens.
    #[error("maximum output tokens reached")]
    MaxOutputTokens,
    /// The run exceeded its wall-clock deadline.
    #[error("wall-clock timeout reached")]
    WallTimeout,
    /// The run exceeded its cost limit in USD.
    #[error("cost limit exceeded")]
    CostLimitExceeded,
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{BudgetError, HelmError, MemoryError, ProviderError, ToolError};

    #[test]
    fn helm_error_wraps_provider_happy_path() {
        let error = HelmError::from(ProviderError::Exhausted);
        assert!(error.to_string().contains("mock provider exhausted"));
    }

    #[test]
    fn tool_error_wraps_io_error_path() {
        let error = ToolError::from(io::Error::new(io::ErrorKind::NotFound, "missing"));
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn budget_error_formats_edge_case() {
        assert_eq!(
            BudgetError::WallTimeout.to_string(),
            "wall-clock timeout reached"
        );
    }

    #[test]
    fn memory_error_preserves_migration_error() {
        let error = MemoryError::Migration("bad version".to_owned());
        assert!(error.to_string().contains("bad version"));
    }
}
