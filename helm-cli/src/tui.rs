//! Full-screen HELM terminal UI built with ratatui.

use std::{collections::HashMap, io, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use helm_agent::{AgentEvent, AgentEventSink, Budget, ReactAgent, RunResult};
use helm_core::{Capability, HelmError};
use helm_memory::MemoryStore;
use helm_tools::ToolRegistry;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Widget,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{ProviderSettings, build_provider, provider_choice_name};

/// Runtime dependencies needed by the TUI.
pub(crate) struct TuiRuntime {
    pub(crate) provider_settings: ProviderSettings,
    pub(crate) db_path: PathBuf,
    pub(crate) memory: Arc<MemoryStore>,
    pub(crate) max_iterations: Option<u32>,
}

/// Starts the interactive terminal UI.
pub(crate) async fn run_tui(runtime: TuiRuntime) -> Result<()> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to initialize terminal")?;
    terminal.clear().ok();

    let result = TuiApp::new(runtime).run(&mut terminal).await;

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    result
}

#[derive(Debug)]
enum UiEvent {
    Input(Event),
    Agent {
        run_id: u64,
        event: AgentEvent,
    },
    AgentDone {
        run_id: u64,
        result: Result<RunResult, HelmError>,
    },
    Tick,
}

#[derive(Clone)]
struct ChannelEventSink {
    tx: mpsc::UnboundedSender<UiEvent>,
    run_id: u64,
}

impl AgentEventSink for ChannelEventSink {
    fn emit(&self, event: AgentEvent) {
        self.tx
            .send(UiEvent::Agent {
                run_id: self.run_id,
                event,
            })
            .ok();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MessageRole {
    User,
    Assistant,
    System,
    Activity,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatMessage {
    role: MessageRole,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolTimelineItem {
    status: String,
    tool: String,
    detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionState {
    episode_id: Option<String>,
    chat: Vec<ChatMessage>,
    transcript_scroll: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputState {
    text: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    draft: String,
}

impl InputState {
    fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            draft: String::new(),
        }
    }

    fn insert(&mut self, ch: char) {
        let byte = char_to_byte(&self.text, self.cursor);
        self.text.insert(byte, ch);
        self.cursor = self.cursor.saturating_add(1);
    }

    fn insert_newline(&mut self) {
        self.insert('\n');
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let byte = char_to_byte(&self.text, self.cursor.saturating_sub(1));
        self.text.remove(byte);
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn delete(&mut self) {
        if self.cursor >= self.text.chars().count() {
            return;
        }
        let byte = char_to_byte(&self.text, self.cursor);
        self.text.remove(byte);
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.history_index = None;
    }

    fn take_submit(&mut self) -> Option<String> {
        let task = self.text.trim().to_owned();
        if task.is_empty() {
            return None;
        }
        if self.history.last() != Some(&task) {
            self.history.push(task.clone());
        }
        self.clear();
        Some(task)
    }

    fn previous_history(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.draft = self.text.clone();
                self.history_index = Some(self.history.len().saturating_sub(1));
            }
            Some(0) => {}
            Some(index) => self.history_index = Some(index.saturating_sub(1)),
        }
        if let Some(index) = self.history_index {
            self.text = self.history[index].clone();
            self.cursor = self.text.chars().count();
        }
    }

    fn next_history(&mut self) {
        match self.history_index {
            None => {}
            Some(index) if index + 1 < self.history.len() => {
                self.history_index = Some(index + 1);
                self.text = self.history[index + 1].clone();
                self.cursor = self.text.chars().count();
            }
            Some(_) => {
                self.history_index = None;
                self.text = self.draft.clone();
                self.cursor = self.text.chars().count();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelFocus {
    Input,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModalState {
    CommandPalette,
    Permission {
        capability: Capability,
        tool_name: String,
        taint: String,
        detail: String,
    },
    ProviderSelector,
    ModelSelector,
    Error(String),
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandPaletteState {
    query: String,
    selected: usize,
}

impl CommandPaletteState {
    fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
        }
    }

    fn filtered(&self) -> Vec<CommandAction> {
        let query = self.query.to_ascii_lowercase();
        CommandAction::all()
            .into_iter()
            .filter(|action| action.label().to_ascii_lowercase().contains(&query))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandAction {
    NewSession,
    Replay,
    Doctor,
    Provider,
    Model,
    Permissions,
    Audit,
    Skills,
    Browser,
    Help,
}

impl CommandAction {
    fn all() -> Vec<Self> {
        vec![
            Self::NewSession,
            Self::Replay,
            Self::Doctor,
            Self::Provider,
            Self::Model,
            Self::Permissions,
            Self::Audit,
            Self::Skills,
            Self::Browser,
            Self::Help,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Self::NewSession => "New Session",
            Self::Replay => "Replay Episode",
            Self::Doctor => "Doctor",
            Self::Provider => "Provider Selector",
            Self::Model => "Model Selector",
            Self::Permissions => "Permissions",
            Self::Audit => "Audit Verify",
            Self::Skills => "Skills",
            Self::Browser => "Browser Status",
            Self::Help => "Help",
        }
    }
}

struct TuiApp {
    runtime: Arc<TuiRuntimeInner>,
    session: SessionState,
    input: InputState,
    focus: PanelFocus,
    modal: Option<ModalState>,
    palette: CommandPaletteState,
    running: bool,
    spinner: usize,
    provider_name: String,
    model: String,
    status_note: String,
    pending_tool_summaries: HashMap<String, String>,
    active_tool_cells: HashMap<String, usize>,
    active_run_id: u64,
    agent_task: Option<JoinHandle<()>>,
}

struct TuiRuntimeInner {
    provider_settings: ProviderSettings,
    db_path: PathBuf,
    memory: Arc<MemoryStore>,
    max_iterations: Option<u32>,
}

impl TuiApp {
    fn new(runtime: TuiRuntime) -> Self {
        let provider_name = provider_choice_name(runtime.provider_settings.choice).to_owned();
        let model = runtime
            .provider_settings
            .model
            .clone()
            .unwrap_or_else(|| "auto".to_owned());
        Self {
            runtime: Arc::new(TuiRuntimeInner {
                provider_settings: runtime.provider_settings,
                db_path: runtime.db_path,
                memory: runtime.memory,
                max_iterations: runtime.max_iterations,
            }),
            session: SessionState::default(),
            input: InputState::new(),
            focus: PanelFocus::Input,
            modal: None,
            palette: CommandPaletteState::new(),
            running: false,
            spinner: 0,
            provider_name,
            model,
            status_note: "ready".to_owned(),
            pending_tool_summaries: HashMap::new(),
            active_tool_cells: HashMap::new(),
            active_run_id: 0,
            agent_task: None,
        }
    }

    async fn run(mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel::<UiEvent>();
        spawn_input_thread(tx.clone());
        spawn_tick_task(tx.clone());

        self.session.chat.push(ChatMessage {
            role: MessageRole::System,
            text: "HELM ready. Type a task, or Ctrl+P for commands.".to_owned(),
        });

        loop {
            terminal
                .draw(|frame| self.render(frame))
                .context("failed to draw TUI")?;
            let Some(event) = rx.recv().await else {
                return Ok(());
            };
            if self.handle_ui_event(event, tx.clone()).await? {
                return Ok(());
            }
        }
    }

    async fn handle_ui_event(
        &mut self,
        event: UiEvent,
        tx: mpsc::UnboundedSender<UiEvent>,
    ) -> Result<bool> {
        match event {
            UiEvent::Input(Event::Key(key)) if key.kind == event::KeyEventKind::Press => {
                self.handle_key(key, tx).await
            }
            UiEvent::Input(Event::Resize(_, _)) => Ok(false),
            UiEvent::Input(_) => Ok(false),
            UiEvent::Tick => {
                self.spinner = self.spinner.wrapping_add(1);
                Ok(false)
            }
            UiEvent::Agent { run_id, event } => {
                if run_id == self.active_run_id {
                    self.apply_agent_event(event);
                }
                Ok(false)
            }
            UiEvent::AgentDone { run_id, result } => {
                if run_id != self.active_run_id {
                    return Ok(false);
                }
                self.running = false;
                self.agent_task = None;
                self.pending_tool_summaries.clear();
                self.active_tool_cells.clear();
                match result {
                    Ok(run) => {
                        self.session.episode_id = Some(run.episode_id.clone());
                        let final_text = if run.final_message.trim().is_empty() {
                            run.last_assistant_text
                                .unwrap_or_else(|| "(no assistant text)".to_owned())
                        } else {
                            run.final_message
                        };
                        if !final_text.trim().is_empty()
                            && final_text != "(no final message)"
                            && !self.chat_ends_with(MessageRole::Assistant, &final_text)
                        {
                            self.push_chat(MessageRole::Assistant, final_text);
                        }
                        self.record_tool_event(
                            "done",
                            "episode",
                            format!(
                                "{} iter(s), {} in / {} out tokens",
                                run.iterations, run.tokens_in, run.tokens_out
                            ),
                        );
                    }
                    Err(error) => {
                        self.status_note = "failed".to_owned();
                        self.push_chat(MessageRole::Error, friendly_error(&error.to_string()));
                        self.modal = Some(ModalState::Error(friendly_error(&error.to_string())));
                    }
                }
                Ok(false)
            }
        }
    }

    async fn handle_key(
        &mut self,
        key: KeyEvent,
        tx: mpsc::UnboundedSender<UiEvent>,
    ) -> Result<bool> {
        if self.modal.is_some() {
            return self.handle_modal_key(key);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => {
                    if self.running {
                        self.cancel_running_task();
                        return Ok(false);
                    }
                    return Ok(true);
                }
                KeyCode::Char('n') => self.new_session(),
                KeyCode::Char('p') => self.open_palette(),
                KeyCode::Char('r') => self.push_chat(MessageRole::System, self.replay_hint()),
                KeyCode::Char('a') => {
                    self.modal = Some(ModalState::Permission {
                        capability: Capability::ShellShell,
                        tool_name: "pending".to_owned(),
                        taint: "user".to_owned(),
                        detail: "No pending permission request.".to_owned(),
                    });
                }
                KeyCode::Char('d') => {
                    self.push_chat(
                        MessageRole::System,
                        "No pending permission request to deny.",
                    );
                }
                KeyCode::Char('l') => {}
                KeyCode::Char('u') => self.input.clear(),
                _ => {}
            }
            return Ok(false);
        }

        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                self.input.insert_newline()
            }
            KeyCode::Enter => self.submit(tx).await?,
            KeyCode::Char(ch) => self.input.insert(ch),
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Delete => self.input.delete(),
            KeyCode::Left => self.input.cursor = self.input.cursor.saturating_sub(1),
            KeyCode::Right => {
                self.input.cursor = (self.input.cursor + 1).min(self.input.text.chars().count());
            }
            KeyCode::Home => self.input.cursor = 0,
            KeyCode::End => self.input.cursor = self.input.text.chars().count(),
            KeyCode::Up => self.input.previous_history(),
            KeyCode::Down => self.input.next_history(),
            KeyCode::PageUp => {
                self.session.transcript_scroll = self.session.transcript_scroll.saturating_add(5)
            }
            KeyCode::PageDown => {
                self.session.transcript_scroll = self.session.transcript_scroll.saturating_sub(5)
            }
            KeyCode::Tab | KeyCode::BackTab => self.focus = PanelFocus::Input,
            KeyCode::Esc => self.focus = PanelFocus::Input,
            _ => {}
        }
        Ok(false)
    }

    fn handle_modal_key(&mut self, key: KeyEvent) -> Result<bool> {
        match self.modal.clone() {
            Some(ModalState::CommandPalette) => match key.code {
                KeyCode::Esc => self.modal = None,
                KeyCode::Enter => {
                    let commands = self.palette.filtered();
                    if let Some(command) = commands.get(self.palette.selected).copied() {
                        self.execute_command(command);
                    }
                }
                KeyCode::Char(ch) => {
                    self.palette.query.push(ch);
                    self.palette.selected = 0;
                }
                KeyCode::Backspace => {
                    self.palette.query.pop();
                    self.palette.selected = 0;
                }
                KeyCode::Up => self.palette.selected = self.palette.selected.saturating_sub(1),
                KeyCode::Down => {
                    let max = self.palette.filtered().len().saturating_sub(1);
                    self.palette.selected = (self.palette.selected + 1).min(max);
                }
                _ => {}
            },
            Some(ModalState::Permission { capability, .. }) => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.push_chat(MessageRole::System, "Permission denied.");
                    self.modal = None;
                }
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let memory = Arc::clone(&self.runtime.memory);
                    tokio::spawn(async move {
                        memory
                            .grant_capability(capability, helm_core::GrantScope::Once)
                            .await
                            .ok();
                    });
                    self.push_chat(MessageRole::System, format!("Granted {capability} once."));
                    self.modal = None;
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    let memory = Arc::clone(&self.runtime.memory);
                    tokio::spawn(async move {
                        memory
                            .grant_capability(capability, helm_core::GrantScope::Session)
                            .await
                            .ok();
                    });
                    self.push_chat(
                        MessageRole::System,
                        format!("Granted {capability} for session."),
                    );
                    self.modal = None;
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    let memory = Arc::clone(&self.runtime.memory);
                    tokio::spawn(async move {
                        memory
                            .grant_capability(capability, helm_core::GrantScope::Always)
                            .await
                            .ok();
                    });
                    self.push_chat(MessageRole::System, format!("Granted {capability} always."));
                    self.modal = None;
                }
                _ => {}
            },
            Some(_) => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    self.modal = None;
                }
            }
            None => {}
        }
        Ok(false)
    }

    async fn submit(&mut self, tx: mpsc::UnboundedSender<UiEvent>) -> Result<()> {
        if self.running {
            return Ok(());
        }
        let Some(task) = self.input.take_submit() else {
            return Ok(());
        };
        self.push_chat(MessageRole::User, task.clone());
        self.record_tool_event("queued", "agent", "task submitted");
        self.running = true;
        self.status_note = "running".to_owned();
        self.active_run_id = self.active_run_id.saturating_add(1);
        let run_id = self.active_run_id;

        let runtime = Arc::clone(&self.runtime);
        self.agent_task = Some(tokio::spawn(async move {
            let result = run_agent_task(runtime, task, tx.clone(), run_id).await;
            tx.send(UiEvent::AgentDone { run_id, result }).ok();
        }));

        Ok(())
    }

    fn cancel_running_task(&mut self) {
        if let Some(task) = self.agent_task.take() {
            task.abort();
        }
        self.running = false;
        self.active_run_id = self.active_run_id.saturating_add(1);
        self.status_note = "cancelled".to_owned();
        self.pending_tool_summaries.clear();
        self.active_tool_cells.clear();
        self.push_chat(
            MessageRole::System,
            "Cancelled current task. HELM is ready for the next prompt.",
        );
        self.record_tool_event("cancel", "agent", "task aborted");
    }

    fn replay_hint(&self) -> String {
        match self.session.episode_id.as_deref() {
            Some(id) => format!("Replay this episode with `helm replay {id}`."),
            None => "No episode is loaded yet.".to_owned(),
        }
    }

    fn doctor_hint(&self) -> String {
        format!(
            "Provider: {} | Model: {} | Database: {}\nRun `helm doctor --provider {} --model {}` for live provider diagnostics.",
            self.provider_name,
            self.model,
            self.runtime.db_path.display(),
            self.provider_name,
            self.model
        )
    }

    fn browser_hint(&self) -> String {
        "Browser automation is PinchTab-backed. Browser content is external-tainted; privileged actions require fresh approval.".to_owned()
    }

    fn apply_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::RunStarted { episode_id, .. } => {
                self.session.episode_id = Some(episode_id.clone());
                self.record_tool_event("run", "episode", episode_id);
            }
            AgentEvent::ProviderCallStarted {
                iteration,
                provider,
                model,
            } => {
                self.provider_name = provider.clone();
                self.model = model.clone();
                self.record_tool_event(
                    "call",
                    provider,
                    format!("iteration {iteration}, model {model}"),
                );
            }
            AgentEvent::ProviderCallFinished {
                iteration,
                stop_reason,
                tokens_in,
                tokens_out,
            } => self.record_tool_event(
                "done",
                "provider",
                format!("{iteration:?} {stop_reason:?}, {tokens_in}/{tokens_out} tokens"),
            ),
            AgentEvent::AssistantText { text } => {
                if !text.trim().is_empty() {
                    self.push_chat(MessageRole::Assistant, text);
                }
            }
            AgentEvent::ToolCallParsed { id, name, input } => {
                self.pending_tool_summaries
                    .insert(id, tool_call_summary(&name, &input));
            }
            AgentEvent::ToolCallValidated { name, .. } => {
                self.record_tool_event("valid", name, "input accepted");
            }
            AgentEvent::ToolCallStarted { id, name } => {
                self.start_tool_cell(&id, &name);
            }
            AgentEvent::ToolCallFinished {
                id,
                name,
                success,
                content,
                ..
            } => {
                self.finish_tool_cell(&id, &name, success, &content);
            }
            AgentEvent::ToolCallDenied { name, reason, .. } => {
                self.record_tool_event("deny", name, reason.clone());
                self.push_chat(MessageRole::Error, reason);
            }
            AgentEvent::PermissionRequested {
                capability,
                tool_name,
                taint,
            } => {
                self.modal = Some(ModalState::Permission {
                    capability,
                    tool_name,
                    taint: format!("{taint:?}"),
                    detail: "This action needs explicit approval before it can run.".to_owned(),
                });
            }
            AgentEvent::FormatRecoveryUsed { format } => {
                self.record_tool_event("recover", "parser", format);
            }
            AgentEvent::CorrectionUsed { count, tool_name } => {
                self.record_tool_event("correct", tool_name, format!("correction {count}"));
            }
            AgentEvent::PostconditionWarning { warning } => {
                self.push_chat(MessageRole::Error, warning.clone());
                self.record_tool_event("warn", "verify", warning);
            }
            AgentEvent::RunFinished { .. } | AgentEvent::RunFailed { .. } => {}
        }
    }

    fn push_chat(&mut self, role: MessageRole, text: impl Into<String>) {
        self.session.chat.push(ChatMessage {
            role,
            text: sanitize_display_text(&text.into()),
        });
        self.session.transcript_scroll = 0;
    }

    fn record_tool_event(
        &mut self,
        status: impl Into<String>,
        tool: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let item = ToolTimelineItem {
            status: sanitize_one_line(&status.into()),
            tool: sanitize_one_line(&tool.into()),
            detail: sanitize_one_line(&detail.into()),
        };
        self.status_note = activity_status_note(&item);
        if let Some(text) = visible_activity_text(&item) {
            self.session.chat.push(ChatMessage {
                role: MessageRole::Activity,
                text,
            });
            self.session.transcript_scroll = 0;
        }
    }

    fn start_tool_cell(&mut self, id: &str, name: &str) {
        let summary = self
            .pending_tool_summaries
            .get(id)
            .cloned()
            .unwrap_or_else(|| name.to_owned());
        let text = format!("running {name}: {summary}");
        self.status_note = format!("running {name}");
        self.session.chat.push(ChatMessage {
            role: MessageRole::Activity,
            text: sanitize_display_text(&text),
        });
        self.active_tool_cells
            .insert(id.to_owned(), self.session.chat.len().saturating_sub(1));
        self.session.transcript_scroll = 0;
    }

    fn finish_tool_cell(&mut self, id: &str, name: &str, success: bool, content: &str) {
        let summary = self
            .pending_tool_summaries
            .remove(id)
            .unwrap_or_else(|| name.to_owned());
        let preview = tool_output_preview(content);
        let text = if success {
            if preview.is_empty() {
                format!("ran {name}: {summary}")
            } else {
                format!("ran {name}: {summary}\n{preview}")
            }
        } else if preview.is_empty() {
            format!("{name} failed: {summary}")
        } else {
            format!("{name} failed: {summary}\n{preview}")
        };
        self.status_note = if success {
            format!("{name} ok")
        } else {
            format!("{name} failed")
        };
        if let Some(index) = self.active_tool_cells.remove(id)
            && let Some(message) = self.session.chat.get_mut(index)
        {
            message.role = if success {
                MessageRole::Activity
            } else {
                MessageRole::Error
            };
            message.text = sanitize_display_text(&text);
            self.session.transcript_scroll = 0;
            return;
        }
        self.push_chat(
            if success {
                MessageRole::Activity
            } else {
                MessageRole::Error
            },
            text,
        );
    }

    fn chat_ends_with(&self, role: MessageRole, text: &str) -> bool {
        self.session
            .chat
            .last()
            .is_some_and(|message| message.role == role && message.text.trim() == text.trim())
    }

    fn new_session(&mut self) {
        self.session = SessionState::default();
        self.input.clear();
        self.push_chat(MessageRole::System, "New session started.");
        self.modal = None;
    }

    fn open_palette(&mut self) {
        self.palette = CommandPaletteState::new();
        self.modal = Some(ModalState::CommandPalette);
    }

    fn execute_command(&mut self, command: CommandAction) {
        self.modal = None;
        match command {
            CommandAction::NewSession => self.new_session(),
            CommandAction::Replay => self.push_chat(MessageRole::System, self.replay_hint()),
            CommandAction::Doctor => self.push_chat(MessageRole::System, self.doctor_hint()),
            CommandAction::Provider => self.modal = Some(ModalState::ProviderSelector),
            CommandAction::Model => self.modal = Some(ModalState::ModelSelector),
            CommandAction::Permissions => {
                self.modal = Some(ModalState::Permission {
                    capability: Capability::ShellShell,
                    tool_name: "permissions".to_owned(),
                    taint: "user".to_owned(),
                    detail: "Use `helm permissions list/grant/revoke` for exact grant control."
                        .to_owned(),
                })
            }
            CommandAction::Audit => {
                self.record_tool_event("audit", "verify", "run `helm audit verify`")
            }
            CommandAction::Skills => {
                self.record_tool_event("skills", "library", "run `helm skills list`")
            }
            CommandAction::Browser => self.push_chat(MessageRole::System, self.browser_hint()),
            CommandAction::Help => self.modal = Some(ModalState::Help),
        }
    }

    fn render(&self, frame: &mut Frame<'_>) {
        render_app(self, frame.area(), frame.buffer_mut());
    }
}

async fn run_agent_task(
    runtime: Arc<TuiRuntimeInner>,
    task: String,
    tx: mpsc::UnboundedSender<UiEvent>,
    run_id: u64,
) -> Result<RunResult, HelmError> {
    let (provider, model) = build_provider(&runtime.provider_settings)
        .map_err(|error| HelmError::Provider(helm_core::ProviderError::Other(error.to_string())))?;
    let mut budget = Budget::default();
    if let Some(max) = runtime.max_iterations {
        budget.max_iterations = max;
    }
    let agent = ReactAgent::new(
        provider,
        ToolRegistry::default(),
        Arc::clone(&runtime.memory),
        budget,
        model,
    )?;
    let sink = ChannelEventSink { tx, run_id };
    agent.run_with_events(&task, &sink).await
}

fn spawn_input_thread(tx: mpsc::UnboundedSender<UiEvent>) {
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if tx.send(UiEvent::Input(event)).is_err() {
                break;
            }
        }
    });
}

fn spawn_tick_task(tx: mpsc::UnboundedSender<UiEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(180));
        loop {
            interval.tick().await;
            if tx.send(UiEvent::Tick).is_err() {
                break;
            }
        }
    });
}

fn render_app(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    if area.width < 20 || area.height < 8 {
        Paragraph::new("HELM needs a larger terminal")
            .block(Block::default().borders(Borders::ALL).title("HELM"))
            .render(area, buf);
        return;
    }

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(input_height(&app.input.text, area.width)),
        ])
        .split(area);

    render_status(app, vertical[0], buf);
    render_chat(app, vertical[1], buf);
    render_input(app, vertical[2], buf);

    if let Some(modal) = &app.modal {
        render_modal(app, modal, centered_rect(72, 52, area), buf);
    }
}

fn render_status(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let spinner = if app.running {
        ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"][app.spinner % 8]
    } else {
        " "
    };
    let episode = app.session.episode_id.as_deref().unwrap_or("-");
    let text = format!(
        " {spinner} HELM  provider={} model={} episode={}  {}  Ctrl+C cancel/quit  Ctrl+P  PgUp/PgDn",
        app.provider_name,
        app.model,
        truncate(episode, 8),
        truncate(&app.status_note, 36)
    );
    Paragraph::new(text)
        .style(Style::default().fg(Color::Black).bg(Color::Green))
        .render(area, buf);
}

fn render_chat(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let lines = chat_lines(&app.session.chat);
    let visible = visible_lines_from_bottom(lines, area.height, app.session.transcript_scroll);
    Paragraph::new(visible)
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

fn render_input(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let title = "Prompt  Enter submit | Alt+Enter newline";
    let block = Block::default().borders(Borders::ALL).title(title);
    let paragraph = Paragraph::new(app.input.text.as_str())
        .block(block)
        .wrap(Wrap { trim: false });
    paragraph.render(area, buf);
}

fn render_modal(app: &TuiApp, modal: &ModalState, area: Rect, buf: &mut Buffer) {
    Clear.render(area, buf);
    match modal {
        ModalState::CommandPalette => render_palette(app, area, buf),
        ModalState::Permission {
            capability,
            tool_name,
            taint,
            detail,
        } => {
            let text = vec![
                Line::from(format!("capability: {capability}")),
                Line::from(format!("tool: {tool_name}")),
                Line::from(format!("taint: {taint}")),
                Line::from(""),
                Line::from(detail.as_str()),
                Line::from(""),
                Line::from("[y] once  [s] session  [a] always  [n/Esc] deny"),
            ];
            Paragraph::new(text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Permission Required"),
                )
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::ProviderSelector => {
            Paragraph::new("Providers: Groq, OpenRouter, Gemini, NVIDIA NIM, Anthropic, Ollama, OpenAI-compatible\n\nUse `helm init --force` or config.toml to persist provider settings.\nEsc closes.")
                .block(Block::default().borders(Borders::ALL).title("Provider Selector"))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::ModelSelector => {
            Paragraph::new("Enter model with CLI/config for now. Recommended: openai/gpt-oss-20b for Groq.\nEsc closes.")
                .block(Block::default().borders(Borders::ALL).title("Model Selector"))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::Error(message) => {
            Paragraph::new(message.as_str())
                .block(Block::default().borders(Borders::ALL).title("Error"))
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::Help => {
            Paragraph::new("Enter submit | Alt+Enter newline | Ctrl+P commands | Ctrl+N new session | Ctrl+C cancel running task, then Ctrl+C again to quit | PageUp/PageDown scroll")
                .block(Block::default().borders(Borders::ALL).title("Help"))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
    }
}

fn render_palette(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let commands = app.palette.filtered();
    let mut lines = vec![Line::from(vec![
        Span::styled("query: ", Style::default().fg(Color::Gray)),
        Span::raw(app.palette.query.as_str()),
    ])];
    lines.push(Line::from(""));
    for (index, command) in commands.iter().enumerate() {
        let style = if index == app.palette.selected {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default()
        };
        lines.push(Line::styled(command.label(), style));
    }
    Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Command Palette"),
        )
        .render(area, buf);
}

fn chat_lines(messages: &[ChatMessage]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for message in messages {
        let (label, style) = match message.role {
            MessageRole::User => (
                "❯",
                Style::default()
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD),
            ),
            MessageRole::Assistant => (
                " ",
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            MessageRole::System => ("•", Style::default().fg(Color::DarkGray)),
            MessageRole::Activity => ("•", Style::default().fg(Color::DarkGray)),
            MessageRole::Error => (
                "!",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        };
        for (index, line) in message.text.lines().enumerate() {
            if index == 0 {
                lines.push(Line::from(vec![
                    Span::styled(format!("{label:>2}  "), style),
                    Span::raw(line.to_owned()),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::raw(line.to_owned()),
                ]));
            }
        }
        if message.role != MessageRole::Activity {
            lines.push(Line::from(""));
        }
    }
    lines
}

fn visible_activity_text(item: &ToolTimelineItem) -> Option<String> {
    match item.status.as_str() {
        "queued" | "run" | "call" | "done" | "parsed" | "valid" => None,
        "start" => Some(format!("running {}", item.tool)),
        "ok" => Some(format!("{} completed", item.tool)),
        "err" => Some(format!(
            "{} failed: {}",
            item.tool,
            truncate(&item.detail, 96)
        )),
        "deny" => Some(format!(
            "{} denied: {}",
            item.tool,
            truncate(&item.detail, 96)
        )),
        "warn" => Some(format!("warning: {}", truncate(&item.detail, 120))),
        "recover" => Some(format!("recovered tool-call format: {}", item.detail)),
        "correct" => Some(format!("corrected {} ({})", item.tool, item.detail)),
        "cancel" => Some("task cancelled".to_owned()),
        "audit" | "skills" => Some(format!("{}: {}", item.tool, item.detail)),
        _ => Some(format!(
            "{} {} — {}",
            item.status,
            item.tool,
            truncate(&item.detail, 120)
        )),
    }
}

fn activity_status_note(item: &ToolTimelineItem) -> String {
    match item.status.as_str() {
        "queued" => "queued".to_owned(),
        "run" => format!("episode {}", truncate(&item.detail, 8)),
        "call" => format!("calling {}", item.tool),
        "done" if item.tool == "provider" => "provider response received".to_owned(),
        "done" if item.tool == "episode" => "ready".to_owned(),
        "parsed" => format!("parsed {}", item.tool),
        "valid" => format!("validated {}", item.tool),
        "start" => format!("running {}", item.tool),
        "ok" => format!("{} ok", item.tool),
        "err" => format!("{} failed", item.tool),
        "deny" => format!("{} denied", item.tool),
        "warn" => "warning".to_owned(),
        "recover" => "format recovery used".to_owned(),
        "correct" => "correction sent".to_owned(),
        "cancel" => "cancelled".to_owned(),
        _ => format!("{} {}", item.status, item.tool),
    }
}

fn tool_call_summary(name: &str, input: &serde_json::Value) -> String {
    let object = input.as_object();
    match name {
        "shell" => {
            let command = object
                .and_then(|value| value.get("command"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("command");
            let mode = object
                .and_then(|value| value.get("mode"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("exec");
            if let Some(path) = object
                .and_then(|value| value.get("redirect_stdout_to"))
                .and_then(serde_json::Value::as_str)
            {
                format!(
                    "{mode} `{}` -> {}",
                    truncate(command, 96),
                    truncate(path, 72)
                )
            } else {
                format!("{mode} `{}`", truncate(command, 120))
            }
        }
        "fs_read" => object
            .and_then(|value| value.get("path"))
            .and_then(serde_json::Value::as_str)
            .map(|path| format!("read {}", truncate(path, 120)))
            .unwrap_or_else(|| truncate(compact_json(input), 140)),
        "fs_write" => {
            let path = object
                .and_then(|value| value.get("path"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("path");
            let mode = object
                .and_then(|value| value.get("mode"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("create_only");
            format!("{mode} {}", truncate(path, 120))
        }
        "browser" => {
            let action = object
                .and_then(|value| value.get("action"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("action");
            let target = object
                .and_then(|value| value.get("url").or_else(|| value.get("ref")))
                .and_then(serde_json::Value::as_str);
            target
                .map(|target| format!("{action} {}", truncate(target, 120)))
                .unwrap_or_else(|| action.to_owned())
        }
        "service" | "package" | "process" | "disk" | "network" | "logs" => object
            .and_then(|value| value.get("action"))
            .and_then(serde_json::Value::as_str)
            .map(|action| format!("{action} {}", truncate(compact_json(input), 120)))
            .unwrap_or_else(|| truncate(compact_json(input), 140)),
        _ => truncate(compact_json(input), 140),
    }
}

fn tool_output_preview(content: &str) -> String {
    let content = sanitize_display_text(content);
    let mut useful = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "STDOUT:" || line == "STDERR:" {
            continue;
        }
        if line.starts_with("[exit code:") {
            continue;
        }
        useful.push(line.to_owned());
        if useful.len() == 2 {
            break;
        }
    }
    truncate(useful.join("\n"), 220)
}

fn visible_lines_from_bottom(
    lines: Vec<Line<'static>>,
    height: u16,
    scroll_from_bottom: usize,
) -> Vec<Line<'static>> {
    let height = height as usize;
    if height == 0 || lines.len() <= height {
        return lines;
    }
    let end = lines.len().saturating_sub(scroll_from_bottom).max(height);
    let start = end.saturating_sub(height);
    lines[start..end.min(lines.len())].to_vec()
}

fn input_height(input: &str, width: u16) -> u16 {
    let usable = width.saturating_sub(4).max(20) as usize;
    let wrapped_lines: usize = input
        .lines()
        .map(|line| line.chars().count().max(1).div_ceil(usable))
        .sum::<usize>()
        .max(1);
    (wrapped_lines as u16 + 2).clamp(3, 7)
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(byte, _)| byte)
        .unwrap_or(s.len())
}

fn truncate(text: impl AsRef<str>, max_chars: usize) -> String {
    let text = text.as_ref();
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn compact_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| format!("invalid json: {error}"))
}

fn sanitize_display_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch == '\n' || ch == '\t' || !ch.is_control() {
                ch
            } else {
                ' '
            }
        })
        .collect()
}

fn sanitize_one_line(text: &str) -> String {
    sanitize_display_text(text)
        .lines()
        .collect::<Vec<_>>()
        .join(" ")
}

fn friendly_error(error: &str) -> String {
    if error.contains("HTTP 401")
        || error.contains("invalid_api_key")
        || error.contains("Invalid API Key")
    {
        "Invalid API key. Replace the provider key or run `helm init --force`.".to_owned()
    } else if error.contains("HTTP 429") || error.contains("rate_limit") {
        "Rate limited. Wait for the provider reset, switch model, or switch provider.".to_owned()
    } else if error.contains("model") && error.contains("not found") {
        "Model not found. For Ollama run `ollama pull qwen3:4b`, or choose an installed model."
            .to_owned()
    } else {
        error.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn app() -> TuiApp {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("helm.db");
        let memory = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(MemoryStore::open(&db))
                .unwrap(),
        );
        TuiApp::new(TuiRuntime {
            provider_settings: ProviderSettings {
                choice: crate::ProviderChoice::Ollama,
                base_url: Some("http://localhost:11434".to_owned()),
                model: Some("qwen3:4b".to_owned()),
                api_key_env: None,
                source: crate::ProviderSource::Fallback,
            },
            db_path: db,
            memory,
            max_iterations: Some(2),
        })
    }

    fn render_to_buffer(mut app: TuiApp, width: u16, height: u16) -> Buffer {
        app.push_chat(MessageRole::User, "hello from user");
        app.push_chat(
            MessageRole::Assistant,
            "this is a very long assistant output that should wrap cleanly inside the transcript without overflowing into the input panel",
        );
        app.record_tool_event("start", "shell", "echo hello");
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn renders_at_normal_size() {
        let buffer = render_to_buffer(app(), 80, 24);
        assert_eq!(buffer.area.width, 80);
        assert_eq!(buffer.area.height, 24);
    }

    #[test]
    fn renders_at_wide_size() {
        let buffer = render_to_buffer(app(), 120, 40);
        assert_eq!(buffer.area.width, 120);
        assert!(buffer.content().iter().any(|cell| cell.symbol() == "H"));
    }

    #[test]
    fn renders_at_small_size() {
        let buffer = render_to_buffer(app(), 40, 15);
        assert_eq!(buffer.area.width, 40);
        assert_eq!(buffer.area.height, 15);
    }

    #[test]
    fn input_height_grows_and_clamps() {
        assert_eq!(input_height("short", 80), 3);
        assert_eq!(input_height(&"x".repeat(1_000), 80), 7);
    }

    #[test]
    fn command_palette_filters_commands() {
        let mut palette = CommandPaletteState::new();
        palette.query = "doctor".to_owned();
        let filtered = palette.filtered();
        assert_eq!(filtered, vec![CommandAction::Doctor]);
    }

    #[test]
    fn modal_overlays_buffer() {
        let mut app = app();
        app.modal = Some(ModalState::Help);
        let buffer = render_to_buffer(app, 80, 24);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Help"));
    }

    #[test]
    fn input_state_supports_multiline_and_history() {
        let mut input = InputState::new();
        input.insert('h');
        input.insert_newline();
        input.insert('i');
        assert_eq!(input.text, "h\ni");
        assert_eq!(input.take_submit(), Some("h\ni".to_owned()));
        input.previous_history();
        assert_eq!(input.text, "h\ni");
    }

    #[test]
    fn friendly_errors_are_actionable() {
        assert!(friendly_error("provider returned HTTP 401").contains("Invalid API key"));
        assert!(friendly_error("provider returned HTTP 429").contains("Rate limited"));
    }

    #[test]
    fn tool_event_sanitizes_multiline_and_control_text() {
        let mut app = app();
        app.record_tool_event("ok\nbad", "shell\u{0007}", "line 1\nline 2");
        let item = app.session.chat.last().unwrap();
        assert_eq!(item.role, MessageRole::Activity);
        assert_eq!(item.text, "ok bad shell  — line 1 line 2");
    }

    #[test]
    fn routine_provider_events_stay_out_of_transcript() {
        let mut app = app();
        app.record_tool_event("queued", "agent", "task submitted");
        app.record_tool_event("run", "episode", "abc");
        app.record_tool_event("call", "groq", "iteration 0");
        app.record_tool_event("done", "provider", "1 EndTurn, 1539/342 tokens");
        app.record_tool_event("done", "episode", "1 iter(s), 1539 in / 342 out tokens");

        assert!(app.session.chat.is_empty());
        assert_eq!(app.status_note, "ready");
    }

    #[test]
    fn visible_tool_events_are_concise_activity_lines() {
        let mut app = app();
        app.record_tool_event("start", "shell", r#"{"command":"du -sh /home"}"#);
        app.record_tool_event("err", "shell", "tool timed out after a very long command");

        assert_eq!(app.session.chat.len(), 2);
        assert_eq!(app.session.chat[0].role, MessageRole::Activity);
        assert_eq!(app.session.chat[0].text, "running shell");
        assert!(app.session.chat[1].text.contains("shell failed"));
    }

    #[test]
    fn tool_activity_cell_mutates_from_running_to_completed() {
        let mut app = app();
        app.apply_agent_event(AgentEvent::ToolCallParsed {
            id: "call_1".to_owned(),
            name: "shell".to_owned(),
            input: serde_json::json!({
                "mode": "shell",
                "command": "date && uname -a",
                "redirect_stdout_to": "/tmp/helm.txt"
            }),
        });
        app.apply_agent_event(AgentEvent::ToolCallStarted {
            id: "call_1".to_owned(),
            name: "shell".to_owned(),
        });

        assert_eq!(app.session.chat.len(), 1);
        assert_eq!(
            app.session.chat[0].text,
            "running shell: shell `date && uname -a` -> /tmp/helm.txt"
        );

        app.apply_agent_event(AgentEvent::ToolCallFinished {
            id: "call_1".to_owned(),
            name: "shell".to_owned(),
            success: true,
            content: "STDOUT:\nLinux PHANTOM\nSTDERR:\n\n[exit code: 0]".to_owned(),
        });

        assert_eq!(app.session.chat.len(), 1);
        assert_eq!(app.session.chat[0].role, MessageRole::Activity);
        assert!(
            app.session.chat[0]
                .text
                .contains("ran shell: shell `date && uname -a`")
        );
        assert!(app.session.chat[0].text.contains("Linux PHANTOM"));
    }

    #[test]
    fn tool_activity_failure_updates_existing_cell() {
        let mut app = app();
        app.apply_agent_event(AgentEvent::ToolCallParsed {
            id: "call_1".to_owned(),
            name: "fs_read".to_owned(),
            input: serde_json::json!({"path": "/etc/shadow"}),
        });
        app.apply_agent_event(AgentEvent::ToolCallStarted {
            id: "call_1".to_owned(),
            name: "fs_read".to_owned(),
        });
        app.apply_agent_event(AgentEvent::ToolCallFinished {
            id: "call_1".to_owned(),
            name: "fs_read".to_owned(),
            success: false,
            content: "path denied: /etc/shadow".to_owned(),
        });

        assert_eq!(app.session.chat.len(), 1);
        assert_eq!(app.session.chat[0].role, MessageRole::Error);
        assert_eq!(
            app.session.chat[0].text,
            "fs_read failed: read /etc/shadow\npath denied: /etc/shadow"
        );
    }

    #[test]
    fn tool_call_summary_prefers_human_readable_fields() {
        assert_eq!(
            tool_call_summary(
                "browser",
                &serde_json::json!({"action": "open", "url": "https://example.com"})
            ),
            "open https://example.com"
        );
        assert_eq!(
            tool_call_summary(
                "fs_write",
                &serde_json::json!({"path": "/tmp/a", "mode": "append"})
            ),
            "append /tmp/a"
        );
    }

    #[test]
    fn transcript_scrolls_from_bottom() {
        let lines = (0..20)
            .map(|index| Line::from(format!("line {index}")))
            .collect::<Vec<_>>();
        let visible = visible_lines_from_bottom(lines.clone(), 5, 0);
        assert_eq!(visible.first().unwrap().to_string(), "line 15");
        let visible = visible_lines_from_bottom(lines, 5, 5);
        assert_eq!(visible.first().unwrap().to_string(), "line 10");
    }

    #[test]
    fn cancel_running_task_aborts_handle_and_keeps_tui_open() {
        let mut app = app();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        app.running = true;
        app.active_run_id = 10;
        app.agent_task = Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));

        app.cancel_running_task();

        assert!(!app.running);
        assert!(app.agent_task.is_none());
        assert_eq!(app.active_run_id, 11);
        assert!(app.chat_ends_with(MessageRole::Activity, "task cancelled"));
    }

    #[test]
    fn final_assistant_duplicate_is_detected() {
        let mut app = app();
        app.push_chat(MessageRole::Assistant, "done");
        assert!(app.chat_ends_with(MessageRole::Assistant, "done"));
        assert!(!app.chat_ends_with(MessageRole::Assistant, "other"));
    }
}
