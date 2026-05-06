//! Google Gemini `generateContent` provider implementation.

use std::{collections::HashMap, env, time::Duration};

use async_trait::async_trait;
use helm_core::{ContentBlock, Message, ProviderError, Role};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::provider::{ChatRequest, ChatResponse, Provider, StopReason, ToolSchema, Usage};

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const GEMINI_DEFAULT_MODEL: &str = "gemini-2.5-flash";

/// Provider backed by Google's Gemini `generateContent` API.
#[derive(Debug, Clone)]
pub struct GeminiProvider {
    api_key: String,
    base_url: String,
    http: Client,
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
    pub fn new(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        Self::with_base_url(api_key, GEMINI_BASE_URL)
    }

    /// Builds a Gemini provider with a custom base URL for tests.
    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            http,
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
            self.api_key
        )
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
        let response = self
            .http
            .post(self.endpoint(model))
            .header("content-type", "application/json")
            .json(&GeminiRequest::from_chat_request(&request)?)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        if !status.is_success() {
            return Err(ProviderError::HttpStatus {
                status: status.as_u16(),
                body,
            });
        }
        let parsed = serde_json::from_str::<GeminiResponse>(&body)
            .map_err(|error| ProviderError::MalformedResponse(error.to_string()))?;
        parsed.into_chat_response()
    }
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
                parameters: schema.input_schema.clone(),
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

#[cfg(test)]
mod tests {
    use helm_core::{ContentBlock, Message, ProviderError};
    use mockito::Matcher;
    use serde_json::json;

    use crate::provider::{ChatRequest, Provider, StopReason, ToolSchema};

    use super::{GeminiProvider, GeminiRequest};

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
        let mut server = mockito::Server::new_async().await;
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
        let mut server = mockito::Server::new_async().await;
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
        let mut server = mockito::Server::new_async().await;
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
        let mut server = mockito::Server::new_async().await;
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
        let mut server = mockito::Server::new_async().await;
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
}
