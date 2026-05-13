//! HTTP tool: generic GET/POST/PUT/DELETE with domain allowlist.

use std::collections::HashSet;

use async_trait::async_trait;
use reqwest::{Client, Method};
use serde_json::{Value, json};

use crate::{
    tool::{Tool, ToolContext, ToolError, ToolOutput},
    validator::AllowlistConfig,
};

pub struct HttpTool {
    client: Client,
    allowed_domains: HashSet<String>,
    blocked_domains: HashSet<String>,
}

impl HttpTool {
    pub fn new(allowed_domains: HashSet<String>) -> Self {
        Self::with_policies(allowed_domains, HashSet::new())
    }

    pub fn with_policies(
        allowed_domains: HashSet<String>,
        blocked_domains: HashSet<String>,
    ) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client must build");
        Self {
            client,
            allowed_domains,
            blocked_domains,
        }
    }

    fn check_domain(&self, url: &str) -> Result<(), ToolError> {
        let parsed = url::Url::parse(url)
            .map_err(|e| ToolError::InvalidInput(format!("invalid URL: {e}")))?;
        let host = parsed.host_str().unwrap_or("");
        if self.blocked_domains.contains(host) {
            return Err(ToolError::InvalidInput(format!(
                "domain '{host}' blocked by ~/.helm/allowlist.toml"
            )));
        }
        if self.allowed_domains.is_empty() {
            return Ok(());
        }
        if !self.allowed_domains.contains(host) {
            return Err(ToolError::InvalidInput(format!(
                "domain '{host}' not in allowlist: {:?}",
                self.allowed_domains
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl Tool for HttpTool {
    fn name(&self) -> &'static str {
        "http"
    }

    fn description(&self) -> &'static str {
        "Generic HTTP client: GET, POST, PUT, DELETE with domain allowlist."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": { "type": "string", "enum": ["get", "post", "put", "delete", "head", "patch"] },
                "url": { "type": "string", "description": "Full URL to request" },
                "headers": { "type": "object", "description": "HTTP headers as key-value pairs" },
                "body": { "type": "string", "description": "Request body for POST/PUT/PATCH" }
            },
            "required": ["action", "url"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = input
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("action required".into()))?;
        let url = input
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("url required".into()))?;

        self.check_domain(url)?;

        if ctx.diagnose_mode {
            match action {
                "post" | "put" | "delete" | "patch" => {
                    return Err(ToolError::InvalidInput(format!(
                        "HTTP {action} is a mutating verb and not allowed in diagnose mode"
                    )));
                }
                _ => {}
            }
        }

        let method = match action {
            "get" => Method::GET,
            "post" => Method::POST,
            "put" => Method::PUT,
            "delete" => Method::DELETE,
            "head" => Method::HEAD,
            "patch" => Method::PATCH,
            _ => {
                return Err(ToolError::InvalidInput(format!(
                    "unsupported HTTP method: {action}"
                )));
            }
        };

        let mut req = self.client.request(method, url);
        if let Some(headers) = input.get("headers").and_then(Value::as_object) {
            for (k, v) in headers {
                if let Some(s) = v.as_str() {
                    req = req.header(k, s);
                }
            }
        }
        if let Some(body) = input.get("body").and_then(Value::as_str) {
            req = req.body(body.to_string());
        }

        let response = req
            .send()
            .await
            .map_err(|e| ToolError::Other(format!("request failed: {e}")))?;

        let status = response.status().as_u16();
        let headers: serde_json::Map<String, Value> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap_or(""))))
            .collect();

        let body = response
            .text()
            .await
            .map_err(|e| ToolError::Other(format!("failed to read response body: {e}")))?;

        let body_len = body.len();
        let truncated = body_len > ctx.max_output_bytes;
        let content = if truncated {
            format!(
                "[truncated {} bytes]\n{}",
                body_len,
                &body[..ctx.max_output_bytes.min(body_len)]
            )
        } else {
            body
        };

        let mut metadata = serde_json::Map::new();
        metadata.insert("status".into(), json!(status));
        metadata.insert("headers".into(), json!(headers));
        metadata.insert("truncated".into(), json!(truncated));
        metadata.insert("content_length".into(), json!(body_len));

        Ok(ToolOutput {
            content,
            success: status < 400,
            metadata,
        })
    }

    fn allowed_in_diagnose(&self) -> bool {
        true
    }

    fn all_write_ops_gated_in_diagnose(&self) -> bool {
        true // POST, PUT, DELETE, PATCH are runtime-gated via ctx.diagnose_mode
    }
}

impl Default for HttpTool {
    fn default() -> Self {
        let config = AllowlistConfig::load().unwrap_or_else(|_| AllowlistConfig::permissive());
        Self::with_policies(
            config.allowed_domains.into_iter().collect(),
            config.blocked_domains.into_iter().collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::{io::AsyncWriteExt, net::TcpListener};

    use crate::tool::{Tool, ToolContext};

    use super::HttpTool;

    #[tokio::test]
    async fn schema_has_get_post_delete() {
        let schema = HttpTool::default().input_schema();
        let actions = schema.pointer("/properties/action/enum").unwrap();
        assert!(actions.as_array().unwrap().contains(&json!("get")));
        assert!(actions.as_array().unwrap().contains(&json!("post")));
    }

    #[tokio::test]
    async fn empty_body_is_ok() {
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(_) => {
                eprintln!("skipping http local-server test: listener unavailable");
                return;
            }
        };
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(response).await;
            }
        });
        let dir = tempdir().unwrap();
        let tool = HttpTool::default();
        let result = tool
            .execute(
                json!({"action": "get", "url": format!("http://{addr}/ping")}),
                &ToolContext::new(dir.path().into()),
            )
            .await;
        server.await.unwrap();
        assert!(result.is_ok());
    }

    // ── v1.6 diagnose-mode gates ──

    #[tokio::test]
    async fn diagnose_mode_blocks_mutating_http_verbs() {
        let dir = tempdir().unwrap();
        let mut ctx = ToolContext::new(dir.path().into());
        ctx.diagnose_mode = true;
        let tool = HttpTool::default();

        for verb in ["post", "put", "delete", "patch"] {
            let err = tool
                .execute(
                    json!({"action": verb, "url": "https://example.com/api"}),
                    &ctx,
                )
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("not allowed in diagnose mode"),
                "HTTP {verb} should be blocked in diagnose mode, got: {err}"
            );
        }
    }

    #[tokio::test]
    async fn diagnose_mode_allows_readonly_http_verbs() {
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(_) => {
                eprintln!("skipping http diagnose test: listener unavailable");
                return;
            }
        };
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(response).await;
            }
        });
        let dir = tempdir().unwrap();
        let mut ctx = ToolContext::new(dir.path().into());
        ctx.diagnose_mode = true;
        let tool = HttpTool::default();

        // GET should succeed
        let result = tool
            .execute(
                json!({"action": "get", "url": format!("http://{addr}/ping")}),
                &ctx,
            )
            .await;
        assert!(result.is_ok(), "HTTP GET should succeed in diagnose mode");

        // HEAD should succeed (need a separate server since first accept consumed)
        let listener2 = match TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(_) => return,
        };
        let addr2 = listener2.local_addr().unwrap();
        let server2 = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener2.accept().await {
                let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(response).await;
            }
        });
        let result = tool
            .execute(
                json!({"action": "head", "url": format!("http://{addr2}/ping")}),
                &ctx,
            )
            .await;
        server.await.unwrap();
        server2.await.unwrap();
        assert!(result.is_ok(), "HTTP HEAD should succeed in diagnose mode");
    }
}
