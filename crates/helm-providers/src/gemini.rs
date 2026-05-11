//! Google Gemini `generateContent` provider implementation.

use std::{collections::HashMap, env, time::Duration};

use async_trait::async_trait;
use helm_core::{ContentBlock, Message, ProviderError, Role, Secret};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::time::sleep;
use uuid::Uuid;

use crate::provider::{ChatRequest, ChatResponse, Provider, StopReason, ToolSchema, Usage};

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const GEMINI_DEFAULT_MODEL: &str = "gemini-2.5-flash";

/// Provider backed by Google's Gemini `generateContent` API.
#[derive(Debug, Clone)]
pub struct GeminiProvider {
    api_key: Secret,
    base_url: String,
    http: Client,
    retry_delays: Vec<Duration>,
}

impl GeminiProvider {
    /// Builds a Gemini provider from `GOOGLE_API_KEY` or `GEMINI_API_KEY`.
    pub fn from_env() -> Result<Self, ProviderError> {
        let api_key = env::var("GOOGLE_API_KEY")
            .or_else(|_| env::var("GEMINI_API_KEY"))
            .map_err(|_| {
                ProviderError::MissingConfig(
                    "GOOGLE_API_KEY or GEMINI_API_KEY is not set".to_owned(),
                )
            })?;
        Self::new(api_key)
    }

    /// Builds a Gemini provider for the default Google endpoint.
    pub fn new(api_key: impl Into<Secret>) -> Result<Self, ProviderError> {
        Self::with_base_url(api_key, GEMINI_BASE_URL)
    }

    /// Builds a Gemini provider with a custom base URL for tests.
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

    /// Builds a Gemini provider with custom retry delays for deterministic tests.
    pub fn with_base_url_and_retry_delays(
        api_key: impl Into<Secret>,
        base_url: impl Into<String>,
        retry_delays: Vec<Duration>,
    ) -> Result<Self, ProviderError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            http,
            retry_delays,
        })
    }

    /// Returns the production default Gemini model.
    pub fn default_model() -> &'static str {
        GEMINI_DEFAULT_MODEL
    }

    fn endpoint(&self, model: &str) -> String {
        format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            self.base_url.trim_end_matches('/'),
            model,
            self.api_key.expose()
        )
    }

    async fn post_once(
        &self,
        model: &str,
        request: &ChatRequest,
    ) -> Result<GeminiAttempt, ProviderError> {
        let response = self
            .http
            .post(self.endpoint(model))
            .header("content-type", "application/json")
            .json(&GeminiRequest::from_chat_request(request)?)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        if status.is_success() {
            let parsed = serde_json::from_str::<GeminiResponse>(&body)
                .map_err(|error| ProviderError::MalformedResponse(error.to_string()))?;
            return Ok(GeminiAttempt::Success(parsed.into_chat_response()?));
        }
        Ok(GeminiAttempt::Status {
            status,
            body,
            retryable: status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error(),
        })
    }

    async fn wait_before_retry(&self, attempt_index: usize) {
        let delay = self
            .retry_delays
            .get(attempt_index)
            .copied()
            .unwrap_or_else(|| Duration::from_secs(0));
        if !delay.is_zero() {
            sleep(delay).await;
        }
    }
}

#[async_trait]
impl Provider for GeminiProvider {
    fn name(&self) -> &'static str {
        "gemini"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let model = if request.model.trim().is_empty() {
            Self::default_model()
        } else {
            request.model.as_str()
        };
        let max_attempts = self.retry_delays.len().max(1);
        let mut last_status = None;
        for attempt_index in 0..max_attempts {
            match self.post_once(model, &request).await? {
                GeminiAttempt::Success(response) => return Ok(response),
                GeminiAttempt::Status {
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
                "gemini request failed without status".to_owned(),
            )),
        }
    }
}

#[derive(Debug)]
enum GeminiAttempt {
    Success(ChatResponse),
    Status {
        status: StatusCode,
        body: String,
        retryable: bool,
    },
}

#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiSystemInstruction>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<GeminiTool>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
}

impl GeminiRequest {
    fn from_chat_request(request: &ChatRequest) -> Result<Self, ProviderError> {
        let mut tool_names_by_id = HashMap::new();
        let mut contents = Vec::new();
        for message in &request.messages {
            contents.push(message_to_gemini(message, &mut tool_names_by_id)?);
        }
        Ok(Self {
            contents,
            system_instruction: request.system.as_ref().map(|text| GeminiSystemInstruction {
                parts: vec![GeminiPart::text(text.clone())],
            }),
            tools: request
                .tools
                .iter()
                .map(GeminiTool::from_tool_schema)
                .collect(),
            generation_config: GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_tokens,
            },
        })
    }
}

#[derive(Debug, Serialize)]
struct GeminiSystemInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GeminiContent {
    #[serde(default)]
    role: String,
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(rename = "functionCall", skip_serializing_if = "Option::is_none")]
    function_call: Option<GeminiFunctionCall>,
    #[serde(rename = "functionResponse", skip_serializing_if = "Option::is_none")]
    function_response: Option<GeminiFunctionResponse>,
}

impl GeminiPart {
    fn text(text: String) -> Self {
        Self {
            text: Some(text),
            function_call: None,
            function_response: None,
        }
    }

    fn function_call(name: String, args: Value) -> Self {
        Self {
            text: None,
            function_call: Some(GeminiFunctionCall { name, args }),
            function_response: None,
        }
    }

    fn function_response(name: String, response: Value) -> Self {
        Self {
            text: None,
            function_call: None,
            function_response: Some(GeminiFunctionResponse { name, response }),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GeminiFunctionResponse {
    name: String,
    #[serde(default)]
    response: Value,
}

#[derive(Debug, Serialize)]
struct GeminiTool {
    #[serde(rename = "functionDeclarations")]
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

impl GeminiTool {
    fn from_tool_schema(schema: &ToolSchema) -> Self {
        Self {
            function_declarations: vec![GeminiFunctionDeclaration {
                name: schema.name.clone(),
                description: schema.description.clone(),
                parameters: sanitize_gemini_schema(&schema.input_schema),
            }],
        }
    }
}

#[derive(Debug, Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
    temperature: f32,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(rename = "usageMetadata", default)]
    usage_metadata: GeminiUsage,
}

impl GeminiResponse {
    fn into_chat_response(self) -> Result<ChatResponse, ProviderError> {
        let candidate = self.candidates.into_iter().next().ok_or_else(|| {
            ProviderError::MalformedResponse("gemini response had no candidates".to_owned())
        })?;
        let mut content = Vec::new();
        for (index, part) in candidate.content.parts.into_iter().enumerate() {
            if let Some(text) = part.text.filter(|text| !text.is_empty()) {
                content.push(ContentBlock::Text(text));
            }
            if let Some(function_call) = part.function_call {
                content.push(ContentBlock::ToolUse {
                    id: format!("gemini_tool_{}_{index}", Uuid::new_v4().simple()),
                    name: function_call.name,
                    input: if function_call.args.is_null() {
                        Value::Object(Map::new())
                    } else {
                        function_call.args
                    },
                });
            }
        }
        let has_tool_calls = content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }));
        let stop_reason = match candidate.finish_reason.as_deref() {
            Some("STOP") if has_tool_calls => StopReason::ToolUse,
            Some("STOP") => StopReason::EndTurn,
            Some("MAX_TOKENS") => StopReason::MaxTokens,
            Some("SAFETY") => StopReason::StopSequence,
            _ if has_tool_calls => StopReason::ToolUse,
            _ => StopReason::EndTurn,
        };
        Ok(ChatResponse {
            id: "gemini".to_owned(),
            content,
            stop_reason,
            usage: Usage {
                input_tokens: self.usage_metadata.prompt_token_count,
                output_tokens: self.usage_metadata.candidates_token_count,
            },
        })
    }
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    #[serde(default)]
    content: GeminiContent,
    #[serde(rename = "finishReason", default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GeminiUsage {
    #[serde(rename = "promptTokenCount", default)]
    prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount", default)]
    candidates_token_count: u32,
}

fn message_to_gemini(
    message: &Message,
    tool_names_by_id: &mut HashMap<String, String>,
) -> Result<GeminiContent, ProviderError> {
    let role = match message.role {
        Role::System | Role::User => "user",
        Role::Assistant => "model",
    };
    let mut parts = Vec::new();
    for block in &message.content {
        match block {
            ContentBlock::Text(text) => parts.push(GeminiPart::text(text.clone())),
            ContentBlock::ToolUse { id, name, input } => {
                if message.role != Role::Assistant {
                    return Err(ProviderError::InvalidConversation {
                        reason: format!("tool_use {id} appeared outside assistant message"),
                    });
                }
                tool_names_by_id.insert(id.clone(), name.clone());
                parts.push(GeminiPart::function_call(name.clone(), input.clone()));
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                if message.role != Role::User {
                    return Err(ProviderError::InvalidConversation {
                        reason: format!("tool_result {tool_use_id} appeared outside user message"),
                    });
                }
                let name = tool_names_by_id.get(tool_use_id).cloned().ok_or_else(|| {
                    ProviderError::InvalidConversation {
                        reason: format!(
                            "tool_result references unknown tool_use_id: {tool_use_id}"
                        ),
                    }
                })?;
                parts.push(GeminiPart::function_response(
                    name,
                    json!({"content": content, "is_error": is_error}),
                ));
            }
        }
    }
    Ok(GeminiContent {
        role: role.to_owned(),
        parts,
    })
}

fn map_reqwest_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Request(error.to_string())
    }
}

fn sanitize_gemini_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(object) => {
            let mut sanitized = Map::new();
            for (key, value) in object {
                if key == "additionalProperties" {
                    continue;
                }
                sanitized.insert(key.clone(), sanitize_gemini_schema(value));
            }
            Value::Object(sanitized)
        }
        Value::Array(values) => Value::Array(values.iter().map(sanitize_gemini_schema).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use helm_core::{ContentBlock, Message, ProviderError};
    use mockito::Matcher;
    use serde_json::json;

    use crate::provider::{ChatRequest, Provider, StopReason, ToolSchema};

    use super::{GeminiProvider, GeminiRequest, sanitize_gemini_schema};

    fn request() -> ChatRequest {
        ChatRequest {
            model: GeminiProvider::default_model().to_owned(),
            system: Some("sys".to_owned()),
            messages: vec![Message::user("hi")],
            tools: vec![ToolSchema {
                name: "shell".to_owned(),
                description: "run command".to_owned(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: 32,
            temperature: 0.0,
        }
    }

    async fn mock_server() -> Option<mockito::ServerGuard> {
        match tokio::spawn(async { mockito::Server::new_async().await }).await {
            Ok(server) => Some(server),
            Err(_) => {
                eprintln!("skipping mockito-backed gemini test: mock server unavailable");
                None
            }
        }
    }

    #[tokio::test]
    async fn text_round_trip_happy_path() {
        let expected = json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "systemInstruction": {"parts": [{"text": "sys"}]},
            "tools": [{"functionDeclarations": [{
                "name": "shell",
                "description": "run command",
                "parameters": {"type": "object"}
            }]}],
            "generationConfig": {"temperature": 0.0, "maxOutputTokens": 32}
        });
        let Some(mut server) = mock_server().await else {
            return;
        };
        let mock = server
            .mock("POST", "/v1beta/models/gemini-2.5-flash:generateContent")
            .match_query(Matcher::UrlEncoded("key".to_owned(), "key".to_owned()))
            .match_body(Matcher::Json(expected))
            .with_status(200)
            .with_body(
                json!({
                    "candidates": [{
                        "content": {"role": "model", "parts": [{"text": "done"}]},
                        "finishReason": "STOP"
                    }],
                    "usageMetadata": {"promptTokenCount": 2, "candidatesTokenCount": 3}
                })
                .to_string(),
            )
            .create_async()
            .await;
        let provider = GeminiProvider::with_base_url("key", server.url()).unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert_eq!(
            response.content,
            vec![ContentBlock::Text("done".to_owned())]
        );
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.usage.input_tokens, 2);
        assert_eq!(response.usage.output_tokens, 3);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn function_call_round_trip() {
        let Some(mut server) = mock_server().await else {
            return;
        };
        let mock = server
            .mock("POST", "/v1beta/models/gemini-2.5-flash:generateContent")
            .match_query(Matcher::UrlEncoded("key".to_owned(), "key".to_owned()))
            .with_status(200)
            .with_body(
                json!({
                    "candidates": [{
                        "content": {"role": "model", "parts": [{
                            "functionCall": {
                                "name": "shell",
                                "args": {"command": "pwd"}
                            }
                        }]},
                        "finishReason": "STOP"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;
        let provider = GeminiProvider::with_base_url("key", server.url()).unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert_eq!(response.stop_reason, StopReason::ToolUse);
        match response.content.first() {
            Some(ContentBlock::ToolUse { id, name, input }) => {
                assert!(id.starts_with("gemini_tool_"));
                assert_eq!(name, "shell");
                assert_eq!(input, &json!({"command": "pwd"}));
            }
            other => panic!("expected tool use, got {other:?}"),
        }
        mock.assert_async().await;
    }

    #[test]
    fn tool_result_maps_to_function_response() {
        let mut request = request();
        request.messages = vec![
            Message::assistant(vec![ContentBlock::ToolUse {
                id: "toolu_1".to_owned(),
                name: "shell".to_owned(),
                input: json!({"command": "pwd"}),
            }]),
            Message::tool_results(vec![ContentBlock::ToolResult {
                tool_use_id: "toolu_1".to_owned(),
                content: "/tmp".to_owned(),
                is_error: false,
            }]),
        ];

        let mapped = GeminiRequest::from_chat_request(&request).unwrap();
        let encoded = serde_json::to_value(mapped).unwrap();

        assert_eq!(
            encoded["contents"][1]["parts"][0]["functionResponse"]["name"],
            "shell"
        );
    }

    #[tokio::test]
    async fn multi_part_response_preserves_text_and_function_call() {
        let Some(mut server) = mock_server().await else {
            return;
        };
        let mock = server
            .mock("POST", "/v1beta/models/gemini-2.5-flash:generateContent")
            .match_query(Matcher::UrlEncoded("key".to_owned(), "key".to_owned()))
            .with_status(200)
            .with_body(
                json!({
                    "candidates": [{
                        "content": {"role": "model", "parts": [
                            {"text": "I will run it."},
                            {"functionCall": {"name": "shell", "args": {"command": "uname"}}}
                        ]},
                        "finishReason": "STOP"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;
        let provider = GeminiProvider::with_base_url("key", server.url()).unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert!(matches!(
            response.content.first(),
            Some(ContentBlock::Text(_))
        ));
        assert!(matches!(
            response.content.get(1),
            Some(ContentBlock::ToolUse { .. })
        ));
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn safety_block_maps_to_stop_sequence() {
        let Some(mut server) = mock_server().await else {
            return;
        };
        let mock = server
            .mock("POST", "/v1beta/models/gemini-2.5-flash:generateContent")
            .match_query(Matcher::UrlEncoded("key".to_owned(), "key".to_owned()))
            .with_status(200)
            .with_body(
                json!({
                    "candidates": [{
                        "content": {"role": "model", "parts": []},
                        "finishReason": "SAFETY"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;
        let provider = GeminiProvider::with_base_url("key", server.url()).unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert_eq!(response.stop_reason, StopReason::StopSequence);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn http_status_error_path() {
        let Some(mut server) = mock_server().await else {
            return;
        };
        let mock = server
            .mock("POST", "/v1beta/models/gemini-2.5-flash:generateContent")
            .match_query(Matcher::UrlEncoded("key".to_owned(), "key".to_owned()))
            .with_status(403)
            .with_body("bad key")
            .create_async()
            .await;
        let provider = GeminiProvider::with_base_url("key", server.url()).unwrap();

        let error = provider.chat(request()).await.unwrap_err();

        assert!(matches!(
            error,
            ProviderError::HttpStatus { status: 403, .. }
        ));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn retry_503_then_success() {
        let Some(mut server) = mock_server().await else {
            return;
        };
        let failing = server
            .mock("POST", "/v1beta/models/gemini-2.5-flash:generateContent")
            .match_query(Matcher::UrlEncoded("key".to_owned(), "key".to_owned()))
            .with_status(503)
            .with_body("busy")
            .expect(1)
            .create_async()
            .await;
        let succeeding = server
            .mock("POST", "/v1beta/models/gemini-2.5-flash:generateContent")
            .match_query(Matcher::UrlEncoded("key".to_owned(), "key".to_owned()))
            .with_status(200)
            .with_body(
                json!({
                    "candidates": [{
                        "content": {"role": "model", "parts": [{"text": "done"}]},
                        "finishReason": "STOP"
                    }]
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        let provider = GeminiProvider::with_base_url_and_retry_delays(
            "key",
            server.url(),
            vec![Duration::ZERO, Duration::ZERO],
        )
        .unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert_eq!(
            response.content,
            vec![ContentBlock::Text("done".to_owned())]
        );
        failing.assert_async().await;
        succeeding.assert_async().await;
    }

    #[tokio::test]
    async fn retry_503_exhausts_error_path() {
        let Some(mut server) = mock_server().await else {
            return;
        };
        let mock = server
            .mock("POST", "/v1beta/models/gemini-2.5-flash:generateContent")
            .match_query(Matcher::UrlEncoded("key".to_owned(), "key".to_owned()))
            .with_status(503)
            .with_body("busy")
            .expect(2)
            .create_async()
            .await;
        let provider = GeminiProvider::with_base_url_and_retry_delays(
            "key",
            server.url(),
            vec![Duration::ZERO, Duration::ZERO],
        )
        .unwrap();

        let error = provider.chat(request()).await.unwrap_err();

        assert!(matches!(
            error,
            ProviderError::HttpStatus { status: 503, .. }
        ));
        mock.assert_async().await;
    }

    #[test]
    fn unknown_tool_result_id_errors_edge_case() {
        let mut request = request();
        request.messages = vec![Message::tool_results(vec![ContentBlock::ToolResult {
            tool_use_id: "missing".to_owned(),
            content: "x".to_owned(),
            is_error: false,
        }])];

        let error = GeminiRequest::from_chat_request(&request).unwrap_err();

        assert!(matches!(error, ProviderError::InvalidConversation { .. }));
    }

    #[test]
    fn gemini_schema_strips_additional_properties_recursively() {
        let input = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "env": {
                    "type": "object",
                    "additionalProperties": {"type": "string"},
                    "properties": {
                        "PATH": {
                            "type": "string",
                            "additionalProperties": false
                        }
                    }
                },
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false
                    }
                }
            }
        });

        let sanitized = sanitize_gemini_schema(&input);

        assert_eq!(
            sanitized,
            json!({
                "type": "object",
                "properties": {
                    "env": {
                        "type": "object",
                        "properties": {
                            "PATH": {
                                "type": "string"
                            }
                        }
                    },
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object"
                        }
                    }
                }
            })
        );
    }
}
