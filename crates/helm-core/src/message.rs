//! Anthropic-compatible chat message and content-block types.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Map, Value};

/// Role attached to a chat message in a provider conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System-level instruction role.
    System,
    /// User role, including tool results returned to the model.
    User,
    /// Assistant role for model-generated content.
    Assistant,
}

/// A single Anthropic Messages API content block.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    /// Plain text content.
    Text(String),
    /// Request from the model to execute a named tool with JSON input.
    ToolUse {
        /// Provider-generated tool-use identifier.
        id: String,
        /// Name of the requested HELM tool.
        name: String,
        /// JSON input object supplied by the provider.
        input: Value,
    },
    /// Result sent back to the provider for a previous tool-use request.
    ToolResult {
        /// Provider-generated tool-use identifier this result answers.
        tool_use_id: String,
        /// Human-readable tool output or error text.
        content: String,
        /// Whether the tool failed.
        is_error: bool,
    },
}

/// A single chat message with a role and ordered content blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Message role.
    pub role: Role,
    /// Ordered Anthropic content blocks.
    pub content: Vec<ContentBlock>,
}

impl Message {
    /// Creates a user message containing one text block.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text(text.into())],
        }
    }

    /// Creates an assistant message with the supplied content blocks.
    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            content,
        }
    }

    /// Creates a user message containing tool results.
    pub fn tool_results(content: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::User,
            content,
        }
    }
}

impl Serialize for ContentBlock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = Map::new();
        match self {
            Self::Text(text) => {
                map.insert("type".to_owned(), Value::String("text".to_owned()));
                map.insert("text".to_owned(), Value::String(text.clone()));
            }
            Self::ToolUse { id, name, input } => {
                map.insert("type".to_owned(), Value::String("tool_use".to_owned()));
                map.insert("id".to_owned(), Value::String(id.clone()));
                map.insert("name".to_owned(), Value::String(name.clone()));
                map.insert("input".to_owned(), input.clone());
            }
            Self::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                map.insert("type".to_owned(), Value::String("tool_result".to_owned()));
                map.insert("tool_use_id".to_owned(), Value::String(tool_use_id.clone()));
                map.insert("content".to_owned(), Value::String(content.clone()));
                if *is_error {
                    map.insert("is_error".to_owned(), Value::Bool(true));
                }
            }
        }
        Value::Object(map).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ContentBlock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut map = Map::<String, Value>::deserialize(deserializer)?;
        let block_type = take_string(&mut map, "type")?;
        match block_type.as_str() {
            "text" => Ok(Self::Text(take_string(&mut map, "text")?)),
            "tool_use" => Ok(Self::ToolUse {
                id: take_string(&mut map, "id")?,
                name: take_string(&mut map, "name")?,
                input: map.remove("input").unwrap_or(Value::Object(Map::new())),
            }),
            "tool_result" => Ok(Self::ToolResult {
                tool_use_id: take_string(&mut map, "tool_use_id")?,
                content: take_string(&mut map, "content")?,
                is_error: take_bool(&mut map, "is_error")?,
            }),
            other => Err(de::Error::custom(format!(
                "unknown content block type: {other}"
            ))),
        }
    }
}

fn take_string<E>(map: &mut Map<String, Value>, key: &str) -> Result<String, E>
where
    E: de::Error,
{
    match map.remove(key) {
        Some(Value::String(value)) => Ok(value),
        Some(_) => Err(E::custom(format!("{key} must be a string"))),
        None => Err(E::custom(format!("missing field: {key}"))),
    }
}

fn take_bool<E>(map: &mut Map<String, Value>, key: &str) -> Result<bool, E>
where
    E: de::Error,
{
    match map.remove(key) {
        Some(Value::Bool(value)) => Ok(value),
        Some(_) => Err(E::custom(format!("{key} must be a boolean"))),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::{ContentBlock, Message, Role};

    #[test]
    fn message_round_trips_anthropic_schema_happy_path() {
        let literal = json!({
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "I will check the weather."
                },
                {
                    "type": "tool_use",
                    "id": "toolu_01A09q90qw90lq917835lq9",
                    "name": "get_weather",
                    "input": { "location": "San Francisco, CA", "unit": "celsius" }
                }
            ]
        });

        let message: Message = serde_json::from_value(literal.clone()).unwrap();
        let encoded = serde_json::to_value(message).unwrap();

        assert_eq!(encoded, literal);
    }

    #[test]
    fn tool_result_without_is_error_defaults_false_error_path() {
        let block: ContentBlock = serde_json::from_value(json!({
            "type": "tool_result",
            "tool_use_id": "toolu_1",
            "content": "ok"
        }))
        .unwrap();

        assert_eq!(
            block,
            ContentBlock::ToolResult {
                tool_use_id: "toolu_1".to_owned(),
                content: "ok".to_owned(),
                is_error: false
            }
        );
    }

    #[test]
    fn unknown_block_type_is_rejected_edge_case() {
        let result = serde_json::from_value::<ContentBlock>(json!({ "type": "image" }));
        assert!(result.is_err());
    }

    #[test]
    fn constructors_create_expected_roles() {
        assert_eq!(Message::user("hi").role, Role::User);
        assert_eq!(Message::assistant(Vec::new()).role, Role::Assistant);
        assert_eq!(Message::tool_results(Vec::new()).role, Role::User);
    }
}
