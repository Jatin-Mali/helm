//! Typed Linux network inspection and probe tool.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    command::{run_command, str_field},
    tool::{Tool, ToolContext, ToolError, ToolOutput},
};

/// Tool for IP addresses, routes, listening ports, HTTP probes, and DNS lookup.
#[derive(Debug, Default)]
pub struct NetworkTool;

#[async_trait]
impl Tool for NetworkTool {
    fn name(&self) -> &'static str {
        "network"
    }

    fn description(&self) -> &'static str {
        "Typed network tool: ip addr, routes, listening ports, curl probe, DNS lookup."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": { "type": "string", "enum": ["ip_addr", "routes", "listening_ports", "curl_probe", "dns_lookup"] },
                "url": { "type": "string" },
                "host": { "type": "string" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = str_field(&input, "action")?;
        match action.as_str() {
            "ip_addr" => run_command("ip", &["addr".into()], ctx).await,
            "routes" => run_command("ip", &["route".into()], ctx).await,
            "listening_ports" => run_command("ss", &["-tulpn".into()], ctx).await,
            "curl_probe" => {
                let url = str_field(&input, "url")?;
                run_command(
                    "curl",
                    &["-I".into(), "--max-time".into(), "10".into(), url],
                    ctx,
                )
                .await
            }
            "dns_lookup" => {
                let host = str_field(&input, "host")?;
                run_command("getent", &["hosts".into(), host], ctx).await
            }
            _ => Err(ToolError::InvalidInput(format!(
                "unsupported network action: {action}"
            ))),
        }
    }

    fn allowed_in_diagnose(&self) -> bool {
        true
    }

    fn all_write_ops_gated_in_diagnose(&self) -> bool {
        true // network has only read-only actions (ip_addr, routes, listening_ports, curl_probe, dns_lookup)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use crate::{network::NetworkTool, tool::ToolContext};

    use super::Tool;

    #[tokio::test]
    async fn schema_mentions_ports_happy_path() {
        assert!(
            NetworkTool
                .input_schema()
                .to_string()
                .contains("listening_ports")
        );
    }

    #[tokio::test]
    async fn dns_lookup_requires_host_error_path() {
        let dir = tempdir().unwrap();
        let err = NetworkTool
            .execute(
                json!({"action": "dns_lookup"}),
                &ToolContext::new(dir.path().to_path_buf()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("host"));
    }
}
