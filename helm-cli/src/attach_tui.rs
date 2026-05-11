//! `helm tui --attach <host:port>` — a small ratatui client that opens a
//! prompt, sends `POST /v1/run` to a remote `helm serve` instance, and
//! renders the NDJSON event stream as a scrolling transcript.
//!
//! Layout: top header strip (target + connection state), middle transcript
//! pane, bottom one-line input. Esc / Ctrl+C exits.

#![allow(dead_code)]

use std::{
    io::{self, Stdout},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self, DisableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

#[derive(Debug, Deserialize)]
struct Line0 {
    event: String,
    #[serde(default)]
    data: Value,
}

#[derive(Debug, Clone)]
struct Entry {
    kind: String,
    text: String,
}

pub async fn run_attach_tui(target: String, token: String) -> Result<()> {
    let url = if target.starts_with("http") {
        format!("{}/v1/run", target.trim_end_matches('/'))
    } else {
        format!("http://{target}/v1/run")
    };
    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("entering alt screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("creating terminal")?;
    let result = run_loop(&mut terminal, url, token).await;
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();
    result
}

struct State {
    target: String,
    input: String,
    transcript: Vec<Entry>,
    busy: bool,
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    url: String,
    token: String,
) -> Result<()> {
    let state = Arc::new(Mutex::new(State {
        target: url.clone(),
        input: String::new(),
        transcript: vec![Entry {
            kind: "system".into(),
            text: format!("attached to {url}. Type a task; Esc or Ctrl+C to exit."),
        }],
        busy: false,
    }));
    let (tx_event, mut rx_event) = mpsc::unbounded_channel::<Entry>();
    loop {
        {
            let mut guard = state.lock().await;
            while let Ok(entry) = rx_event.try_recv() {
                guard.transcript.push(entry);
            }
            draw(terminal, &guard)?;
        }
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
        {
            match (code, modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    break;
                }
                (KeyCode::Backspace, _) => {
                    let mut guard = state.lock().await;
                    guard.input.pop();
                }
                (KeyCode::Enter, _) => {
                    let mut guard = state.lock().await;
                    let task = guard.input.trim().to_owned();
                    if task.is_empty() {
                        continue;
                    }
                    guard.input.clear();
                    guard.busy = true;
                    guard.transcript.push(Entry {
                        kind: "you".into(),
                        text: task.clone(),
                    });
                    let url = url.clone();
                    let token = token.clone();
                    let tx = tx_event.clone();
                    let state_for_done = Arc::clone(&state);
                    tokio::spawn(async move {
                        if let Err(error) = stream_run(&url, &token, &task, tx.clone()).await {
                            let _ = tx.send(Entry {
                                kind: "error".into(),
                                text: error.to_string(),
                            });
                        }
                        let mut g = state_for_done.lock().await;
                        g.busy = false;
                    });
                }
                (KeyCode::Char(c), _) => {
                    let mut guard = state.lock().await;
                    guard.input.push(c);
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn draw(terminal: &mut Terminal<CrosstermBackend<Stdout>>, state: &State) -> Result<()> {
    terminal.draw(|frame| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(frame.area());
        let header_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let header_text = if state.busy {
            format!("HELM attach · {} · running…", state.target)
        } else {
            format!("HELM attach · {} · idle", state.target)
        };
        let header = Paragraph::new(Line::from(Span::styled(header_text, header_style)));
        frame.render_widget(header, chunks[0]);

        let total = state.transcript.len();
        let max_visible = chunks[1].height.saturating_sub(2) as usize;
        let start = total.saturating_sub(max_visible.max(1));
        let lines: Vec<Line> = state.transcript[start..]
            .iter()
            .map(|entry| {
                let (label, color) = match entry.kind.as_str() {
                    "you" => ("you  ", Color::Yellow),
                    "system" => ("info ", Color::Gray),
                    "error" => ("error", Color::Red),
                    "tool" => ("tool ", Color::Magenta),
                    "result" => ("done ", Color::Green),
                    _ => ("helm ", Color::Cyan),
                };
                Line::from(vec![
                    Span::styled(
                        label,
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::raw(entry.text.clone()),
                ])
            })
            .collect();
        let transcript = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("transcript"))
            .wrap(Wrap { trim: true });
        frame.render_widget(transcript, chunks[1]);

        let input_text = if state.busy {
            format!("(streaming) {}", state.input)
        } else {
            format!("> {}", state.input)
        };
        let input =
            Paragraph::new(input_text).block(Block::default().borders(Borders::ALL).title("task"));
        frame.render_widget(input, chunks[2]);
    })?;
    Ok(())
}

async fn stream_run(
    url: &str,
    token: &str,
    task: &str,
    tx: mpsc::UnboundedSender<Entry>,
) -> Result<()> {
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&serde_json::json!({"task": task}))
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
            let raw: Vec<u8> = buf.drain(..=pos).collect();
            let trimmed = std::str::from_utf8(&raw).unwrap_or("").trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(line) = serde_json::from_str::<Line0>(trimmed) else {
                continue;
            };
            let entry = match line.event.as_str() {
                "text_delta" => Entry {
                    kind: "helm".into(),
                    text: line
                        .data
                        .get("chunk")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                },
                "assistant_text" => Entry {
                    kind: "helm".into(),
                    text: line
                        .data
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                },
                "tool_started" => Entry {
                    kind: "tool".into(),
                    text: format!(
                        "▶ {}",
                        line.data
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                    ),
                },
                "tool_finished" => Entry {
                    kind: "tool".into(),
                    text: format!(
                        "✓ {} ({})",
                        line.data
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                        line.data
                            .get("success")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    ),
                },
                "run_finished" | "completed" => Entry {
                    kind: "result".into(),
                    text: line
                        .data
                        .get("final")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(done)")
                        .to_owned(),
                },
                "run_failed" | "error" => Entry {
                    kind: "error".into(),
                    text: line
                        .data
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(error)")
                        .to_owned(),
                },
                _ => continue,
            };
            let _ = tx.send(entry);
        }
    }
    Ok(())
}
