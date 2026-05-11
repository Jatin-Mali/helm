//! MCP (Model Context Protocol) bridge tool for HELM.
//!
//! Connects to stdio-based MCP servers configured in ~/.helm/mcp-servers.toml
//! and proxies tool calls through the JSON-RPC 2.0 protocol.

use std::{path::PathBuf, process::Stdio, time::Duration};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    time::timeout,
};

use crate::{
    command::build_command_in_dir,
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

fn xdg_config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("helm")
}

// ── Config ──────────────────────────────────────────────────────────────────

/// Deserialization root for `~/.helm/mcp-servers.toml`.
#[derive(Debug, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// One MCP server entry in the config file.
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<McpEnvEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpEnvEntry {
    pub key: String,
    pub value: String,
}

/// Returns the default path for the MCP config file.
pub fn default_mcp_config_path() -> Option<PathBuf> {
    Some(xdg_config_dir().join("mcp-servers.toml"))
}

/// Loads the MCP config file.  Returns an empty config if the file does not exist.
pub fn load_mcp_config() -> Result<McpConfig, ToolError> {
    let path = default_mcp_config_path()
        .ok_or_else(|| ToolError::Other("could not determine HOME directory".into()))?;

    if !path.exists() {
        return Ok(McpConfig::default());
    }

    let raw = std::fs::read_to_string(&path)
        .map_err(|e| ToolError::Other(format!("failed to read {}: {e}", path.display())))?;

    toml::from_str(&raw).map_err(|e| ToolError::Other(format!("malformed mcp-servers.toml: {e}")))
}

fn find_server<'a>(config: &'a McpConfig, name: &str) -> Result<&'a McpServerConfig, ToolError> {
    config.servers.iter().find(|s| s.name == name).ok_or_else(|| {
        let names: Vec<&str> = config.servers.iter().map(|s| s.name.as_str()).collect();
        if names.is_empty() {
            ToolError::Other(format!(
                "no MCP server named '{name}' — ~/.helm/mcp-servers.toml has no servers configured"
            ))
        } else {
            ToolError::Other(format!(
                "no MCP server named '{name}' — configured servers: {}",
                names.join(", ")
            ))
        }
    })
}

// ── Tool ─────────────────────────────────────────────────────────────────────

/// Bridges agent tool calls to external MCP servers over the stdio transport.
#[derive(Debug, Default)]
pub struct McpTool;

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &'static str {
        "mcp"
    }

    fn description(&self) -> &'static str {
        "Bridge to an external MCP (Model Context Protocol) server configured in ~/.helm/mcp-servers.toml. Supports listing tools and calling tools on any configured server."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_tools", "call"],
                    "description": "list_tools: enumerate available tools on the server. call: invoke a named tool."
                },
                "server": {
                    "type": "string",
                    "description": "Server name matching an entry in ~/.helm/mcp-servers.toml."
                },
                "tool": {
                    "type": "string",
                    "description": "Tool name to call (required for action=call)."
                },
                "arguments": {
                    "type": "object",
                    "description": "JSON arguments to pass to the tool (action=call only).",
                    "additionalProperties": true
                }
            },
            "required": ["action", "server"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = input["action"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidInput("action is required".into()))?;
        let server_name = input["server"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidInput("server is required".into()))?;

        let config = load_mcp_config()?;
        let server = find_server(&config, server_name)?;
        let mut client = McpClient::connect(server, ctx).await?;
        client.initialize().await?;

        match action {
            "list_tools" => {
                let tools = client.list_tools().await?;
                let content = format_tools_list(&tools);
                Ok(ToolOutput {
                    content,
                    success: true,
                    metadata: Map::new(),
                })
            }
            "call" => {
                let tool_name = input["tool"].as_str().ok_or_else(|| {
                    ToolError::InvalidInput("tool is required for action=call".into())
                })?;
                let args = match &input["arguments"] {
                    Value::Null | Value::Object(_) => {
                        if input["arguments"].is_null() {
                            json!({})
                        } else {
                            input["arguments"].clone()
                        }
                    }
                    _ => {
                        return Err(ToolError::InvalidInput(
                            "arguments must be a JSON object".into(),
                        ));
                    }
                };
                let result = client.call_tool(tool_name, args).await?;
                let content = format_call_result(&result);
                Ok(ToolOutput {
                    content,
                    success: true,
                    metadata: Map::new(),
                })
            }
            _ => Err(ToolError::InvalidInput(format!("unknown action: {action}"))),
        }
    }
}

// ── MCP Client ───────────────────────────────────────────────────────────────

struct McpClient {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
    child: Child,
}

impl McpClient {
    async fn connect(server: &McpServerConfig, ctx: &ToolContext) -> Result<Self, ToolError> {
        let mut cmd = build_command_in_dir(&server.command, &server.args, ctx, &ctx.working_dir)?;
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        for entry in &server.env {
            cmd.env(&entry.key, &entry.value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            ToolError::Other(format!(
                "failed to start MCP server '{}' ({}): {e}",
                server.name, server.command
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ToolError::Other("could not open MCP server stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::Other("could not open MCP server stdout".into()))?;

        Ok(Self {
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
            child,
        })
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        let id = self.next_id;
        self.next_id += 1;

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.send_message(&request).await?;

        // Read messages until we find one matching our request id.
        loop {
            let msg = self.read_message().await?;
            match msg.get("id") {
                Some(Value::Number(n)) if n.as_u64() == Some(id) => {
                    if let Some(err) = msg.get("error") {
                        return Err(ToolError::Other(format!("MCP server error: {err}")));
                    }
                    return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                }
                // Skip server-initiated notifications (no id or different id).
                _ => continue,
            }
        }
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> Result<(), ToolError> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.send_message(&notification).await
    }

    async fn send_message(&mut self, msg: &Value) -> Result<(), ToolError> {
        let mut line = serde_json::to_string(msg)
            .map_err(|e| ToolError::Other(format!("failed to serialize MCP message: {e}")))?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| ToolError::Other(format!("failed to write to MCP server: {e}")))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| ToolError::Other(format!("failed to flush MCP stdin: {e}")))
    }

    async fn read_message(&mut self) -> Result<Value, ToolError> {
        let mut line = String::new();
        timeout(Duration::from_secs(15), self.reader.read_line(&mut line))
            .await
            .map_err(|_| ToolError::Other("MCP server did not respond within 15s".into()))?
            .map_err(|e| ToolError::Other(format!("failed to read from MCP server: {e}")))?;

        if line.is_empty() {
            return Err(ToolError::Other(
                "MCP server closed connection unexpectedly".into(),
            ));
        }

        serde_json::from_str(line.trim())
            .map_err(|e| ToolError::Other(format!("MCP server sent invalid JSON: {e}")))
    }

    async fn initialize(&mut self) -> Result<(), ToolError> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "helm",
                "version": "1.1.0"
            }
        });
        self.send_request("initialize", params).await?;
        self.send_notification("notifications/initialized", json!({}))
            .await
    }

    async fn list_tools(&mut self) -> Result<Vec<Value>, ToolError> {
        let result = self.send_request("tools/list", json!({})).await?;
        Ok(result["tools"].as_array().cloned().unwrap_or_default())
    }

    async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        let params = json!({ "name": name, "arguments": arguments });
        self.send_request("tools/call", params).await
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Best-effort kill; don't block in Drop.
        let _ = self.child.start_kill();
    }
}

// ── Formatters ───────────────────────────────────────────────────────────────

fn format_tools_list(tools: &[Value]) -> String {
    if tools.is_empty() {
        return "No tools available on this server.".to_owned();
    }
    let mut out = String::new();
    for tool in tools {
        let name = tool["name"].as_str().unwrap_or("<unnamed>");
        let desc = tool["description"].as_str().unwrap_or("");
        out.push_str(&format!("- {name}: {desc}\n"));
    }
    out.trim_end().to_owned()
}

fn format_call_result(result: &Value) -> String {
    // MCP tool results contain a `content` array of content blocks.
    if let Some(contents) = result["content"].as_array() {
        let parts: Vec<String> = contents
            .iter()
            .filter_map(|block| {
                if block["type"].as_str() == Some("text") {
                    block["text"].as_str().map(String::from)
                } else {
                    Some(serde_json::to_string(block).unwrap_or_default())
                }
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    // Fallback: dump the full result.
    serde_json::to_string_pretty(result).unwrap_or_else(|_| format!("{result:?}"))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{McpConfig, format_tools_list};
    use serde_json::json;

    #[test]
    fn config_deserializes_from_toml_happy_path() {
        let raw = r#"
[[servers]]
name = "test-server"
command = "echo"
args = ["hello"]

[[servers]]
name = "other"
command = "cat"
"#;
        let config: McpConfig = toml::from_str(raw).unwrap();
        assert_eq!(config.servers.len(), 2);
        assert_eq!(config.servers[0].name, "test-server");
        assert_eq!(config.servers[0].command, "echo");
        assert_eq!(config.servers[0].args, vec!["hello"]);
        assert_eq!(config.servers[1].name, "other");
    }

    #[test]
    fn empty_config_deserializes_happy_path() {
        let config: McpConfig = toml::from_str("").unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn format_tools_list_empty_edge_case() {
        let out = format_tools_list(&[]);
        assert!(out.contains("No tools"));
    }

    #[test]
    fn format_tools_list_with_tools_happy_path() {
        let tools = vec![json!({"name": "read_file", "description": "Reads a file"})];
        let out = format_tools_list(&tools);
        assert!(out.contains("read_file"));
        assert!(out.contains("Reads a file"));
    }

    #[test]
    fn find_server_missing_error_path() {
        use super::find_server;
        let config = McpConfig::default();
        let err = find_server(&config, "nonexistent").unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }
}
