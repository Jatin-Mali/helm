//! Scripted provider used by unit and integration tests.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use helm_core::ProviderError;

use crate::provider::{ChatRequest, ChatResponse, Provider};

/// Provider that returns a fixed sequence of responses without network access.
#[derive(Debug, Clone)]
pub struct MockProvider {
    responses: Arc<Mutex<VecDeque<ChatResponse>>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl MockProvider {
    /// Creates a mock provider from responses returned in order.
    pub fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns the number of scripted responses not yet consumed.
    pub fn remaining(&self) -> Result<usize, ProviderError> {
        let guard = self.responses.lock().map_err(|error| {
            ProviderError::Other(format!("mock provider lock poisoned: {error}"))
        })?;
        Ok(guard.len())
    }

    /// Returns the chat requests received by this provider.
    pub fn requests(&self) -> Result<Vec<ChatRequest>, ProviderError> {
        let guard = self.requests.lock().map_err(|error| {
            ProviderError::Other(format!("mock provider lock poisoned: {error}"))
        })?;
        Ok(guard.clone())
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        {
            let mut guard = self.requests.lock().map_err(|error| {
                ProviderError::Other(format!("mock provider lock poisoned: {error}"))
            })?;
            guard.push(request);
        }
        let mut guard = self.responses.lock().map_err(|error| {
            ProviderError::Other(format!("mock provider lock poisoned: {error}"))
        })?;
        guard.pop_front().ok_or(ProviderError::Exhausted)
    }
}

#[cfg(test)]
mod tests {
    use helm_core::ContentBlock;

    use crate::provider::{ChatRequest, ChatResponse, Provider, StopReason, Usage};

    use super::MockProvider;

    fn request() -> ChatRequest {
        ChatRequest {
            model: "mock".to_owned(),
            system: None,
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: 1,
            temperature: 0.0,
        }
    }

    fn response(text: &str) -> ChatResponse {
        ChatResponse {
            id: text.to_owned(),
            content: vec![ContentBlock::Text(text.to_owned())],
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        }
    }

    #[tokio::test]
    async fn returns_canned_responses_in_order_happy_path() {
        let provider = MockProvider::new(vec![response("one"), response("two")]);

        assert_eq!(provider.chat(request()).await.unwrap().id, "one");
        assert_eq!(provider.chat(request()).await.unwrap().id, "two");
    }

    #[tokio::test]
    async fn captures_requests_happy_path() {
        let provider = MockProvider::new(vec![response("one")]);

        provider.chat(request()).await.unwrap();

        let requests = provider.requests().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model, "mock");
    }

    #[tokio::test]
    async fn errors_when_exhausted_error_path() {
        let provider = MockProvider::new(Vec::new());

        let error = provider.chat(request()).await.unwrap_err();

        assert!(error.to_string().contains("exhausted"));
    }

    #[test]
    fn remaining_reports_empty_edge_case() {
        let provider = MockProvider::new(Vec::new());

        assert_eq!(provider.remaining().unwrap(), 0);
    }
}
