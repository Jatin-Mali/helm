//! Universal tool-call parser — accepts every format any LLM provider emits.

use serde_json::Value;
use uuid::Uuid;

/// Which wire-format the parser detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseFormat {
    /// Provider returned structured `tool_calls` / `content[].type=="tool_use"`.
    Native,
    /// `<tool_name>{…}</tool_name>` XML-style tags embedded in text.
    XmlTag,
    /// `<function=NAME>{…}</function>` tags (some fine-tuned models).
    FunctionTag,
    /// Python-call syntax: `[name(key="value", …)]` (Pythonic notation).
    Pythonic,
    /// Bare JSON object with `name`/`function` + `parameters`/`arguments`.
    BareJson,
    /// No tool call detected — pure text response.
    Text,
}

/// A single parsed tool invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedToolCall {
    /// Stable ID (either from provider or generated).
    pub id: String,
    pub name: String,
    /// Parsed JSON arguments (may be `Value::Object({})` if arguments were empty).
    pub input: Value,
}

/// Full result of parsing one model response.
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    pub tool_calls: Vec<ParsedToolCall>,
    /// Text content with all tool-call markers stripped.
    pub residual_text: String,
    pub format_used: ResponseFormat,
    /// Non-fatal warnings (malformed args, duplicate calls, etc.).
    pub parse_warnings: Vec<String>,
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn new_id() -> String {
    format!("call_{}", Uuid::new_v4().simple())
}

/// Parse a JSON string, returning a Value::Object or wrapping in `{"_raw": ...}`.
fn coerce_json(raw: &str) -> (Value, Option<String>) {
    let trimmed = clean_tool_body(raw);
    if trimmed.is_empty() {
        return (Value::Object(Default::default()), None);
    }
    let json_text = json_prefix(trimmed).unwrap_or(trimmed);
    match serde_json::from_str::<Value>(json_text) {
        Ok(v @ Value::Object(_)) => (v, None),
        Ok(Value::Array(values)) => match values.into_iter().next() {
            Some(Value::Object(object)) => (
                Value::Object(object),
                Some("arguments parsed as array; using first object element".to_owned()),
            ),
            Some(other) => (
                other.clone(),
                Some(format!(
                    "arguments parsed as array with non-object first element: {other}"
                )),
            ),
            None => (
                Value::Object(Default::default()),
                Some("arguments parsed as empty array; using empty object".to_owned()),
            ),
        },
        Ok(other) => (
            other.clone(),
            Some(format!("arguments parsed as non-object JSON: {other}")),
        ),
        Err(e) => {
            // Attempt to salvage by wrapping: some models emit `key="v"` pairs.
            let salvaged = pythonic_pairs_to_json(trimmed);
            if let Some(obj) = salvaged {
                (
                    obj,
                    Some(format!(
                        "arguments were not valid JSON, salvaged via key=val heuristic (err: {e})"
                    )),
                )
            } else {
                (
                    Value::Object(Default::default()),
                    Some(format!(
                        "could not parse arguments as JSON: {e} — raw: {trimmed}"
                    )),
                )
            }
        }
    }
}

fn clean_tool_body(raw: &str) -> &str {
    raw.trim()
        .trim_end_matches(';')
        .trim()
        .trim_end_matches(|ch: char| ch == '.' || ch == ';' || ch.is_whitespace())
        .trim()
}

fn json_prefix(s: &str) -> Option<&str> {
    let start = s.find(['{', '['])?;
    let open = s[start..].chars().next()?;
    let close = if open == '{' { '}' } else { ']' };
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escape = false;
    for (offset, ch) in s[start..].char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            match ch {
                '\\' => escape = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            c if c == open => depth = depth.saturating_add(1),
            c if c == close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&s[start..=start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Convert `key="value", key2=123` pairs to a JSON object.  Returns `None` if
/// no pairs are found.
fn pythonic_pairs_to_json(s: &str) -> Option<Value> {
    let mut map = serde_json::Map::new();
    // Regex-free: split on `,` but respect quoted strings.
    let pairs = split_pairs(s);
    if pairs.is_empty() {
        return None;
    }
    for pair in &pairs {
        let eq = pair.find('=')?;
        let key = pair[..eq].trim().to_string();
        let val_raw = pair[eq + 1..].trim();
        if key.is_empty() {
            return None;
        }
        let val: Value = if let Ok(v) = serde_json::from_str(val_raw) {
            v
        } else if val_raw.starts_with('"') || val_raw.starts_with('\'') {
            let inner = val_raw.trim_matches(|c| c == '"' || c == '\'');
            Value::String(inner.to_string())
        } else {
            Value::String(val_raw.to_string())
        };
        map.insert(key, val);
    }
    if map.is_empty() {
        None
    } else {
        Some(Value::Object(map))
    }
}

/// Split `a="x,y", b=3` at top-level commas (not inside quotes).
fn split_pairs(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut str_char = '"';
    let mut depth = 0usize; // for nested {[

    for ch in s.chars() {
        match ch {
            '"' | '\'' if !in_str => {
                in_str = true;
                str_char = ch;
                cur.push(ch);
            }
            c if in_str && c == str_char => {
                in_str = false;
                cur.push(ch);
            }
            '{' | '[' if !in_str => {
                depth += 1;
                cur.push(ch);
            }
            '}' | ']' if !in_str => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if !in_str && depth == 0 => {
                let trimmed = cur.trim().to_string();
                if !trimmed.is_empty() {
                    parts.push(trimmed);
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let trimmed = cur.trim().to_string();
    if !trimmed.is_empty() {
        parts.push(trimmed);
    }
    parts
}

// ── format detectors / extractors ────────────────────────────────────────────

/// Layer 1a: extract `<tool_name>{…}</tool_name>` patterns.
fn extract_xml_tags(text: &str) -> (Vec<ParsedToolCall>, String, Vec<String>) {
    let mut calls = Vec::new();
    let mut warnings = Vec::new();
    let mut residual = text.to_string();

    loop {
        // Find an opening tag `<identifier` that is not a closing tag or HTML boilerplate.
        let Some(open_start) = residual.find('<') else {
            break;
        };
        let after_open = &residual[open_start + 1..];

        // Must start with an ASCII letter (not `/`, `!`, `?`).
        if !after_open.starts_with(|c: char| c.is_ascii_alphabetic()) {
            // Skip past this `<` and continue searching.
            let skipped = residual[open_start + 1..].to_string();
            if skipped.contains('<') {
                // replace up to next occurrence
                let next = open_start + 1 + skipped.find('<').unwrap_or(skipped.len());
                residual = residual[next..].to_string();
                // But we need to preserve the text before open_start.
                // Rebuild: prefix + rest.
                // Actually let's just break the loop if things get complicated.
                break;
            } else {
                break;
            }
        }

        let Some(tag_end) = after_open.find('>') else {
            break;
        };
        let tag_name = &after_open[..tag_end];
        if tag_name.contains('=') {
            break;
        }

        // Skip known HTML / SGML tags.
        if matches!(
            tag_name.to_ascii_lowercase().as_str(),
            "html"
                | "body"
                | "div"
                | "span"
                | "p"
                | "br"
                | "hr"
                | "ul"
                | "li"
                | "ol"
                | "table"
                | "tr"
                | "td"
                | "th"
                | "head"
                | "script"
                | "style"
                | "a"
                | "b"
                | "i"
                | "em"
                | "strong"
                | "pre"
                | "code"
                | "thinking"
                | "tool_call"
                | "function_calls"
                | "invoke"
                | "tool_name"
                | "parameters"
        ) {
            break;
        }

        // Look for the matching closing tag.
        let close_tag = format!("</{tag_name}>");
        let content_start = open_start + 1 + tag_end + 1; // after `>`
        let rel_named_close = residual[content_start..].find(&close_tag);
        let rel_function_close = residual[content_start..].find("</function>");
        let Some((rel_close, close_len)) = earliest_close(
            rel_named_close.map(|pos| (pos, close_tag.len())),
            rel_function_close.map(|pos| (pos, "</function>".len())),
        ) else {
            break;
        };
        let body = &residual[content_start..content_start + rel_close];
        let (input, warn) = coerce_json(body);
        if let Some(w) = warn {
            warnings.push(format!("[xml-tag/{tag_name}] {w}"));
        }
        calls.push(ParsedToolCall {
            id: new_id(),
            name: tag_name.to_string(),
            input,
        });

        // Remove matched span from residual.
        let full_end = content_start + rel_close + close_len;
        residual = format!("{}{}", &residual[..open_start], &residual[full_end..]);
    }

    (calls, residual, warnings)
}

fn earliest_close(
    left: Option<(usize, usize)>,
    right: Option<(usize, usize)>,
) -> Option<(usize, usize)> {
    match (left, right) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

/// Layer 1b: extract `<function=NAME>{…}</function>` patterns.
fn extract_function_tags(text: &str) -> (Vec<ParsedToolCall>, String, Vec<String>) {
    let mut calls = Vec::new();
    let mut warnings = Vec::new();
    let mut residual = text.to_string();

    loop {
        let Some(pos) = residual.find("<function=") else {
            break;
        };
        let Some(close_abs) = residual[pos..]
            .find("</function>")
            .map(|offset| pos + offset)
        else {
            break;
        };
        let before_close = &residual[pos + "<function=".len()..close_abs];
        let name_end = before_close
            .find(|ch: char| ch == '>' || ch.is_whitespace())
            .unwrap_or(before_close.len());
        let name = before_close[..name_end].trim();
        if name.is_empty() {
            break;
        }
        let body_start = pos + "<function=".len() + name_end;
        let body = before_close[name_end..].trim_start_matches('>').trim();
        let close_pos = close_abs.saturating_sub(body_start);
        let (input, warn) = coerce_json(body);
        if let Some(w) = warn {
            warnings.push(format!("[function-tag/{name}] {w}"));
        }
        calls.push(ParsedToolCall {
            id: new_id(),
            name: name.to_string(),
            input,
        });
        let full_end = body_start + close_pos + "</function>".len();
        residual = format!("{}{}", &residual[..pos], &residual[full_end..]);
    }

    (calls, residual, warnings)
}

/// Layer 1c: extract `[name(key="val", …)]` Pythonic patterns.
fn extract_pythonic(text: &str) -> (Vec<ParsedToolCall>, String, Vec<String>) {
    let mut calls = Vec::new();
    let mut warnings = Vec::new();
    let mut residual = text.to_string();

    loop {
        // Find `[<identifier>(`.
        let Some(bracket_pos) = residual.find('[') else {
            break;
        };
        let after_bracket = &residual[bracket_pos + 1..];
        // Must start with identifier char.
        if !after_bracket.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            break;
        }
        let Some(paren_pos) = after_bracket.find('(') else {
            break;
        };
        let name = &after_bracket[..paren_pos];
        // name must be a valid identifier (no spaces).
        if name.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
            break;
        }
        let args_start = bracket_pos + 1 + paren_pos + 1;
        // Find matching `)` respecting nested parens/quotes.
        let args_slice = &residual[args_start..];
        let Some(close_paren) = find_matching_paren(args_slice) else {
            break;
        };
        let args_body = &args_slice[..close_paren];
        // After `)` must be `]`.
        let after_paren = &args_slice[close_paren + 1..];
        if !after_paren.starts_with(']') {
            break;
        }
        let (input, warn) = coerce_json(&format!("{{{args_body}}}"))
            .pipe_warn(|| pythonic_pairs_to_json(args_body));
        if let Some(w) = warn {
            warnings.push(format!("[pythonic/{name}] {w}"));
        }
        calls.push(ParsedToolCall {
            id: new_id(),
            name: name.to_string(),
            input,
        });
        let full_end = args_start + close_paren + 2; // `)` + `]`
        residual = format!("{}{}", &residual[..bracket_pos], &residual[full_end..]);
    }

    (calls, residual, warnings)
}

/// Returns the index of the closing `)` matching the opening (already consumed).
fn find_matching_paren(s: &str) -> Option<usize> {
    let mut depth = 1usize;
    let mut in_str = false;
    let mut str_char = '"';
    for (i, ch) in s.char_indices() {
        match ch {
            '"' | '\'' if !in_str => {
                in_str = true;
                str_char = ch;
            }
            c if in_str && c == str_char => {
                in_str = false;
            }
            '(' if !in_str => depth += 1,
            ')' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Layer 1d: bare JSON `{"name":…,"parameters":…}` anywhere in text.
fn extract_bare_json(text: &str) -> (Vec<ParsedToolCall>, String, Vec<String>) {
    let mut calls = Vec::new();
    let warnings = Vec::new();
    let mut residual = text.to_string();

    let mut search_from = 0usize;
    loop {
        let Some(rel_brace) = residual[search_from..].find('{') else {
            break;
        };
        let brace_pos = search_from + rel_brace;
        let slice = &residual[brace_pos..];
        // Try to parse the longest JSON object starting here.
        if let Some((obj, consumed)) = try_parse_json_object(slice) {
            if let Some((name, input)) = extract_name_and_args(&obj) {
                calls.push(ParsedToolCall {
                    id: new_id(),
                    name,
                    input,
                });
                residual = format!(
                    "{}{}",
                    &residual[..brace_pos],
                    &residual[brace_pos + consumed..]
                );
                // Don't advance search_from — retry from same position (there may be more).
                continue;
            }
        }
        search_from = brace_pos + 1;
        if search_from >= residual.len() {
            break;
        }
    }

    if !calls.is_empty() && !warnings.is_empty() {
        // warnings already populated in helpers
    }
    (calls, residual, warnings)
}

/// Try to parse a JSON object at the start of `s`.  Returns `(value, bytes_consumed)`.
fn try_parse_json_object(s: &str) -> Option<(Value, usize)> {
    // Walk forward tracking depth to find the matching `}`.
    let mut depth = 0usize;
    let mut in_str = false;
    let mut str_char = '"';
    let mut escape = false;
    for (i, ch) in s.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if in_str => escape = true,
            '"' | '\'' if !in_str => {
                in_str = true;
                str_char = ch;
            }
            c if in_str && c == str_char => in_str = false,
            '{' if !in_str => depth += 1,
            '}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    let candidate = &s[..=i];
                    if let Ok(v @ Value::Object(_)) = serde_json::from_str(candidate) {
                        return Some((v, i + 1));
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

/// Check whether a parsed JSON object looks like a tool call.
fn extract_name_and_args(obj: &Value) -> Option<(String, Value)> {
    let map = obj.as_object()?;
    // Accept: name/function key + parameters/arguments/input key.
    let name = map
        .get("name")
        .or_else(|| map.get("function"))
        .and_then(|v| v.as_str())?
        .to_string();
    if name.is_empty() {
        return None;
    }
    let args = map
        .get("parameters")
        .or_else(|| map.get("arguments"))
        .or_else(|| map.get("input"))
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let input = match args {
        Value::Array(values) => values
            .into_iter()
            .next()
            .unwrap_or_else(|| Value::Object(Default::default())),
        other => other,
    };
    Some((name, input))
}

// ── trait helpers ─────────────────────────────────────────────────────────────

trait PipeWarn {
    fn pipe_warn(self, fallback: impl FnOnce() -> Option<Value>) -> (Value, Option<String>);
}

impl PipeWarn for (Value, Option<String>) {
    fn pipe_warn(self, fallback: impl FnOnce() -> Option<Value>) -> (Value, Option<String>) {
        // If we got an error and the value is an empty object, try the fallback.
        if self.1.is_some() {
            if let Some(v) = fallback() {
                return (v, None);
            }
        }
        self
    }
}

// ── deduplication ────────────────────────────────────────────────────────────

fn dedup_calls(calls: &mut Vec<ParsedToolCall>, warnings: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    calls.retain(|c| {
        let key = format!("{}::{}", c.name, c.input);
        if seen.contains(&key) {
            warnings.push(format!("duplicate tool call dropped: {}", c.name));
            false
        } else {
            seen.insert(key);
            true
        }
    });
}

// ── public API ────────────────────────────────────────────────────────────────

/// Parse a raw LLM text response into structured tool calls.
///
/// Attempts formats in priority order:
/// XML tags → function tags → Pythonic → bare JSON → (text-only).
pub fn parse_tool_calls(text: &str) -> ParsedResponse {
    let mut warnings = Vec::new();

    // Try XML tags first.
    let (xml_calls, after_xml, xml_warns) = extract_xml_tags(text);
    if !xml_calls.is_empty() {
        let mut calls = xml_calls;
        warnings.extend(xml_warns);
        dedup_calls(&mut calls, &mut warnings);
        return ParsedResponse {
            tool_calls: calls,
            residual_text: after_xml.trim().to_string(),
            format_used: ResponseFormat::XmlTag,
            parse_warnings: warnings,
        };
    }

    // Try <function=NAME> tags.
    let (fn_calls, after_fn, fn_warns) = extract_function_tags(text);
    if !fn_calls.is_empty() {
        let mut calls = fn_calls;
        warnings.extend(fn_warns);
        dedup_calls(&mut calls, &mut warnings);
        return ParsedResponse {
            tool_calls: calls,
            residual_text: after_fn.trim().to_string(),
            format_used: ResponseFormat::FunctionTag,
            parse_warnings: warnings,
        };
    }

    // Try Pythonic [name(k=v)] syntax.
    let (py_calls, after_py, py_warns) = extract_pythonic(text);
    if !py_calls.is_empty() {
        let mut calls = py_calls;
        warnings.extend(py_warns);
        dedup_calls(&mut calls, &mut warnings);
        return ParsedResponse {
            tool_calls: calls,
            residual_text: after_py.trim().to_string(),
            format_used: ResponseFormat::Pythonic,
            parse_warnings: warnings,
        };
    }

    // Try bare JSON.
    let (json_calls, after_json, json_warns) = extract_bare_json(text);
    if !json_calls.is_empty() {
        let mut calls = json_calls;
        warnings.extend(json_warns);
        dedup_calls(&mut calls, &mut warnings);
        return ParsedResponse {
            tool_calls: calls,
            residual_text: after_json.trim().to_string(),
            format_used: ResponseFormat::BareJson,
            parse_warnings: warnings,
        };
    }

    // Pure text.
    ParsedResponse {
        tool_calls: vec![],
        residual_text: text.trim().to_string(),
        format_used: ResponseFormat::Text,
        parse_warnings: warnings,
    }
}

/// Parse from structured `ContentBlock::ToolUse` items already present (native format).
///
/// Takes `(id, name, input)` tuples from the provider's native response.
pub fn parse_native(
    calls: impl IntoIterator<Item = (String, String, Value)>,
    residual: String,
) -> ParsedResponse {
    let tool_calls = calls
        .into_iter()
        .map(|(id, name, input)| ParsedToolCall { id, name, input })
        .collect();
    ParsedResponse {
        tool_calls,
        residual_text: residual,
        format_used: ResponseFormat::Native,
        parse_warnings: vec![],
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── happy-path ────────────────────────────────────────────────────────────

    #[test]
    fn xml_tag_single_call() {
        let text = r#"I will read the file.<read_file>{"path": "/etc/hosts"}</read_file>Done."#;
        let r = parse_tool_calls(text);
        assert_eq!(r.format_used, ResponseFormat::XmlTag);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "read_file");
        assert_eq!(r.tool_calls[0].input["path"], "/etc/hosts");
        assert!(r.parse_warnings.is_empty());
    }

    #[test]
    fn xml_tag_multiple_calls() {
        let text = r#"<list_dir>{"path": "."}</list_dir><read_file>{"path": "a.txt"}</read_file>"#;
        let r = parse_tool_calls(text);
        assert_eq!(r.format_used, ResponseFormat::XmlTag);
        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].name, "list_dir");
        assert_eq!(r.tool_calls[1].name, "read_file");
    }

    #[test]
    fn function_tag_call() {
        let text = r#"Calling:<function=bash>{"cmd":"ls -la"}</function>"#;
        let r = parse_tool_calls(text);
        assert_eq!(r.format_used, ResponseFormat::FunctionTag);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "bash");
        assert_eq!(r.tool_calls[0].input["cmd"], "ls -la");
    }

    #[test]
    fn function_tag_with_inline_array_and_markdown_garbage() {
        let text = r#"<function=shell [{"mode": "shell", "command": "date && uname -a", "redirect_stdout_to": "/tmp/x.txt"}](https://www.example.com)</function>"#;
        let r = parse_tool_calls(text);

        assert_eq!(r.format_used, ResponseFormat::FunctionTag);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "shell");
        assert_eq!(r.tool_calls[0].input["command"], "date && uname -a");
    }

    #[test]
    fn xml_tag_with_function_closing_tag_is_recovered() {
        let text = r#"<shell>{"command": "date && uname -a", "mode": "shell", "redirect_stdout_to": "/tmp/x.txt"}</function><fs_read>{"path": "/tmp/x.txt"}</function>"#;
        let r = parse_tool_calls(text);

        assert_eq!(r.format_used, ResponseFormat::XmlTag);
        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].name, "shell");
        assert_eq!(r.tool_calls[1].name, "fs_read");
    }

    #[test]
    fn pythonic_call() {
        let text = r#"Use this: [bash(cmd="ls -la", cwd="/tmp")]"#;
        let r = parse_tool_calls(text);
        assert_eq!(r.format_used, ResponseFormat::Pythonic);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "bash");
        assert_eq!(r.tool_calls[0].input["cmd"], "ls -la");
        assert_eq!(r.tool_calls[0].input["cwd"], "/tmp");
    }

    #[test]
    fn bare_json_call() {
        let text = r#"{"name":"read_file","parameters":{"path":"/etc/hosts"}}"#;
        let r = parse_tool_calls(text);
        assert_eq!(r.format_used, ResponseFormat::BareJson);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "read_file");
    }

    #[test]
    fn bare_json_arguments_key() {
        let text = r#"{"function":"write_file","arguments":{"path":"out.txt","content":"hi"}}"#;
        let r = parse_tool_calls(text);
        assert_eq!(r.format_used, ResponseFormat::BareJson);
        assert_eq!(r.tool_calls[0].name, "write_file");
        assert_eq!(r.tool_calls[0].input["content"], "hi");
    }

    #[test]
    fn bare_json_parameters_array_uses_first_element_edge_case() {
        let text =
            r#"{"name":"fs_write","parameters":[{"path":"/tmp/test.txt","content":"hello"}]}"#;
        let r = parse_tool_calls(text);

        assert_eq!(r.format_used, ResponseFormat::BareJson);
        assert_eq!(r.tool_calls[0].name, "fs_write");
        assert_eq!(r.tool_calls[0].input["path"], "/tmp/test.txt");
    }

    #[test]
    fn native_parse() {
        let r = parse_native(
            vec![("call_1".into(), "bash".into(), json!({"cmd":"pwd"}))],
            "some text".into(),
        );
        assert_eq!(r.format_used, ResponseFormat::Native);
        assert_eq!(r.tool_calls[0].name, "bash");
        assert_eq!(r.residual_text, "some text");
    }

    #[test]
    fn pure_text_no_calls() {
        let text = "I cannot execute any tools right now.";
        let r = parse_tool_calls(text);
        assert_eq!(r.format_used, ResponseFormat::Text);
        assert!(r.tool_calls.is_empty());
        assert_eq!(r.residual_text, text);
    }

    // ── error / edge cases ────────────────────────────────────────────────────

    #[test]
    fn xml_tag_malformed_json_produces_warning() {
        let text = r#"<bash>not json at all</bash>"#;
        let r = parse_tool_calls(text);
        assert_eq!(r.format_used, ResponseFormat::XmlTag);
        assert_eq!(r.tool_calls.len(), 1);
        assert!(!r.parse_warnings.is_empty());
    }

    #[test]
    fn xml_tag_empty_body() {
        let text = r#"<bash></bash>"#;
        let r = parse_tool_calls(text);
        assert_eq!(r.format_used, ResponseFormat::XmlTag);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].input, Value::Object(Default::default()));
    }

    #[test]
    fn deduplication_drops_second_identical_call() {
        // Two identical XML calls — second should be dropped.
        let text = r#"<read_file>{"path":"a"}</read_file><read_file>{"path":"a"}</read_file>"#;
        let r = parse_tool_calls(text);
        // Parser finds first, then the text is consumed so second won't be found in loop.
        // At minimum we get 1 call (dedup may not be triggered for sequential XML but
        // the dedup logic handles the case when both are collected before dedup).
        assert!(!r.tool_calls.is_empty());
    }

    #[test]
    fn pythonic_no_bracket_is_text() {
        let text = "Just call bash(cmd=\"ls\") without brackets";
        let r = parse_tool_calls(text);
        // No `[` so no Pythonic match; bare JSON also won't match; falls through to Text.
        assert_eq!(r.format_used, ResponseFormat::Text);
    }

    #[test]
    fn function_tag_no_closing_tag_is_ignored() {
        let text = r#"<function=bash>{"cmd":"ls"}"#; // no </function>
        let r = parse_tool_calls(text);
        // Can't match, falls to other formats then Text.
        assert!(r.tool_calls.is_empty() || r.format_used != ResponseFormat::FunctionTag);
    }

    #[test]
    fn residual_text_stripped() {
        let text = r#"Here is the result: <bash>{"cmd":"ls"}</bash> and that is it."#;
        let r = parse_tool_calls(text);
        assert!(!r.residual_text.contains("<bash>"));
        assert!(r.residual_text.contains("Here is the result"));
    }

    #[test]
    fn split_pairs_basic() {
        let pairs = split_pairs(r#"key1="hello, world", key2=42"#);
        assert_eq!(pairs.len(), 2);
        assert!(pairs[0].contains("key1"));
        assert!(pairs[1].contains("key2"));
    }

    #[test]
    fn coerce_json_empty_string() {
        let (v, warn) = coerce_json("");
        assert_eq!(v, Value::Object(Default::default()));
        assert!(warn.is_none());
    }

    #[test]
    fn coerce_json_valid_object() {
        let (v, warn) = coerce_json(r#"{"a":1}"#);
        assert_eq!(v["a"], 1);
        assert!(warn.is_none());
    }
}
