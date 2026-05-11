//! Minimal bearer-auth HTTP server for `helm serve` and matching client for
//! `helm tui --attach`. Implements only what the v1.5 remote demo needs:
//! - `POST /v1/run` with `{"task": "..."}` body, returns NDJSON event stream.
//! - `GET  /v1/ping` returns `{"ok": true}`.
//!
//! Authentication: clients must send `Authorization: Bearer <TOKEN>` on every
//! request. Token is generated at startup or supplied via `--token`.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use helm_agent::{AgentEvent, AgentEventSink, Budget, CancellationToken, ReactAgent};
use helm_memory::MemoryStore;
use helm_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc},
};
use uuid::Uuid;

use crate::{ProviderSettings, build_provider, secrets::SecretsStore};

#[derive(Debug, Deserialize)]
struct RunRequest {
    task: String,
}

#[derive(Debug, Serialize)]
struct LineEvent {
    event: String,
    #[serde(skip_serializing_if = "serde_json::Value::is_null", default)]
    data: serde_json::Value,
}

pub struct ServeConfig {
    pub bind: String,
    pub token: String,
    pub provider_settings: ProviderSettings,
    pub memory: Arc<MemoryStore>,
    pub max_iterations: Option<u32>,
    pub auto_approve: bool,
    pub read_only: bool,
    pub secrets: SecretsStore,
}

pub async fn serve(config: ServeConfig) -> Result<()> {
    let listener = TcpListener::bind(&config.bind)
        .await
        .with_context(|| format!("binding {}", config.bind))?;
    let local = listener.local_addr()?;
    eprintln!("[helm serve] listening on http://{local} (bearer token required)");
    let state = Arc::new(ServeState {
        token: config.token,
        provider_settings: config.provider_settings,
        memory: config.memory,
        max_iterations: config.max_iterations,
        auto_approve: config.auto_approve,
        read_only: config.read_only,
        secrets: Arc::new(Mutex::new(config.secrets)),
    });
    loop {
        let (stream, _peer) = listener.accept().await?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, state).await {
                tracing::warn!(target: "helm::serve", "connection error: {error}");
            }
        });
    }
}

struct ServeState {
    token: String,
    provider_settings: ProviderSettings,
    memory: Arc<MemoryStore>,
    max_iterations: Option<u32>,
    auto_approve: bool,
    read_only: bool,
    secrets: Arc<Mutex<SecretsStore>>,
}

async fn handle_connection(mut stream: TcpStream, state: Arc<ServeState>) -> Result<()> {
    let (read_half, mut write_half) = stream.split();
    let mut reader = BufReader::new(read_half);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("").to_owned();

    let mut auth: Option<String> = None;
    let mut content_length: usize = 0;
    loop {
        let mut header = String::new();
        let n = reader.read_line(&mut header).await?;
        if n == 0 || header == "\r\n" || header == "\n" {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("authorization:") {
            auth = Some(value.trim().to_owned());
        } else if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }

    if auth.as_deref() != Some(&format!("bearer {}", state.token.to_ascii_lowercase())) {
        write_response(&mut write_half, 401, "unauthorized\n").await?;
        return Ok(());
    }

    match (method.as_str(), target.as_str()) {
        ("GET", "/v1/ping") => {
            write_response(&mut write_half, 200, "{\"ok\":true}\n").await?;
        }
        ("POST", "/v1/run") => {
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).await?;
            let parsed: RunRequest =
                serde_json::from_slice(&body).context("invalid /v1/run body")?;
            stream_run(state, parsed.task, &mut write_half).await?;
        }
        _ => {
            write_response(&mut write_half, 404, "not found\n").await?;
        }
    }
    Ok(())
}

async fn write_response<W>(write: &mut W, status: u16, body: &str) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    write.write_all(header.as_bytes()).await?;
    write.write_all(body.as_bytes()).await?;
    write.flush().await?;
    Ok(())
}

async fn stream_run<W>(state: Arc<ServeState>, task: String, write: &mut W) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    // NDJSON streaming response: write the chunked headers first.
    let preamble = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    write.write_all(preamble.as_bytes()).await?;

    let secrets = state.secrets.lock().await.clone();
    let (provider, model) = build_provider(&state.provider_settings, &secrets)
        .map_err(|error| anyhow!(error.to_string()))?;
    let mut budget = Budget::default();
    if let Some(max) = state.max_iterations {
        budget.max_iterations = max;
    }
    budget.auto_approve = state.auto_approve;
    budget.read_only = state.read_only;
    let cancel = CancellationToken::new();
    let agent = ReactAgent::new(
        provider,
        ToolRegistry::default(),
        Arc::clone(&state.memory),
        budget,
        model,
    )
    .map_err(|error| anyhow!(error.to_string()))?
    .with_cancel_token(cancel.child());

    let (tx, mut rx) = mpsc::unbounded_channel::<LineEvent>();
    let sink = ChannelSink { tx: tx.clone() };
    let agent_task = tokio::spawn(async move {
        let result = agent.run_with_events(&task, &sink).await;
        let event = match result {
            Ok(res) => LineEvent {
                event: "completed".to_owned(),
                data: json!({
                    "episode_id": res.episode_id,
                    "tokens_in": res.tokens_in,
                    "tokens_out": res.tokens_out,
                    "final": res.final_message,
                }),
            },
            Err(error) => LineEvent {
                event: "error".to_owned(),
                data: json!({"message": error.to_string()}),
            },
        };
        let _ = tx.send(event);
    });

    while let Some(line) = rx.recv().await {
        let body = serde_json::to_string(&line)? + "\n";
        let chunk = format!("{:X}\r\n{body}\r\n", body.len());
        if write.write_all(chunk.as_bytes()).await.is_err() {
            break;
        }
        let _ = write.flush().await;
    }
    let _ = write.write_all(b"0\r\n\r\n").await;
    let _ = write.flush().await;
    let _ = agent_task.await;
    Ok(())
}

struct ChannelSink {
    tx: mpsc::UnboundedSender<LineEvent>,
}

impl AgentEventSink for ChannelSink {
    fn emit(&self, event: AgentEvent) {
        let (kind, data) = match &event {
            AgentEvent::RunStarted { episode_id, goal } => (
                "run_started",
                json!({"episode_id": episode_id, "goal": goal}),
            ),
            AgentEvent::TextDelta { chunk } => ("text_delta", json!({"chunk": chunk})),
            AgentEvent::AssistantText { text } => ("assistant_text", json!({"text": text})),
            AgentEvent::ToolCallStarted { id, name } => {
                ("tool_started", json!({"id": id, "name": name}))
            }
            AgentEvent::ToolCallFinished {
                id,
                name,
                success,
                content,
            } => (
                "tool_finished",
                json!({"id": id, "name": name, "success": success, "content": content}),
            ),
            AgentEvent::ToolCallDenied { id, name, reason } => (
                "tool_denied",
                json!({"id": id, "name": name, "reason": reason}),
            ),
            AgentEvent::RunFinished { result } => (
                "run_finished",
                json!({
                    "episode_id": result.episode_id,
                    "tokens_in": result.tokens_in,
                    "tokens_out": result.tokens_out,
                    "final": result.final_message,
                }),
            ),
            AgentEvent::RunFailed { episode_id, error } => (
                "run_failed",
                json!({"episode_id": episode_id, "error": error}),
            ),
            _ => ("event", json!({})),
        };
        let _ = self.tx.send(LineEvent {
            event: kind.to_owned(),
            data,
        });
    }
}

pub fn generate_token() -> String {
    let mut t = Uuid::new_v4().to_string();
    t.retain(|c| c != '-');
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    format!("{t}{ts:x}")
}

#[allow(dead_code)]
pub async fn attach(target: &str, token: &str) -> Result<()> {
    let url = if target.starts_with("http") {
        format!("{}/v1/run", target.trim_end_matches('/'))
    } else {
        format!("http://{target}/v1/run")
    };
    eprintln!("[helm attach] connected to {url}");
    eprintln!("Type a task and press Enter. Ctrl+C to exit.");
    let stdin = tokio::io::stdin();
    let mut lines = tokio::io::BufReader::new(stdin).lines();
    while let Some(line) = lines.next_line().await? {
        let task = line.trim().to_owned();
        if task.is_empty() {
            continue;
        }
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .bearer_auth(token)
            .json(&json!({"task": task}))
            .send()
            .await
            .context("posting /v1/run")?;
        if !response.status().is_success() {
            bail!("server returned {}", response.status());
        }
        let mut response = response;
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = response.chunk().await.context("reading chunk")? {
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let trimmed = std::str::from_utf8(&line).unwrap_or("").trim();
                if trimmed.is_empty() {
                    continue;
                }
                println!("{trimmed}");
            }
        }
    }
    Ok(())
}
