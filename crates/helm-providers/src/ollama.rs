//! Ollama local provider implementation for development without cloud API keys.

use std::{collections::HashMap, env, time::Duration};

use async_trait::async_trait;
use helm_core::{ContentBlock, Message, ProviderError, Role};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::provider::{ChatRequest, ChatResponse, Provider, StopReason, ToolSchema, Usage};

const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

/// Provider backed by a local Ollama `/api/chat` server.
#[derive(Debug, Clone)]
pub struct OllamaProvider {
    client: Client,
    base_url: String,
}

impl OllamaProvider {
    /// Builds an Ollama provider using `OLLAMA_HOST` or the local default.
    pub fn from_env() -> Result<Self, ProviderError> {
        let base_url =
            env::var("OLLAMA_HOST").unwrap_or_else(|_| DEFAULT_OLLAMA_BASE_URL.to_owned());
        Self::with_base_url(base_url)
    }

    /// Builds an Ollama provider with a custom base URL.
    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, ProviderError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into(),
        })
    }

    /// Returns the default model name used when the CLI selects Ollama without `--model`.
    pub fn default_model() -> &'static str {
        "qwen3:4b"
    }

    fn endpoint(&self) -> String {
        format!("{}/api/chat", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &'static str {
        "ollama"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let response = self
            .client
            .post(self.endpoint())
            .json(&OllamaRequest::from_chat_request(&request)?)
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

        let parsed = serde_json::from_str::<OllamaResponse>(&body)
            .map_err(|error| ProviderError::MalformedResponse(error.to_string()))?;
        parsed.into_chat_response()
    }
}

#[derive(Debug, Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OllamaTool>,
    options: OllamaOptions,
}

impl OllamaRequest {
    fn from_chat_request(request: &ChatRequest) -> Result<Self, ProviderError> {
        let mut messages = Vec::new();
        let mut tool_names_by_id = HashMap::new();
        if let Some(system) = &request.system {
            messages.push(OllamaMessage {
                role: "system".to_owned(),
                content: system.clone(),
                tool_calls: Vec::new(),
                tool_name: None,
            });
        }
        for message in &request.messages {
            messages.extend(message_to_ollama(message, &mut tool_names_by_id)?);
        }
        Ok(Self {
            model: request.model.clone(),
            messages,
            stream: false,
            tools: request
                .tools
                .iter()
                .map(OllamaTool::from_tool_schema)
                .collect(),
            options: OllamaOptions {
                temperature: request.temperature,
            },
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<OllamaToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaToolCall {
    function: OllamaFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    arguments: Value,
}

#[derive(Debug, Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OllamaToolFunction,
}

impl OllamaTool {
    fn from_tool_schema(schema: &ToolSchema) -> Self {
        Self {
            tool_type: "function".to_owned(),
            function: OllamaToolFunction {
                name: schema.name.clone(),
                description: schema.description.clone(),
                parameters: schema.input_schema.clone(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct OllamaToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    #[serde(default)]
    model: String,
    message: OllamaMessage,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
}

impl OllamaResponse {
    fn into_chat_response(self) -> Result<ChatResponse, ProviderError> {
        let mut content = Vec::new();
        if !self.message.content.is_empty() {
            content.push(ContentBlock::Text(self.message.content));
        }
        for (index, call) in self.message.tool_calls.into_iter().enumerate() {
            content.push(ContentBlock::ToolUse {
                id: format!("ollama_tool_{}_{index}", Uuid::new_v4().simple()),
                name: call.function.name,
                input: call.function.arguments,
            });
        }
        let has_tool_calls = content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }));
        let stop_reason = match self.done_reason.as_deref() {
            Some("length") => StopReason::MaxTokens,
            Some("stop") if has_tool_calls => StopReason::ToolUse,
            Some("stop") => StopReason::EndTurn,
            _ if has_tool_calls => StopReason::ToolUse,
            _ => StopReason::EndTurn,
        };
        Ok(ChatResponse {
            id: if self.model.is_empty() {
                "ollama".to_owned()
            } else {
                self.model
            },
            content,
            stop_reason,
            usage: Usage {
                input_tokens: self.prompt_eval_count,
                output_tokens: self.eval_count,
            },
        })
    }
}

fn message_to_ollama(
    message: &Message,
    tool_names_by_id: &mut HashMap<String, String>,
) -> Result<Vec<OllamaMessage>, ProviderError> {
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    match message.role {
        Role::System | Role::Assistant => {
            let mut content_parts = Vec::new();
            let mut tool_calls = Vec::new();
            for block in &message.content {
                match block {
                    ContentBlock::Text(text) => content_parts.push(text.clone()),
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_names_by_id.insert(id.clone(), name.clone());
                        tool_calls.push(OllamaToolCall {
                            function: OllamaFunctionCall {
                                name: name.clone(),
                                arguments: input.clone(),
                            },
                        });
                    }
                    ContentBlock::ToolResult { tool_use_id, .. } => {
                        return Err(ProviderError::InvalidConversation {
                            reason: format!(
                                "tool_result {tool_use_id} appeared in a non-user message"
                            ),
                        });
                    }
                }
            }
            Ok(vec![OllamaMessage {
                role: role.to_owned(),
                content: content_parts.join("\n"),
                tool_calls,
                tool_name: None,
            }])
        }
        Role::User => {
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
                            messages.push(OllamaMessage {
                                role: "user".to_owned(),
                                content: text_parts.join("\n"),
                                tool_calls: Vec::new(),
                                tool_name: None,
                            });
                            text_parts.clear();
                        }
                        let tool_name =
                            tool_names_by_id.get(tool_use_id).cloned().ok_or_else(|| {
                                ProviderError::InvalidConversation {
                                    reason: format!(
                                        "tool_result references unknown tool_use_id: {tool_use_id}"
                                    ),
                                }
                            })?;
                        messages.push(OllamaMessage {
                            role: "tool".to_owned(),
                            content: content.clone(),
                            tool_calls: Vec::new(),
                            tool_name: Some(tool_name),
                        });
                    }
                }
            }
            if !text_parts.is_empty() || messages.is_empty() {
                messages.push(OllamaMessage {
                    role: "user".to_owned(),
                    content: text_parts.join("\n"),
                    tool_calls: Vec::new(),
                    tool_name: None,
                });
            }
            Ok(messages)
        }
    }
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

    use super::{OllamaProvider, OllamaRequest};

    fn request() -> ChatRequest {
        ChatRequest {
            model: "llama3.2".to_owned(),
            system: Some("sys".to_owned()),
            messages: vec![helm_core::Message::user("hi")],
            tools: vec![ToolSchema {
                name: "shell".to_owned(),
                description: "run command".to_owned(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: 32,
            temperature: 0.0,
        }
    }

    #[test]
    fn request_maps_tools_happy_path() {
        let mapped = OllamaRequest::from_chat_request(&request()).unwrap();

        assert_eq!(mapped.tools.len(), 1);
        assert_eq!(mapped.messages[0].role, "system");
    }

    #[tokio::test]
    async fn request_body_with_tool_history_matches_ollama_wire_format() {
        let mut request = request();
        request.model = "qwen3:4b".to_owned();
        request.messages = vec![
            Message::user("list"),
            Message::assistant(vec![
                ContentBlock::Text("I will list files.".to_owned()),
                ContentBlock::ToolUse {
                    id: "toolu_shell".to_owned(),
                    name: "shell".to_owned(),
                    input: json!({"command": "ls"}),
                },
            ]),
            Message::tool_results(vec![ContentBlock::ToolResult {
                tool_use_id: "toolu_shell".to_owned(),
                content: "<result text>".to_owned(),
                is_error: false,
            }]),
        ];
        let expected = json!({
            "model": "qwen3:4b",
            "stream": false,
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "list"},
                {"role": "assistant", "content": "I will list files.", "tool_calls": [
                    {"function": {"name": "shell", "arguments": {"command": "ls"}}}
                ]},
                {"role": "tool", "content": "<result text>", "tool_name": "shell"}
            ],
            "tools": [
                {"type": "function", "function": {
                    "name": "shell",
                    "description": "run command",
                    "parameters": {"type": "object"}
                }}
            ],
            "options": {"temperature": 0.0}
        });
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(Matcher::Json(expected))
            .with_status(200)
            .with_body(
                json!({
                    "model": "qwen3:4b",
                    "message": {"role": "assistant", "content": "done"},
                    "done": true,
                    "done_reason": "stop"
                })
                .to_string(),
            )
            .create_async()
            .await;
        let provider = OllamaProvider::with_base_url(server.url()).unwrap();

        let response = provider.chat(request).await.unwrap();

        assert_eq!(response.stop_reason, StopReason::EndTurn);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn response_with_tool_calls_maps_to_tool_use() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body(
                json!({
                    "model": "llama3.2",
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "function": {
                                "name": "shell",
                                "arguments": { "command": "echo", "args": ["hi"] }
                            }
                        }]
                    },
                    "done": true,
                    "done_reason": "stop",
                    "prompt_eval_count": 5,
                    "eval_count": 2
                })
                .to_string(),
            )
            .create_async()
            .await;
        let provider = OllamaProvider::with_base_url(server.url()).unwrap();

        let response = provider.chat(request()).await.unwrap();

        match response.content.first() {
            Some(ContentBlock::ToolUse { id, name, input }) => {
                assert!(id.starts_with("ollama_tool_"));
                assert_eq!(name, "shell");
                assert_eq!(input, &json!({"command": "echo", "args": ["hi"]}));
            }
            other => panic!("expected tool use, got {other:?}"),
        }
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.usage.input_tokens, 5);
        assert_eq!(response.usage.output_tokens, 2);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn response_with_text_and_empty_tool_calls_is_end_turn() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body(
                json!({
                    "model": "llama3.2",
                    "message": {
                        "role": "assistant",
                        "content": "hello",
                        "tool_calls": []
                    },
                    "done": true,
                    "done_reason": "stop",
                    "prompt_eval_count": 7,
                    "eval_count": 4
                })
                .to_string(),
            )
            .create_async()
            .await;
        let provider = OllamaProvider::with_base_url(server.url()).unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(
            response.content,
            vec![ContentBlock::Text("hello".to_owned())]
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn done_reason_length_maps_to_max_tokens() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body(
                json!({
                    "model": "llama3.2",
                    "message": {"role": "assistant", "content": "cut off"},
                    "done": true,
                    "done_reason": "length",
                    "prompt_eval_count": 7,
                    "eval_count": 4
                })
                .to_string(),
            )
            .create_async()
            .await;
        let provider = OllamaProvider::with_base_url(server.url()).unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert_eq!(response.stop_reason, StopReason::MaxTokens);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn missing_token_counts_default_to_zero() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body(
                json!({
                    "model": "llama3.2",
                    "message": {"role": "assistant", "content": "done"},
                    "done": true,
                    "done_reason": "stop"
                })
                .to_string(),
            )
            .create_async()
            .await;
        let provider = OllamaProvider::with_base_url(server.url()).unwrap();

        let response = provider.chat(request()).await.unwrap();

        assert_eq!(response.usage.input_tokens, 0);
        assert_eq!(response.usage.output_tokens, 0);
        mock.assert_async().await;
    }

    #[test]
    fn mismatched_tool_use_id_surfaces_invalid_conversation() {
        let mut request = request();
        request.messages = vec![Message::tool_results(vec![ContentBlock::ToolResult {
            tool_use_id: "missing".to_owned(),
            content: "result".to_owned(),
            is_error: false,
        }])];

        let error = OllamaRequest::from_chat_request(&request).unwrap_err();

        assert!(matches!(error, ProviderError::InvalidConversation { .. }));
        assert!(error.to_string().contains("missing"));
    }

    #[tokio::test]
    async fn http_status_error_path() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(404)
            .with_body("missing")
            .create_async()
            .await;
        let provider = OllamaProvider::with_base_url(server.url()).unwrap();

        let error = provider.chat(request()).await.unwrap_err();

        assert!(error.to_string().contains("404"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn malformed_json_edge_case() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{bad")
            .create_async()
            .await;
        let provider = OllamaProvider::with_base_url(server.url()).unwrap();

        let error = provider.chat(request()).await.unwrap_err();

        assert!(error.to_string().contains("malformed"));
        mock.assert_async().await;
    }
}
