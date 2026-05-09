//! Anthropic Messages API provider implementation.

use std::{env, time::Duration};

use async_trait::async_trait;
use helm_core::{ProviderError, Secret};
use reqwest::{Client, StatusCode};
#[cfg(test)]
use serde::Deserialize;
use serde::Serialize;
use tokio::time::sleep;

use crate::provider::{ChatRequest, ChatResponse, Provider, ToolSchema};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const DEFAULT_MODEL: &str = "claude-opus-4-1-20250805";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Provider backed by Anthropic's Messages API.
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    client: Client,
    api_key: Secret,
    base_url: String,
    retry_delays: Vec<Duration>,
}

impl AnthropicProvider {
    /// Builds an Anthropic provider from `ANTHROPIC_API_KEY`.
    pub fn from_env() -> Result<Self, ProviderError> {
        let api_key = env::var("ANTHROPIC_API_KEY")
            .map_err(|_| ProviderError::MissingConfig("ANTHROPIC_API_KEY is not set".to_owned()))?;
        Self::new(api_key)
    }

    /// Builds an Anthropic provider for the default API endpoint.
    pub fn new(api_key: impl Into<Secret>) -> Result<Self, ProviderError> {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    /// Builds an Anthropic provider with a custom base URL for tests.
    pub fn with_base_url(
        api_key: impl Into<Secret>,
        base_url: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        Self::with_base_url_and_retry_delays(
            api_key,
            base_url,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ],
        )
    }

    /// Builds an Anthropic provider with custom retry delays for deterministic tests.
    pub fn with_base_url_and_retry_delays(
        api_key: impl Into<Secret>,
        base_url: impl Into<String>,
        retry_delays: Vec<Duration>,
    ) -> Result<Self, ProviderError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(Self {
            client,
            api_key: api_key.into(),
            base_url: base_url.into(),
            retry_delays,
        })
    }

    /// Returns the production default model ID for Anthropic requests.
    pub fn default_model() -> &'static str {
        DEFAULT_MODEL
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
    }

    async fn post_once(&self, request: &ChatRequest) -> Result<ProviderAttempt, ProviderError> {
        let response = self
            .client
            .post(self.endpoint())
            .header("x-api-key", self.api_key.expose())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&AnthropicRequest::from_chat_request(request))
            .send()
            .await
            .map_err(map_reqwest_error)?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| ProviderError::Request(error.to_string()))?;

        if status.is_success() {
            let parsed = serde_json::from_str::<ChatResponse>(&body)
                .map_err(|error| ProviderError::MalformedResponse(error.to_string()))?;
            return Ok(ProviderAttempt::Success(parsed));
        }

        Ok(ProviderAttempt::Status {
            status,
            body,
            retryable: status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error(),
        })
    }

    async fn wait_before_retry(&self, attempt_index: usize) {
        let base_delay = self
            .retry_delays
            .get(attempt_index)
            .copied()
            .unwrap_or_else(|| Duration::from_secs(0));
        let delay = jitter_delay(base_delay);
        if !delay.is_zero() {
            sleep(delay).await;
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let max_attempts = self.retry_delays.len().max(1);
        let mut last_status: Option<(StatusCode, String)> = None;

        for attempt_index in 0..max_attempts {
            match self.post_once(&request).await? {
                ProviderAttempt::Success(response) => return Ok(response),
                ProviderAttempt::Status {
                    status,
                    body,
                    retryable,
                } => {
                    if !retryable {
                        return Err(ProviderError::HttpStatus {
                            status: status.as_u16(),
                            body,
                        });
                    }
                    last_status = Some((status, body));
                    if attempt_index + 1 < max_attempts {
                        self.wait_before_retry(attempt_index).await;
                    }
                }
            }
        }

        match last_status {
            Some((status, body)) => Err(ProviderError::HttpStatus {
                status: status.as_u16(),
                body,
            }),
            None => Err(ProviderError::Other(
                "anthropic request failed without status".to_owned(),
            )),
        }
    }
}

#[derive(Debug)]
enum ProviderAttempt {
    Success(ChatResponse),
    Status {
        status: StatusCode,
        body: String,
        retryable: bool,
    },
}

#[derive(Debug, Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: &'a [helm_core::Message],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<&'a ToolSchema>,
    max_tokens: u32,
    temperature: f32,
}

impl<'a> AnthropicRequest<'a> {
    fn from_chat_request(request: &'a ChatRequest) -> Self {
        Self {
            model: &request.model,
            system: request.system.as_deref(),
            messages: &request.messages,
            tools: request.tools.iter().collect(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
struct AnthropicErrorBody {
    error: Option<AnthropicErrorMessage>,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
struct AnthropicErrorMessage {
    message: String,
}

fn map_reqwest_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Request(error.to_string())
    }
}

fn jitter_delay(base_delay: Duration) -> Duration {
    if base_delay.is_zero() {
        return base_delay;
    }

    let nanos = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => u64::from(duration.subsec_nanos()),
        Err(_) => 0,
    };
    let jitter_window_ms = (base_delay.as_millis() / 10).max(1);
    let offset_ms = u128::from(nanos) % jitter_window_ms;
    let base_ms = base_delay.as_millis();
    let jittered_ms = if nanos % 2 == 0 {
        base_ms.saturating_add(offset_ms)
    } else {
        base_ms.saturating_sub(offset_ms)
    };
    let millis = u64::try_from(jittered_ms).unwrap_or(u64::MAX);
    Duration::from_millis(millis)
}

#[cfg(test)]
fn extract_error_message(body: &str) -> String {
    match serde_json::from_str::<AnthropicErrorBody>(body) {
        Ok(parsed) => match parsed.error {
            Some(error) => error.message,
            None => body.to_owned(),
        },
        Err(_) => body.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::{io, time::Duration};

    use helm_core::{ContentBlock, Secret};
    use mockito::Matcher;
    use serde_json::json;
    use tokio::{io::AsyncWriteExt, net::TcpListener};

    use crate::provider::{ChatRequest, Provider, StopReason, Usage};

    use super::{AnthropicProvider, extract_error_message, jitter_delay};

    fn request() -> ChatRequest {
        ChatRequest {
            model: AnthropicProvider::default_model().to_owned(),
            system: Some("system".to_owned()),
            messages: vec![helm_core::Message::user("hello")],
            tools: Vec::new(),
            max_tokens: 128,
            temperature: 0.0,
        }
    }

    fn success_body() -> String {
        json!({
            "id": "msg_1",
            "content": [{ "type": "text", "text": "ok" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 3, "output_tokens": 4 }
        })
        .to_string()
    }

    #[tokio::test]
    async fn success_happy_path() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .match_header("anthropic-version", "2023-06-01")
            .match_header(
                "content-type",
                Matcher::Regex("application/json.*".to_owned()),
            )
            .with_status(200)
            .with_body(success_body())
            .create_async()
            .await;
        let provider =
            AnthropicProvider::with_base_url_and_retry_delays("key", server.url(), Vec::new())
                .unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert_eq!(response.id, "msg_1");
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.usage.input_tokens, 3);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn retry_429_then_success() {
        let mut server = mockito::Server::new_async().await;
        let retry = server
            .mock("POST", "/v1/messages")
            .with_status(429)
            .with_body("rate limited")
            .expect(1)
            .create_async()
            .await;
        let success = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_body(success_body())
            .expect(1)
            .create_async()
            .await;
        let provider = AnthropicProvider::with_base_url_and_retry_delays(
            "key",
            server.url(),
            vec![Duration::from_millis(0), Duration::from_millis(0)],
        )
        .unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert_eq!(response.id, "msg_1");
        retry.assert_async().await;
        success.assert_async().await;
    }

    #[tokio::test]
    async fn retry_500_then_fail_error_path() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(500)
            .with_body("server failed")
            .expect(2)
            .create_async()
            .await;
        let provider = AnthropicProvider::with_base_url_and_retry_delays(
            "key",
            server.url(),
            vec![Duration::from_millis(0), Duration::from_millis(0)],
        )
        .unwrap();

        let error = provider.chat(request()).await.unwrap_err();

        assert!(error.to_string().contains("500"));
        assert!(error.to_string().contains("server failed"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn malformed_json_errors() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_body("{bad json")
            .create_async()
            .await;
        let provider =
            AnthropicProvider::with_base_url_and_retry_delays("key", server.url(), Vec::new())
                .unwrap();

        let error = provider.chat(request()).await.unwrap_err();

        assert!(error.to_string().contains("malformed"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn network_timeout_errors_edge_case() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(b"").await;
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Ok::<(), io::Error>(())
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(25))
            .build()
            .unwrap();
        let provider = AnthropicProvider {
            client,
            api_key: Secret::new("key"),
            base_url: format!("http://{addr}"),
            retry_delays: Vec::new(),
        };

        let error = provider.chat(request()).await.unwrap_err();
        let _ = server.await.unwrap();

        assert!(matches!(error, helm_core::ProviderError::Timeout));
    }

    #[test]
    fn retry_delay_jitter_preserves_zero_edge_case() {
        assert_eq!(
            jitter_delay(Duration::from_millis(0)),
            Duration::from_millis(0)
        );
    }

    #[test]
    fn error_body_extraction_happy_path() {
        let body = json!({"error": {"message": "bad key"}}).to_string();
        assert_eq!(extract_error_message(&body), "bad key");
    }

    #[test]
    fn response_with_tool_use_round_trips() {
        let body = json!({
            "id": "msg_2",
            "content": [{
                "type": "tool_use",
                "id": "toolu_1",
                "name": "shell",
                "input": { "command": "echo", "args": ["hi"] }
            }],
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 10, "output_tokens": 5 }
        });
        let response: crate::provider::ChatResponse = serde_json::from_value(body).unwrap();

        assert_eq!(
            response.content,
            vec![ContentBlock::ToolUse {
                id: "toolu_1".to_owned(),
                name: "shell".to_owned(),
                input: json!({"command": "echo", "args": ["hi"]})
            }]
        );
        assert_eq!(
            response.usage,
            Usage {
                input_tokens: 10,
                output_tokens: 5
            }
        );
    }
}
