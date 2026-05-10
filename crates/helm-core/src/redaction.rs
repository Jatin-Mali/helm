//! Best-effort redaction helpers for local storage and debug logging.

use crate::ContentBlock;

const REDACTED: &str = "[REDACTED]";
const REDACTED_PATH: &str = "[REDACTED_PATH]";

const SECRET_PREFIXES: &[&str] = &[
    "sk-or-",
    "sk-proj-",
    "sk-ant-",
    "sk-",
    "gsk_",
];

const SECRET_PATH_SUFFIXES: &[&str] = &[
    "/.helm/secrets.toml",
    "/.helm/.secrets.toml.lock",
    "/.helm/helm.db",
    "/.helm/helm.log",
];

pub fn redact_text(text: &str) -> String {
    let mut redacted = redact_secret_paths(text);
    for prefix in SECRET_PREFIXES {
        redacted = redact_prefixed_secret(&redacted, prefix);
    }
    redacted
}

pub fn redact_content_blocks(blocks: &[ContentBlock]) -> Vec<ContentBlock> {
    blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text(text) => ContentBlock::Text(redact_text(text)),
            ContentBlock::ToolUse { id, name, input } => ContentBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: redact_json_value(input),
            },
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => ContentBlock::ToolResult {
                tool_use_id: tool_use_id.clone(),
                content: redact_text(content),
                is_error: *is_error,
            },
        })
        .collect()
}

fn redact_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::Value::String(redact_text(text)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_json_value).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), redact_json_value(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn redact_secret_paths(text: &str) -> String {
    let mut out = text.to_owned();
    for suffix in SECRET_PATH_SUFFIXES {
        while let Some(idx) = out.find(suffix) {
            let start = path_start(&out, idx);
            let end = idx + suffix.len();
            out.replace_range(start..end, REDACTED_PATH);
        }
    }
    out
}

fn path_start(text: &str, suffix_start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut start = suffix_start;
    while start > 0 {
        let prev = bytes[start - 1];
        if prev.is_ascii_whitespace() || matches!(prev, b'"' | b'\'' | b'(' | b'[' | b'{') {
            break;
        }
        start -= 1;
    }
    start
}

fn redact_prefixed_secret(text: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = rest.find(prefix) {
        let (before, after_prefix) = rest.split_at(idx);
        if prefix == "sk-"
            && (after_prefix.starts_with("sk-or-")
                || after_prefix.starts_with("sk-proj-")
                || after_prefix.starts_with("sk-ant-"))
        {
            out.push_str(before);
            out.push_str("sk-");
            rest = &after_prefix["sk-".len()..];
            continue;
        }
        out.push_str(before);
        out.push_str(prefix);
        out.push_str(REDACTED);
        let secret = &after_prefix[prefix.len()..];
        let secret_len = secret
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
            .count();
        rest = &secret[secret
            .char_indices()
            .nth(secret_len)
            .map(|(i, _)| i)
            .unwrap_or(secret.len())..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{redact_content_blocks, redact_text};
    use crate::ContentBlock;

    #[test]
    fn redacts_openrouter_style_keys() {
        let text = "token sk-or-v1-abc123XYZ";
        let redacted = redact_text(text);
        assert!(redacted.contains("sk-or-[REDACTED]"));
        assert!(!redacted.contains("abc123XYZ"));
    }

    #[test]
    fn redacts_secret_store_paths() {
        let text = "cat /home/rick/.helm/secrets.toml now";
        let redacted = redact_text(text);
        assert!(redacted.contains("[REDACTED_PATH]"));
        assert!(!redacted.contains(".helm/secrets.toml"));
    }

    #[test]
    fn redacts_tool_result_blocks() {
        let blocks = vec![ContentBlock::ToolResult {
            tool_use_id: "toolu_1".to_owned(),
            content: "OPENROUTER_API_KEY=sk-or-test123".to_owned(),
            is_error: false,
        }];
        let redacted = redact_content_blocks(&blocks);
        let ContentBlock::ToolResult { content, .. } = &redacted[0] else {
            panic!("expected tool result");
        };
        assert!(!content.contains("test123"));
    }

    #[test]
    fn redacts_nested_json_strings() {
        let block = ContentBlock::ToolUse {
            id: "toolu_1".to_owned(),
            name: "shell".to_owned(),
            input: json!({"command": "printf", "env": {"OPENROUTER_API_KEY": "sk-or-test123"}}),
        };
        let redacted = redact_content_blocks(&[block]);
        let ContentBlock::ToolUse { input, .. } = &redacted[0] else {
            panic!("expected tool use");
        };
        assert!(!input.to_string().contains("test123"));
    }
}
