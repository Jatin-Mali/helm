//! OpenAI Chat Completions compatible provider implementation.

use std::{collections::HashSet, env, time::Duration};

use async_trait::async_trait;
use helm_core::{ContentBlock, Message, ProviderError, Role};
use reqwest::{
    Client, StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::time::sleep;

use crate::provider::{ChatRequest, ChatResponse, Provider, StopReason, ToolSchema, Usage};

const GROQ_BASE_URL: &str = "https://api.groq.com/openai/v1";
const GROQ_DEFAULT_MODEL: &str = "openai/gpt-oss-20b";
const GROQ_MAX_TOKENS: u32 = 1_024;
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const OPENROUTER_DEFAULT_MODEL: &str = "meta-llama/llama-3.3-70b-instruct";
const NVIDIA_NIM_BASE_URL: &str = "https://integrate.api.nvidia.com/v1";
const NVIDIA_NIM_DEFAULT_MODEL: &str = "meta/llama-3.3-70b-instruct";
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const OPENAI_DEFAULT_MODEL: &str = "gpt-4o-mini";

/// Provider backed by OpenAI Chat Completions compatible HTTP APIs.
#[derive(Debug, Clone)]
pub struct OpenAiCompatProvider {
    base_url: String,
    api_key: Option<String>,
    default_model: String,
    http: Client,
    /// Used in `name()` for logs and episodes; for example `groq`.
    label: &'static str,
    extra_headers: Vec<(HeaderName, HeaderValue)>,
    retry_delays: Vec<Duration>,
}

impl OpenAiCompatProvider {
    /// Creates a builder for an OpenAI-compatible provider.
    pub fn builder() -> OpenAiCompatProviderBuilder {
        OpenAiCompatProviderBuilder::default()
    }

    /// Builds a Groq provider from `GROQ_API_KEY`.
    pub fn groq_from_env() -> Result<Self, ProviderError> {
        let api_key = require_env("GROQ_API_KEY")?;
        Self::groq(api_key)
    }

    /// Builds a Groq provider from an explicit API key.
    pub fn groq(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        Self::builder()
            .base_url(GROQ_BASE_URL)
            .api_key(api_key)
            .default_model(GROQ_DEFAULT_MODEL)
            .label("groq")
            .build()
    }

    /// Builds an OpenRouter provider from `OPENROUTER_API_KEY`.
    pub fn openrouter_from_env() -> Result<Self, ProviderError> {
        let api_key = require_env("OPENROUTER_API_KEY")?;
        Self::openrouter(api_key)
    }

    /// Builds an OpenRouter provider from an explicit API key.
    pub fn openrouter(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        let mut provider = Self::builder()
            .base_url(OPENROUTER_BASE_URL)
            .api_key(api_key)
            .default_model(OPENROUTER_DEFAULT_MODEL)
            .label("openrouter")
            .build()?;
        provider.add_header("HTTP-Referer", "https://github.com/helm")?;
        provider.add_header("X-Title", "HELM")?;
        Ok(provider)
    }

    /// Builds an NVIDIA NIM provider from `NVIDIA_API_KEY`.
    pub fn nvidia_nim_from_env() -> Result<Self, ProviderError> {
        let api_key = require_env("NVIDIA_API_KEY")?;
        Self::nvidia_nim(api_key)
    }

    /// Builds an NVIDIA NIM provider from an explicit API key.
    pub fn nvidia_nim(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        Self::builder()
            .base_url(NVIDIA_NIM_BASE_URL)
            .api_key(api_key)
            .default_model(NVIDIA_NIM_DEFAULT_MODEL)
            .label("nvidia-nim")
            .build()
    }

    /// Builds an OpenAI API provider from `OPENAI_API_KEY`.
    pub fn openai_from_env() -> Result<Self, ProviderError> {
        let api_key = require_env("OPENAI_API_KEY")?;
        Self::openai(api_key)
    }

    /// Builds an OpenAI API provider from an explicit API key.
    pub fn openai(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        Self::builder()
            .base_url(OPENAI_BASE_URL)
            .api_key(api_key)
            .default_model(OPENAI_DEFAULT_MODEL)
            .label("openai")
            .build()
    }

    /// Returns the configured provider default model.
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// Returns the configured API base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn add_header(&mut self, name: &str, value: &str) -> Result<(), ProviderError> {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| ProviderError::MissingConfig(error.to_string()))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| ProviderError::MissingConfig(error.to_string()))?;
        self.extra_headers.push((header_name, header_value));
        Ok(())
    }

    async fn post_once(&self, request: &ChatRequest) -> Result<ProviderAttempt, ProviderError> {
        let body = OpenAiRequest::from_chat_request(request, self.default_model(), self.label)?;
        let mut builder = self
            .http
            .post(self.endpoint())
            .header(CONTENT_TYPE, "application/json");
        if let Some(api_key) = &self.api_key {
            builder = builder.header(AUTHORIZATION, format!("Bearer {api_key}"));
        }
        for (name, value) in &self.extra_headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let status = response.status();
        let headers = response.headers().clone();
        let text = response
            .text()
            .await
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        if status.is_success() {
            let parsed = serde_json::from_str::<OpenAiResponse>(&text)
                .map_err(|error| ProviderError::MalformedResponse(error.to_string()))?;
            return Ok(ProviderAttempt::Success(parsed.into_chat_response()?));
        }
        Ok(ProviderAttempt::Status {
            status,
            retry_after: retry_after_delay(&headers, &text),
            body: text,
            retryable: status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error(),
        })
    }

    async fn wait_before_retry(&self, attempt_index: usize, retry_after: Option<Duration>) {
        let base_delay = self
            .retry_delays
            .get(attempt_index)
            .copied()
            .unwrap_or_else(|| Duration::from_secs(0));
        let delay = retry_after.unwrap_or_else(|| jitter_delay(base_delay));
        if !delay.is_zero() {
            sleep(delay).await;
        }
    }
}

#[async_trait]
impl Provider for OpenAiCompatProvider {
    fn name(&self) -> &'static str {
        self.label
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let max_attempts = self.retry_delays.len().max(1);
        let mut last_status = None;
        for attempt_index in 0..max_attempts {
            match self.post_once(&request).await? {
                ProviderAttempt::Success(response) => return Ok(response),
                ProviderAttempt::Status {
                    status,
                    body,
                    retry_after,
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
                        self.wait_before_retry(attempt_index, retry_after).await;
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
                "openai-compatible request failed without status".to_owned(),
            )),
        }
    }
}

/// Builder for `OpenAiCompatProvider`.
#[derive(Debug, Default)]
pub struct OpenAiCompatProviderBuilder {
    base_url: Option<String>,
    api_key: Option<String>,
    default_model: Option<String>,
    label: Option<&'static str>,
}

impl OpenAiCompatProviderBuilder {
    /// Sets the OpenAI-compatible API base URL.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Sets the optional bearer API key.
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Sets the provider default model.
    pub fn default_model(mut self, default_model: impl Into<String>) -> Self {
        self.default_model = Some(default_model.into());
        self
    }

    /// Sets the stable provider label returned by `Provider::name`.
    pub fn label(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }

    /// Validates fields and builds the provider.
    pub fn build(self) -> Result<OpenAiCompatProvider, ProviderError> {
        let base_url = require_nonempty(self.base_url, "base_url")?;
        let default_model = require_nonempty(self.default_model, "default_model")?;
        let label = self.label.ok_or_else(|| {
            ProviderError::MissingConfig("openai-compatible label is required".to_owned())
        })?;
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(OpenAiCompatProvider {
            base_url,
            api_key: self.api_key.filter(|value| !value.trim().is_empty()),
            default_model,
            http,
            label,
            extra_headers: Vec::new(),
            retry_delays: vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ],
        })
    }
}

#[derive(Debug)]
enum ProviderAttempt {
    Success(ChatResponse),
    Status {
        status: StatusCode,
        retry_after: Option<Duration>,
        body: String,
        retryable: bool,
    },
}

fn retry_after_delay(headers: &HeaderMap, body: &str) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after_seconds)
        .or_else(|| parse_groq_retry_after(body))
}

fn parse_retry_after_seconds(value: &str) -> Option<Duration> {
    let seconds = value.trim().parse::<f64>().ok()?;
    finite_positive_duration(seconds)
}

fn parse_groq_retry_after(body: &str) -> Option<Duration> {
    let marker = "Please try again in ";
    let start = body.find(marker)? + marker.len();
    let tail = &body[start..];
    let seconds_text = tail
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>();
    let seconds = seconds_text.parse::<f64>().ok()?;
    finite_positive_duration(seconds)
}

fn finite_positive_duration(seconds: f64) -> Option<Duration> {
    if seconds.is_finite() && seconds > 0.0 {
        Some(Duration::from_secs_f64(seconds))
    } else {
        None
    }
}

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    tools: Vec<OpenAiTool>,
    temperature: f32,
    max_tokens: u32,
}

impl OpenAiRequest {
    fn from_chat_request(
        request: &ChatRequest,
        default_model: &str,
        provider_label: &str,
    ) -> Result<Self, ProviderError> {
        let mut messages = Vec::new();
        let mut known_tool_ids = HashSet::new();
        if let Some(system) = &request.system {
            messages.push(OpenAiMessage::text("system", system.clone()));
        }
        for message in &request.messages {
            messages.extend(message_to_openai(message, &mut known_tool_ids)?);
        }
        let model = if request.model.trim().is_empty() {
            default_model.to_owned()
        } else {
            request.model.clone()
        };
        Ok(Self {
            model,
            messages,
            tools: request
                .tools
                .iter()
                .map(OpenAiTool::from_tool_schema)
                .collect(),
            temperature: request.temperature,
            max_tokens: provider_max_tokens(provider_label, request.max_tokens),
        })
    }
}

fn provider_max_tokens(provider_label: &str, requested: u32) -> u32 {
    if provider_label == "groq" {
        requested.min(GROQ_MAX_TOKENS)
    } else {
        requested
    }
}

#[derive(Debug, Serialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<OpenAiToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

impl OpenAiMessage {
    fn text(role: &str, content: String) -> Self {
        Self {
            role: role.to_owned(),
            content: Some(content),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiToolFunction,
}

impl OpenAiTool {
    fn from_tool_schema(schema: &ToolSchema) -> Self {
        Self {
            tool_type: "function".to_owned(),
            function: OpenAiToolFunction {
                name: schema.name.clone(),
                description: schema.description.clone(),
                parameters: schema.input_schema.clone(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenAiToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    #[serde(default)]
    id: String,
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: OpenAiUsage,
}

impl OpenAiResponse {
    fn into_chat_response(self) -> Result<ChatResponse, ProviderError> {
        let choice = self.choices.into_iter().next().ok_or_else(|| {
            ProviderError::MalformedResponse("openai-compatible response had no choices".to_owned())
        })?;
        let mut content = Vec::new();
        if let Some(text) = choice.message.content.filter(|text| !text.is_empty()) {
            content.push(ContentBlock::Text(text));
        }
        for call in choice.message.tool_calls {
            let input = parse_arguments(&call.function.arguments)?;
            content.push(ContentBlock::ToolUse {
                id: call.id,
                name: call.function.name,
                input,
            });
        }
        let has_tool_calls = content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }));
        let stop_reason = match choice.finish_reason.as_deref() {
            Some("tool_calls") => StopReason::ToolUse,
            Some("stop") if has_tool_calls => StopReason::ToolUse,
            Some("stop") => StopReason::EndTurn,
            Some("length") => StopReason::MaxTokens,
            // OpenAI-compatible content filters have no exact HELM variant.
            Some("content_filter") => StopReason::StopSequence,
            _ if has_tool_calls => StopReason::ToolUse,
            _ => StopReason::EndTurn,
        };
        Ok(ChatResponse {
            id: if self.id.is_empty() {
                "openai-compatible".to_owned()
            } else {
                self.id
            },
            content,
            stop_reason,
            usage: Usage {
                input_tokens: self.usage.prompt_tokens,
                output_tokens: self.usage.completion_tokens,
            },
        })
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAiResponseToolCall>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseToolCall {
    id: String,
    function: OpenAiResponseFunctionCall,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Default, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

fn message_to_openai(
    message: &Message,
    known_tool_ids: &mut HashSet<String>,
) -> Result<Vec<OpenAiMessage>, ProviderError> {
    match message.role {
        Role::System => message_text_only(message, "system"),
        Role::Assistant => assistant_message(message, known_tool_ids),
        Role::User => user_message(message, known_tool_ids),
    }
}

fn message_text_only(message: &Message, role: &str) -> Result<Vec<OpenAiMessage>, ProviderError> {
    let mut parts = Vec::new();
    for block in &message.content {
        match block {
            ContentBlock::Text(text) => parts.push(text.clone()),
            ContentBlock::ToolUse { id, .. } => {
                return Err(ProviderError::InvalidConversation {
                    reason: format!("tool_use {id} appeared in a {role} message"),
                });
            }
            ContentBlock::ToolResult { tool_use_id, .. } => {
                return Err(ProviderError::InvalidConversation {
                    reason: format!("tool_result {tool_use_id} appeared in a {role} message"),
                });
            }
        }
    }
    Ok(vec![OpenAiMessage::text(role, parts.join("\n"))])
}

fn assistant_message(
    message: &Message,
    known_tool_ids: &mut HashSet<String>,
) -> Result<Vec<OpenAiMessage>, ProviderError> {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
    for block in &message.content {
        match block {
            ContentBlock::Text(text) => text_parts.push(text.clone()),
            ContentBlock::ToolUse { id, name, input } => {
                known_tool_ids.insert(id.clone());
                let arguments = serde_json::to_string(input)
                    .map_err(|error| ProviderError::MalformedResponse(error.to_string()))?;
                tool_calls.push(OpenAiToolCall {
                    id: id.clone(),
                    tool_type: "function".to_owned(),
                    function: OpenAiFunctionCall {
                        name: name.clone(),
                        arguments,
                    },
                });
            }
            ContentBlock::ToolResult { tool_use_id, .. } => {
                return Err(ProviderError::InvalidConversation {
                    reason: format!("tool_result {tool_use_id} appeared in an assistant message"),
                });
            }
        }
    }
    Ok(vec![OpenAiMessage {
        role: "assistant".to_owned(),
        content: Some(text_parts.join("\n")),
        tool_calls,
        tool_call_id: None,
    }])
}

fn user_message(
    message: &Message,
    known_tool_ids: &HashSet<String>,
) -> Result<Vec<OpenAiMessage>, ProviderError> {
    let mut messages = Vec::new();
    let mut text_parts = Vec::new();
    for block in &message.content {
        match block {
            ContentBlock::Text(text) => text_parts.push(text.clone()),
            ContentBlock::ToolUse { id, .. } => {
                return Err(ProviderError::InvalidConversation {
                    reason: format!("tool_use {id} appeared in a user message"),
                });
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                if !text_parts.is_empty() {
                    messages.push(OpenAiMessage::text("user", text_parts.join("\n")));
                    text_parts.clear();
                }
                if !known_tool_ids.contains(tool_use_id) {
                    return Err(ProviderError::InvalidConversation {
                        reason: format!(
                            "tool_result references unknown tool_use_id: {tool_use_id}"
                        ),
                    });
                }
                messages.push(OpenAiMessage {
                    role: "tool".to_owned(),
                    content: Some(content.clone()),
                    tool_calls: Vec::new(),
                    tool_call_id: Some(tool_use_id.clone()),
                });
            }
        }
    }
    if !text_parts.is_empty() || messages.is_empty() {
        messages.push(OpenAiMessage::text("user", text_parts.join("\n")));
    }
    Ok(messages)
}

fn parse_arguments(arguments: &str) -> Result<Value, ProviderError> {
    if arguments.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(arguments).map_err(|error| {
        ProviderError::MalformedResponse(format!(
            "invalid tool call arguments JSON string: {error}"
        ))
    })
}

fn require_env(name: &str) -> Result<String, ProviderError> {
    env::var(name).map_err(|_| ProviderError::MissingConfig(format!("{name} is not set")))
}

fn require_nonempty(value: Option<String>, name: &str) -> Result<String, ProviderError> {
    let value = value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ProviderError::MissingConfig(format!("openai-compatible {name} is required"))
        })?;
    Ok(value)
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
mod tests {
    use std::time::Duration;

    use helm_core::{ContentBlock, Message, ProviderError};
    use mockito::Matcher;
    use serde_json::json;

    use crate::provider::{ChatRequest, Provider, StopReason, ToolSchema};

    use super::{OpenAiCompatProvider, OpenAiRequest, OpenAiResponse};

    fn request() -> ChatRequest {
        ChatRequest {
            model: "llama-3.1-8b-instant".to_owned(),
            system: Some("sys".to_owned()),
            messages: vec![Message::user("hi")],
            tools: vec![ToolSchema {
                name: "shell".to_owned(),
                description: "run command".to_owned(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: 64,
            temperature: 0.0,
        }
    }

    fn provider(base_url: String) -> OpenAiCompatProvider {
        let mut provider = OpenAiCompatProvider::builder()
            .base_url(base_url)
            .api_key("key")
            .default_model("model")
            .label("test")
            .build()
            .unwrap();
        provider.retry_delays = vec![Duration::ZERO, Duration::ZERO, Duration::ZERO];
        provider
    }

    fn success_body(finish_reason: &str) -> String {
        json!({
            "id": "chatcmpl_1",
            "model": "m",
            "choices": [{
                "message": {"role": "assistant", "content": "done"},
                "finish_reason": finish_reason
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}
        })
        .to_string()
    }

    #[test]
    fn builder_validates_required_fields_error_path() {
        let error = OpenAiCompatProvider::builder()
            .default_model("m")
            .label("x")
            .build()
            .unwrap_err();

        assert!(error.to_string().contains("base_url"));
    }

    #[test]
    fn builder_accepts_local_no_key_edge_case() {
        let provider = OpenAiCompatProvider::builder()
            .base_url("http://localhost:8000/v1")
            .default_model("local")
            .label("local")
            .build()
            .unwrap();

        assert_eq!(provider.name(), "local");
        assert_eq!(provider.default_model(), "local");
    }

    #[test]
    fn openrouter_constructor_adds_best_practice_headers() {
        let provider = OpenAiCompatProvider::openrouter("key").unwrap();
        let names = provider
            .extra_headers
            .iter()
            .map(|(name, _value)| name.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"http-referer"));
        assert!(names.contains(&"x-title"));
    }

    #[tokio::test]
    async fn tool_use_round_trip_parses_arguments_string_happy_path() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_body(
                json!({
                    "id": "chatcmpl_1",
                    "model": "m",
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "",
                            "tool_calls": [{
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "shell",
                                    "arguments": "{\"command\":\"echo\",\"args\":[\"hi\"]}"
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }],
                    "usage": {"prompt_tokens": 5, "completion_tokens": 6}
                })
                .to_string(),
            )
            .create_async()
            .await;
        let response = provider(server.url()).chat(request()).await.unwrap();

        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.usage.input_tokens, 5);
        assert_eq!(response.usage.output_tokens, 6);
        match response.content.first() {
            Some(ContentBlock::ToolUse { id, name, input }) => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "shell");
                assert_eq!(input, &json!({"command": "echo", "args": ["hi"]}));
            }
            other => panic!("expected tool use, got {other:?}"),
        }
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn tool_result_message_has_correct_tool_call_id() {
        let mut request = request();
        request.messages = vec![
            Message::user("run"),
            Message::assistant(vec![ContentBlock::ToolUse {
                id: "call_1".to_owned(),
                name: "shell".to_owned(),
                input: json!({"command": "pwd"}),
            }]),
            Message::tool_results(vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_owned(),
                content: "/tmp".to_owned(),
                is_error: false,
            }]),
        ];
        let expected = json!({
            "model": "llama-3.1-8b-instant",
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "run"},
                {"role": "assistant", "content": "", "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "shell", "arguments": "{\"command\":\"pwd\"}"}
                }]},
                {"role": "tool", "content": "/tmp", "tool_call_id": "call_1"}
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "shell",
                    "description": "run command",
                    "parameters": {"type": "object"}
                }
            }],
            "temperature": 0.0,
            "max_tokens": 64
        });
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .match_header("authorization", "Bearer key")
            .match_body(Matcher::Json(expected))
            .with_status(200)
            .with_body(success_body("stop"))
            .create_async()
            .await;

        provider(server.url()).chat(request).await.unwrap();

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn null_content_on_assistant_tool_call_is_handled() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_body(
                json!({
                    "id": "chatcmpl_1",
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": "call_1",
                                "function": {"name": "shell", "arguments": "{}"}
                            }]
                        },
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let response = provider(server.url()).chat(request()).await.unwrap();

        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert!(matches!(
            response.content.first(),
            Some(ContentBlock::ToolUse { .. })
        ));
        mock.assert_async().await;
    }

    #[test]
    fn finish_reasons_map_to_stop_reason() {
        for (finish_reason, expected) in [
            ("stop", StopReason::EndTurn),
            ("length", StopReason::MaxTokens),
            ("content_filter", StopReason::StopSequence),
            ("tool_calls", StopReason::ToolUse),
        ] {
            let response: OpenAiResponse =
                serde_json::from_str(&success_body(finish_reason)).unwrap();

            assert_eq!(response.into_chat_response().unwrap().stop_reason, expected);
        }
    }

    #[tokio::test]
    async fn retry_429_then_success() {
        let mut server = mockito::Server::new_async().await;
        let first = server
            .mock("POST", "/chat/completions")
            .with_status(429)
            .with_body("rate limited")
            .create_async()
            .await;
        let second = server
            .mock("POST", "/chat/completions")
            .with_status(200)
            .with_body(success_body("stop"))
            .create_async()
            .await;

        let response = provider(server.url()).chat(request()).await.unwrap();

        assert_eq!(response.stop_reason, StopReason::EndTurn);
        first.assert_async().await;
        second.assert_async().await;
    }

    #[tokio::test]
    async fn retry_500_exhausts_error_path() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(500)
            .with_body("broken")
            .expect(3)
            .create_async()
            .await;

        let error = provider(server.url()).chat(request()).await.unwrap_err();

        assert!(matches!(
            error,
            ProviderError::HttpStatus { status: 500, .. }
        ));
        mock.assert_async().await;
    }

    #[test]
    fn missing_type_on_tool_calls_is_tolerated() {
        let response: OpenAiResponse = serde_json::from_value(json!({
            "id": "chatcmpl_1",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_1",
                        "function": {"name": "shell", "arguments": "{}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }))
        .unwrap();

        assert_eq!(
            response.into_chat_response().unwrap().stop_reason,
            StopReason::ToolUse
        );
    }

    #[test]
    fn unknown_tool_result_id_errors_edge_case() {
        let mut request = request();
        request.messages = vec![Message::tool_results(vec![ContentBlock::ToolResult {
            tool_use_id: "missing".to_owned(),
            content: "x".to_owned(),
            is_error: false,
        }])];

        let error = OpenAiRequest::from_chat_request(&request, "model", "test").unwrap_err();

        assert!(matches!(error, ProviderError::InvalidConversation { .. }));
    }

    #[test]
    fn groq_caps_requested_max_tokens_edge_case() {
        let mut request = request();
        request.max_tokens = 50_000;

        let openai_request = OpenAiRequest::from_chat_request(&request, "model", "groq").unwrap();

        assert_eq!(openai_request.max_tokens, 1_024);
    }

    #[test]
    fn non_groq_keeps_requested_max_tokens_happy_path() {
        let mut request = request();
        request.max_tokens = 2_048;

        let openai_request = OpenAiRequest::from_chat_request(&request, "model", "openai").unwrap();

        assert_eq!(openai_request.max_tokens, 2_048);
    }

    #[test]
    fn parses_retry_after_header_happy_path() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "2.5".parse().unwrap());

        let delay = super::retry_after_delay(&headers, "{}").unwrap();

        assert_eq!(delay.as_millis(), 2_500);
    }

    #[test]
    fn parses_groq_retry_after_body_edge_case() {
        let body = r#"{"error":{"message":"Rate limit reached. Please try again in 13.94s."}}"#;
        let headers = reqwest::header::HeaderMap::new();

        let delay = super::retry_after_delay(&headers, body).unwrap();

        assert_eq!(delay.as_millis(), 13_940);
    }
}
