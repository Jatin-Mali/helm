//! Full-screen HELM terminal UI built with ratatui.

use std::{
    cell::Cell,
    collections::HashMap,
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use helm_agent::{AgentEvent, AgentEventSink, Budget, ReactAgent, RunResult};
use helm_core::{Capability, HelmError, Message};
use helm_memory::MemoryStore;
use helm_providers::ChatRequest;
use helm_tools::ToolRegistry;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Widget,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph, Wrap},
};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::secrets::SecretsStore;
use crate::{
    ProviderChoice, ProviderSettings, build_provider, default_api_key_env, default_model_name,
    provider_choice_name, write_helm_config,
};

const APP_BG: Color = Color::Rgb(11, 19, 30);
const APP_FG: Color = Color::Rgb(208, 214, 224);
const HEADER_BG: Color = Color::Rgb(26, 41, 64);
const HEADER_BORDER: Color = Color::Rgb(45, 123, 215);
const USER_BG: Color = Color::Rgb(31, 44, 60);
const USER_FG: Color = Color::Rgb(224, 229, 240);
const USER_BAR: Color = Color::Rgb(45, 123, 215);
const ASSISTANT_BG: Color = Color::Rgb(13, 26, 40);
const ASSISTANT_FG: Color = Color::Rgb(200, 210, 224);
const ASSISTANT_BAR: Color = Color::Rgb(74, 144, 217);
const TOOL_BG: Color = Color::Rgb(18, 35, 59);
const TOOL_FG: Color = Color::Rgb(139, 164, 204);
const ERROR_BG: Color = Color::Rgb(31, 17, 21);
const ERROR_FG: Color = Color::Rgb(242, 139, 130);
const ERROR_BAR: Color = Color::Rgb(217, 58, 58);
const INPUT_BG: Color = Color::Rgb(10, 17, 24);
const INPUT_FOCUS: Color = Color::Rgb(45, 123, 215);
const INPUT_IDLE: Color = Color::Rgb(42, 58, 74);
const MODAL_BG: Color = Color::Rgb(19, 34, 53);
const MODAL_FG: Color = Color::Rgb(203, 209, 222);
const DIM_FG: Color = Color::Rgb(103, 119, 139);
const SUCCESS_FG: Color = Color::Rgb(111, 221, 137);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Dim,
    Dracula,
    Solarized,
    #[allow(dead_code)]
    Custom {
        bg: Color,
        fg: Color,
        accent: Color,
    },
}

#[allow(dead_code)]
impl Theme {
    pub fn colors(&self) -> (Color, Color, Color) {
        match self {
            Theme::Dark => (
                Color::Rgb(11, 19, 30),
                Color::Rgb(208, 214, 224),
                Color::Rgb(45, 123, 215),
            ),
            Theme::Light => (
                Color::Rgb(254, 254, 254),
                Color::Rgb(26, 26, 26),
                Color::Rgb(45, 123, 215),
            ),
            Theme::Dim => (
                Color::Rgb(40, 40, 40),
                Color::Rgb(180, 180, 180),
                Color::Rgb(45, 123, 215),
            ),
            Theme::Dracula => (
                Color::Rgb(40, 42, 54),
                Color::Rgb(248, 248, 242),
                Color::Rgb(98, 114, 164),
            ),
            Theme::Solarized => (
                Color::Rgb(0, 43, 54),
                Color::Rgb(131, 148, 150),
                Color::Rgb(181, 137, 0),
            ),
            Theme::Custom { bg, fg, accent } => (*bg, *fg, *accent),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Dim => "dim",
            Theme::Dracula => "dracula",
            Theme::Solarized => "solarized",
            Theme::Custom { .. } => "custom",
        }
    }

    pub fn all() -> Vec<Theme> {
        vec![
            Theme::Dark,
            Theme::Light,
            Theme::Dim,
            Theme::Dracula,
            Theme::Solarized,
        ]
    }
}

/// Runtime dependencies needed by the TUI.
pub(crate) struct TuiRuntime {
    pub(crate) provider_settings: ProviderSettings,
    pub(crate) db_path: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) memory: Arc<MemoryStore>,
    pub(crate) max_iterations: Option<u32>,
    pub(crate) secrets: SecretsStore,
    pub(crate) tui_paste_key_modal: bool,
    pub(crate) auto_approve: bool,
    pub(crate) read_only: bool,
}

/// Starts the interactive terminal UI.
pub(crate) async fn run_tui(runtime: TuiRuntime) -> Result<()> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to initialize terminal")?;
    terminal.clear().ok();

    let result = TuiApp::new(runtime).run(&mut terminal).await;

    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .ok();
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
        task: String,
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
    tool_timeline: Vec<ToolTimelineItem>,
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

#[derive(Debug, Clone, PartialEq)]
enum ModalState {
    CommandPalette,
    Permission {
        capability: Capability,
        tool_name: String,
        taint: String,
        detail: String,
    },
    ProviderSelector {
        selected: usize,
    },
    ModelSelector {
        query: String,
        selected: usize,
    },
    ApiKeyInput {
        choice: ProviderChoice,
        input: String,
    },
    AuthRequired {
        provider_name: String,
        env_name: String,
        input: String,
        error: Option<String>,
    },
    Error(String),
    Help,
    #[allow(dead_code)]
    ThemeSelector {
        selected: usize,
    },
    #[allow(dead_code)]
    CostMeter {
        session_tokens_in: u64,
        session_tokens_out: u64,
        session_cost_usd: f64,
    },
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
struct ModelCatalogEntry {
    group: &'static str,
    label: &'static str,
    provider: ProviderChoice,
    model: &'static str,
    note: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandAction {
    NewSession,
    Clear,
    Replay,
    Doctor,
    Provider,
    Model,
    Permissions,
    Audit,
    Skills,
    Browser,
    Init,
    Sessions,
    Resume,
    Quit,
    Help,
}

impl CommandAction {
    fn all() -> Vec<Self> {
        vec![
            Self::NewSession,
            Self::Clear,
            Self::Replay,
            Self::Doctor,
            Self::Provider,
            Self::Model,
            Self::Permissions,
            Self::Audit,
            Self::Skills,
            Self::Browser,
            Self::Init,
            Self::Sessions,
            Self::Resume,
            Self::Quit,
            Self::Help,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Self::NewSession => "New Session",
            Self::Clear => "Clear Transcript",
            Self::Replay => "Replay Episode",
            Self::Doctor => "Doctor",
            Self::Provider => "Provider Selector",
            Self::Model => "Model Selector",
            Self::Permissions => "Permissions",
            Self::Audit => "Audit Verify",
            Self::Skills => "Skills",
            Self::Browser => "Browser Status",
            Self::Init => "Init AGENTS.md",
            Self::Sessions => "Sessions",
            Self::Resume => "Resume",
            Self::Quit => "Quit",
            Self::Help => "Help",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::NewSession => "new",
            Self::Clear => "clear",
            Self::Replay => "replay",
            Self::Doctor => "doctor",
            Self::Provider => "provider",
            Self::Model => "model",
            Self::Permissions => "permissions",
            Self::Audit => "audit",
            Self::Skills => "skills",
            Self::Browser => "browser",
            Self::Init => "init",
            Self::Sessions => "sessions",
            Self::Resume => "resume",
            Self::Quit => "quit",
            Self::Help => "help",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::NewSession => "clear transcript and start over",
            Self::Clear => "clear the visible transcript",
            Self::Replay => "show replay command for this episode",
            Self::Doctor => "show provider and system diagnostics hint",
            Self::Provider => "switch LLM backend",
            Self::Model => "edit active model id",
            Self::Permissions => "grant or inspect capabilities",
            Self::Audit => "verify audit log chain",
            Self::Skills => "inspect local skill library",
            Self::Browser => "browser automation status",
            Self::Init => "generate AGENTS.md in the current project",
            Self::Sessions => "show session-resume guidance",
            Self::Resume => "show resume guidance",
            Self::Quit => "exit HELM",
            Self::Help => "keyboard shortcuts and commands",
        }
    }

    fn matches_slug(self, slug: &str) -> bool {
        match self {
            Self::Quit => matches!(slug, "quit" | "exit" | "q"),
            Self::NewSession => matches!(slug, "new" | "n"),
            Self::Clear => slug == "clear",
            Self::Replay => slug == "replay",
            Self::Doctor => slug == "doctor",
            Self::Provider => slug == "provider",
            Self::Model => slug == "model",
            Self::Permissions => slug == "permissions",
            Self::Audit => slug == "audit",
            Self::Skills => slug == "skills",
            Self::Browser => matches!(slug, "browser" | "tools"),
            Self::Init => slug == "init",
            Self::Sessions => slug == "sessions",
            Self::Resume => slug == "resume",
            Self::Help => slug == "help",
        }
    }

    fn from_slug(slug: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|action| action.matches_slug(slug))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Chat,
    Plan,
    AutoAccept,
}

impl AgentMode {
    fn next(self) -> Self {
        match self {
            Self::Chat => Self::Plan,
            Self::Plan => Self::AutoAccept,
            Self::AutoAccept => Self::Chat,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Plan => "Plan",
            Self::AutoAccept => "Auto-Accept",
        }
    }
}

pub struct TuiApp {
    runtime: Arc<TuiRuntimeInner>,
    active_settings: ProviderSettings,
    session: SessionState,
    input: InputState,
    focus: PanelFocus,
    modal: Option<ModalState>,
    slash_popup: Option<usize>,
    command_palette: CommandPaletteState,
    running: bool,
    shutdown: bool,
    mode: AgentMode,
    show_sidebar: bool,
    spinner: usize,
    provider_name: String,
    model: String,
    status_note: String,
    pending_tool_summaries: HashMap<String, String>,
    active_tool_cells: HashMap<String, usize>,
    toast: Option<ToastState>,
    last_chat_height: Cell<u16>,
    active_run_id: u64,
    agent_task: Option<JoinHandle<()>>,
    pending_auth_retry: Option<String>,
    session_tokens_in: u32,
    session_tokens_out: u32,
    #[allow(dead_code)]
    theme: Theme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToastState {
    text: String,
    created: Instant,
}

struct TuiRuntimeInner {
    db_path: PathBuf,
    config_path: PathBuf,
    memory: Arc<MemoryStore>,
    max_iterations: Option<u32>,
    secrets: SecretsStore,
    tui_paste_key_modal: bool,
}

impl TuiApp {
    fn new(runtime: TuiRuntime) -> Self {
        let config_path = runtime.config_path.clone();
        let mut active_settings = runtime.provider_settings.clone();

        // Allow env-only sessions without silently importing keys into the
        // persistent secrets store.
        if active_settings.api_key.is_none() {
            if let Some(env_name) = default_api_key_env(active_settings.choice) {
                let in_store = runtime.secrets.get(env_name).ok().flatten().is_some();
                if !in_store {
                    if let Ok(key) = std::env::var(env_name) {
                        active_settings.api_key = Some(key);
                    }
                }
            }
        }

        let provider_name = provider_choice_name(active_settings.choice).to_owned();
        let model = active_settings
            .model
            .clone()
            .unwrap_or_else(|| "auto".to_owned());
        let mode = if runtime.read_only {
            AgentMode::Plan
        } else if runtime.auto_approve {
            AgentMode::AutoAccept
        } else {
            AgentMode::Chat
        };

        Self {
            runtime: Arc::new(TuiRuntimeInner {
                db_path: runtime.db_path,
                config_path,
                memory: runtime.memory,
                max_iterations: runtime.max_iterations,
                secrets: runtime.secrets,
                tui_paste_key_modal: runtime.tui_paste_key_modal,
            }),
            active_settings,
            session: SessionState::default(),
            input: InputState::new(),
            focus: PanelFocus::Input,
            modal: None,
            slash_popup: None,
            command_palette: CommandPaletteState::new(),
            running: false,
            shutdown: false,
            mode,
            show_sidebar: false,
            spinner: 0,
            provider_name,
            model,
            status_note: "ready".to_owned(),
            pending_tool_summaries: HashMap::new(),
            active_tool_cells: HashMap::new(),
            toast: None,
            last_chat_height: Cell::new(10),
            active_run_id: 0,
            agent_task: None,
            pending_auth_retry: None,
            session_tokens_in: 0,
            session_tokens_out: 0,
            theme: Theme::default(),
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
            UiEvent::Input(Event::Mouse(mouse)) => {
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        let step = usize::from(self.last_chat_height.get().max(6) / 3);
                        self.session.transcript_scroll =
                            self.session.transcript_scroll.saturating_add(step.max(1));
                    }
                    MouseEventKind::ScrollDown => {
                        let step = usize::from(self.last_chat_height.get().max(6) / 3);
                        self.session.transcript_scroll =
                            self.session.transcript_scroll.saturating_sub(step.max(1));
                    }
                    _ => {}
                }
                Ok(false)
            }
            UiEvent::Input(Event::Resize(_, _)) => Ok(false),
            UiEvent::Input(_) => Ok(false),
            UiEvent::Tick => {
                self.spinner = self.spinner.wrapping_add(1);
                if self
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.created.elapsed() > Duration::from_secs(2))
                {
                    self.toast = None;
                }
                Ok(false)
            }
            UiEvent::Agent { run_id, event } => {
                if run_id == self.active_run_id {
                    self.apply_agent_event(event);
                }
                Ok(false)
            }
            UiEvent::AgentDone {
                run_id,
                task,
                result,
            } => {
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
                            let redacted = helm_core::redact_secrets(&final_text);
                            self.push_chat(MessageRole::Assistant, redacted);
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
                        let msg = error.to_string();
                        self.push_chat(MessageRole::Error, friendly_error(&msg));
                        if is_auth_error(&msg) && self.runtime.tui_paste_key_modal {
                            let env_name = default_api_key_env(self.active_settings.choice)
                                .unwrap_or("API_KEY")
                                .to_owned();
                            self.pending_auth_retry = Some(task);
                            self.modal = Some(ModalState::AuthRequired {
                                provider_name: self.provider_name.clone(),
                                env_name,
                                input: String::new(),
                                error: None,
                            });
                        } else {
                            self.modal = Some(ModalState::Error(friendly_error(&msg)));
                        }
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
            return self.handle_modal_key(key, tx).await;
        }

        // Slash popup navigation (intercept before normal input handling)
        if self.slash_popup.is_some() && !key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Esc => {
                    self.slash_popup = None;
                    return Ok(false);
                }
                KeyCode::Up => {
                    if let Some(sel) = &mut self.slash_popup {
                        *sel = sel.saturating_sub(1);
                    }
                    return Ok(false);
                }
                KeyCode::Down => {
                    let max = self.slash_filtered().len().saturating_sub(1);
                    if let Some(sel) = &mut self.slash_popup {
                        *sel = (*sel + 1).min(max);
                    }
                    return Ok(false);
                }
                KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => {
                    self.execute_slash_from_popup();
                    return Ok(self.shutdown);
                }
                KeyCode::Tab => {
                    let filtered = self.slash_filtered();
                    if let Some(sel) = self.slash_popup {
                        if let Some(cmd) = filtered.get(sel) {
                            self.input.text = format!("/{}", cmd.slug());
                            self.input.cursor = self.input.text.chars().count();
                            self.update_slash_popup();
                        }
                    }
                    return Ok(false);
                }
                _ => {}
            }
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
                KeyCode::Char('d') if self.input.text.is_empty() => return Ok(true),
                KeyCode::Char('j') => self.input.insert_newline(),
                KeyCode::Char('n') => self.new_session(),
                KeyCode::Char('p') => self.open_palette(),
                KeyCode::Char('r') => self.push_chat(MessageRole::System, self.replay_hint()),
                KeyCode::Char('t') => {
                    self.show_sidebar = !self.show_sidebar;
                    self.toast(if self.show_sidebar {
                        "Sidebar visible"
                    } else {
                        "Sidebar hidden"
                    });
                }
                KeyCode::Char('a') => {
                    self.modal = Some(ModalState::Permission {
                        capability: Capability::ShellShell,
                        tool_name: "pending".to_owned(),
                        taint: "user".to_owned(),
                        detail: "No pending permission request.".to_owned(),
                    });
                }
                KeyCode::Char('h') => {
                    self.modal = Some(ModalState::Help);
                }
                KeyCode::Char('l') => self.clear_transcript(),
                KeyCode::Home => self.session.transcript_scroll = usize::MAX / 2,
                KeyCode::End => self.session.transcript_scroll = 0,
                KeyCode::Char('u') => self.input.clear(),
                _ => {}
            }
            return Ok(false);
        }

        match key.code {
            KeyCode::Enter
                if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.input.insert_newline()
            }
            KeyCode::Enter => {
                self.submit(tx).await?;
                if self.shutdown {
                    return Ok(true);
                }
            }
            KeyCode::Char('?') if self.input.text.is_empty() => {
                self.modal = Some(ModalState::Help);
            }
            KeyCode::Char(ch) => {
                self.input.insert(ch);
                self.update_slash_popup();
            }
            KeyCode::Backspace => {
                self.input.backspace();
                self.update_slash_popup();
            }
            KeyCode::Delete => {
                self.input.delete();
                self.update_slash_popup();
            }
            KeyCode::Left => self.input.cursor = self.input.cursor.saturating_sub(1),
            KeyCode::Right => {
                self.input.cursor = (self.input.cursor + 1).min(self.input.text.chars().count());
            }
            KeyCode::Home => self.input.cursor = 0,
            KeyCode::End => self.input.cursor = self.input.text.chars().count(),
            KeyCode::Up => self.input.previous_history(),
            KeyCode::Down => self.input.next_history(),
            KeyCode::PageUp => {
                let step = usize::from(self.last_chat_height.get().max(6) / 2);
                self.session.transcript_scroll =
                    self.session.transcript_scroll.saturating_add(step.max(1));
            }
            KeyCode::PageDown => {
                let step = usize::from(self.last_chat_height.get().max(6) / 2);
                self.session.transcript_scroll =
                    self.session.transcript_scroll.saturating_sub(step.max(1));
            }
            KeyCode::Tab => self.focus = PanelFocus::Input,
            KeyCode::BackTab => {
                self.mode = self.mode.next();
                self.toast(format!("Mode changed to {}", self.mode.as_str()));
            }
            KeyCode::Esc => self.focus = PanelFocus::Input,
            _ => {}
        }
        Ok(false)
    }

    async fn handle_modal_key(
        &mut self,
        key: KeyEvent,
        tx: mpsc::UnboundedSender<UiEvent>,
    ) -> Result<bool> {
        match self.modal.clone() {
            Some(ModalState::CommandPalette) => match key.code {
                KeyCode::Esc => self.modal = None,
                KeyCode::Enter => {
                    let commands = self.command_palette.filtered();
                    if let Some(command) = commands.get(self.command_palette.selected).copied() {
                        self.execute_command(command);
                        return Ok(self.shutdown);
                    }
                }
                KeyCode::Char(ch) => {
                    self.command_palette.query.push(ch);
                    self.command_palette.selected = 0;
                }
                KeyCode::Backspace => {
                    self.command_palette.query.pop();
                    self.command_palette.selected = 0;
                }
                KeyCode::Up => {
                    self.command_palette.selected = self.command_palette.selected.saturating_sub(1)
                }
                KeyCode::Down => {
                    let max = self.command_palette.filtered().len().saturating_sub(1);
                    self.command_palette.selected = (self.command_palette.selected + 1).min(max);
                }
                _ => {}
            },
            Some(ModalState::Permission { capability, .. }) => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.push_chat(MessageRole::System, "Permission denied.");
                    self.toast("Permission denied");
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
                    self.toast("Permission granted once");
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
                    self.toast("Permission granted for session");
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
                    self.toast("Permission granted always");
                    self.modal = None;
                }
                _ => {}
            },
            Some(ModalState::ProviderSelector { .. }) => match key.code {
                KeyCode::Esc => self.modal = None,
                KeyCode::Char(d @ '1'..='7') => {
                    let choices = provider_selector_list();
                    let idx = (d as usize) - ('1' as usize);
                    if let Some((choice, _)) = choices.get(idx) {
                        self.apply_provider_choice(*choice);
                    }
                }
                KeyCode::Up => {
                    if let Some(ModalState::ProviderSelector { selected }) = &mut self.modal {
                        *selected = selected.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if let Some(ModalState::ProviderSelector { selected }) = &mut self.modal {
                        let max = provider_selector_list().len().saturating_sub(1);
                        *selected = (*selected + 1).min(max);
                    }
                }
                KeyCode::Enter => {
                    if let Some(ModalState::ProviderSelector { selected }) = self.modal.clone() {
                        let choices = provider_selector_list();
                        if let Some((choice, _)) = choices.get(selected) {
                            self.apply_provider_choice(*choice);
                        } else {
                            self.modal = None;
                        }
                    }
                }
                _ => {}
            },
            Some(ModalState::ModelSelector { .. }) => match key.code {
                KeyCode::Esc => self.modal = None,
                KeyCode::Enter => {
                    if let Some(ModalState::ModelSelector { query, selected }) = self.modal.clone()
                    {
                        let entries = filtered_model_catalog(&query);
                        if let Some(entry) = entries.get(selected).copied() {
                            self.apply_model_entry(entry);
                        } else if !query.trim().is_empty() {
                            self.apply_manual_model(query.trim().to_owned());
                        }
                    }
                }
                KeyCode::Up => {
                    if let Some(ModalState::ModelSelector { selected, .. }) = &mut self.modal {
                        *selected = selected.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if let Some(ModalState::ModelSelector { query, selected }) = &mut self.modal {
                        let max = filtered_model_catalog(query).len().saturating_sub(1);
                        *selected = (*selected + 1).min(max);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(ModalState::ModelSelector { query, selected }) = &mut self.modal {
                        query.pop();
                        *selected = 0;
                    }
                }
                KeyCode::Char(ch) => {
                    if let Some(ModalState::ModelSelector { query, selected }) = &mut self.modal {
                        query.push(ch);
                        *selected = 0;
                    }
                }
                _ => {}
            },
            Some(ModalState::ApiKeyInput { .. }) => match key.code {
                KeyCode::Esc => self.modal = None,
                KeyCode::Enter => {
                    if let Some(ModalState::ApiKeyInput { choice, input }) = self.modal.clone() {
                        let key = input.trim().to_owned();
                        if key.is_empty() {
                            self.push_chat(
                                MessageRole::System,
                                "API key cannot be empty. Press Esc to cancel.",
                            );
                        } else {
                            self.apply_provider_with_key(choice, key, true);
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Some(ModalState::ApiKeyInput { input, .. }) = &mut self.modal {
                        input.pop();
                    }
                }
                KeyCode::Char(ch) => {
                    if let Some(ModalState::ApiKeyInput { input, .. }) = &mut self.modal {
                        input.push(ch);
                    }
                }
                _ => {}
            },
            Some(ModalState::AuthRequired { .. }) => match key.code {
                KeyCode::Esc => {
                    self.pending_auth_retry = None;
                    self.modal = None;
                }
                KeyCode::Enter => {
                    if let Some(ModalState::AuthRequired {
                        provider_name,
                        env_name,
                        ..
                    }) = self.modal.clone()
                    {
                        let key_val = match &mut self.modal {
                            Some(ModalState::AuthRequired { input, .. }) => {
                                let value = input.trim().to_owned();
                                input.clear();
                                value
                            }
                            _ => String::new(),
                        };
                        if key_val.is_empty() {
                            if let Some(ModalState::AuthRequired { error, .. }) = &mut self.modal {
                                *error = Some("API key cannot be empty.".to_owned());
                            }
                        } else {
                            self.status_note = "validating key".to_owned();
                            match self.validate_key(&key_val).await {
                                Ok(()) => {
                                    let secret = helm_core::Secret::new(key_val.clone());
                                    if let Err(e) = self.runtime.secrets.set(&env_name, secret) {
                                        if let Some(ModalState::AuthRequired { error, .. }) =
                                            &mut self.modal
                                        {
                                            *error = Some(format!("Failed to save key: {e}"));
                                        }
                                    } else {
                                        self.active_settings.api_key = Some(key_val);
                                        self.push_chat(
                                            MessageRole::System,
                                            format!(
                                                "API key saved for {provider_name}. Retrying the task."
                                            ),
                                        );
                                        self.modal = None;
                                        if let Some(task) = self.pending_auth_retry.take() {
                                            self.start_task(task, tx, false).await?;
                                        }
                                    }
                                }
                                Err(error_text) => {
                                    if let Some(ModalState::AuthRequired { error, .. }) =
                                        &mut self.modal
                                    {
                                        *error = Some(error_text);
                                    }
                                }
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Some(ModalState::AuthRequired { input, .. }) = &mut self.modal {
                        input.pop();
                    }
                }
                KeyCode::Char(ch) => {
                    if let Some(ModalState::AuthRequired { input, .. }) = &mut self.modal {
                        input.push(ch);
                    }
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
        if let Some(command_text) = task.trim().strip_prefix('/') {
            let slug = command_text.split_whitespace().next().unwrap_or("");
            if let Some(command) = CommandAction::from_slug(slug) {
                self.execute_command(command);
            } else {
                self.push_chat(
                    MessageRole::Error,
                    format!("Unknown command `{task}`. Type /help or press Ctrl+P."),
                );
            }
            return Ok(());
        }
        self.start_task(task, tx, true).await
    }

    async fn start_task(
        &mut self,
        task: String,
        tx: mpsc::UnboundedSender<UiEvent>,
        echo_user: bool,
    ) -> Result<()> {
        if self.running {
            return Ok(());
        }
        if echo_user {
            self.push_chat(MessageRole::User, task.clone());
        }
        self.record_tool_event("queued", "agent", "task submitted");
        self.running = true;
        self.status_note = "running".to_owned();
        self.active_run_id = self.active_run_id.saturating_add(1);
        self.session.transcript_scroll = 0;
        let run_id = self.active_run_id;

        let runtime = Arc::clone(&self.runtime);
        let settings = self.active_settings.clone();
        let task_for_event = task.clone();
        let mode = self.mode;
        self.agent_task = Some(tokio::spawn(async move {
            let result = run_agent_task(runtime, settings, task, tx.clone(), run_id, mode).await;
            tx.send(UiEvent::AgentDone {
                run_id,
                task: task_for_event,
                result,
            })
            .ok();
        }));

        Ok(())
    }

    async fn validate_key(&self, key: &str) -> Result<(), String> {
        let mut settings = self.active_settings.clone();
        settings.api_key = Some(key.to_owned());
        let (provider, model) =
            build_provider(&settings, &self.runtime.secrets).map_err(|error| error.to_string())?;
        let request = ChatRequest {
            model,
            system: None,
            messages: vec![Message::user("Reply ok.")],
            tools: vec![],
            max_tokens: 1,
            temperature: 0.0,
        };
        provider.chat(request).await.map(|_| ()).map_err(|error| {
            let text = error.to_string();
            if is_auth_error(&text) {
                "Validation failed: invalid API key.".to_owned()
            } else {
                format!("Validation failed: {}", truncate(text, 180))
            }
        })
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
            } => {
                self.session_tokens_in = self.session_tokens_in.saturating_add(tokens_in);
                self.session_tokens_out = self.session_tokens_out.saturating_add(tokens_out);
                self.record_tool_event(
                    "done",
                    "provider",
                    format!("{iteration:?} {stop_reason:?}, {tokens_in}/{tokens_out} tokens"),
                );
            }
            AgentEvent::AssistantText { text } => {
                if !text.trim().is_empty() {
                    let redacted = helm_core::redact_secrets(&text);
                    self.push_chat(MessageRole::Assistant, redacted);
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
                let redacted = helm_core::redact_secrets(&content);
                self.finish_tool_cell(&id, &name, success, &redacted);
            }
            AgentEvent::ToolCallDenied { name, reason, .. } => {
                let redacted = helm_core::redact_secrets(&reason);
                self.record_tool_event("deny", name, redacted.clone());
                self.push_chat(MessageRole::Error, redacted);
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
            AgentEvent::RunFinished { .. }
            | AgentEvent::RunFailed { .. }
            | AgentEvent::TextDelta { .. }
            | AgentEvent::PlanCacheHit { .. } => {}
        }
    }

    fn push_chat(&mut self, role: MessageRole, text: impl Into<String>) {
        self.session.chat.push(ChatMessage {
            role,
            text: sanitize_display_text(&text.into()),
        });
        self.session.transcript_scroll = 0;
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(ToastState {
            text: sanitize_one_line(&text.into()),
            created: Instant::now(),
        });
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
        self.session.tool_timeline.push(item.clone());
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
        let text = format!("{name}: {summary} ...");
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
                format!("{name}: {summary}")
            } else {
                format!("{name}: {summary}\n{preview}")
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
        self.session_tokens_in = 0;
        self.session_tokens_out = 0;
        self.input.clear();
        self.push_chat(MessageRole::System, "New session started.");
        self.modal = None;
    }

    fn clear_transcript(&mut self) {
        self.session.chat.clear();
        self.session.transcript_scroll = 0;
        self.push_chat(MessageRole::System, "Transcript cleared.");
    }

    fn apply_provider_choice(&mut self, choice: ProviderChoice) {
        // Check env var for this provider (don't carry over key from previous provider)
        let env_key = default_api_key_env(choice).and_then(|env_name| std::env::var(env_name).ok());

        if choice == ProviderChoice::Ollama || env_key.is_some() {
            let key = env_key.as_deref();
            self.apply_provider_with_key(choice, key.unwrap_or("").to_owned(), false);
        } else {
            // No key available — prompt the user
            let mut s = self.active_settings.with_choice(choice);
            s.api_key_env = default_api_key_env(choice).map(str::to_owned);
            s.api_key = None;
            s.model = None;
            self.active_settings = s;
            self.modal = Some(ModalState::ApiKeyInput {
                choice,
                input: String::new(),
            });
        }
    }

    fn apply_provider_with_key(&mut self, choice: ProviderChoice, key: String, persist_key: bool) {
        if persist_key {
            if let Some(env_name) = default_api_key_env(choice) {
                if let Err(error) = self
                    .runtime
                    .secrets
                    .set(env_name, helm_core::Secret::new(key.clone()))
                {
                    self.push_chat(
                        MessageRole::Error,
                        format!("failed to save key to secrets store: {error}"),
                    );
                }
            }
        }
        let mut s = self.active_settings.with_choice(choice);
        s.api_key_env = default_api_key_env(choice).map(str::to_owned);
        s.api_key = if key.is_empty() {
            None
        } else {
            Some(key.clone())
        };
        s.model = None;
        self.active_settings = s;
        self.provider_name = provider_choice_name(choice).to_owned();
        self.model = "auto".to_owned();
        self.modal = None;
        self.push_chat(
            MessageRole::System,
            format!(
                "Switched to {}. Type a task to begin.",
                provider_choice_name(choice)
            ),
        );
        self.save_provider_to_config(choice);
    }

    fn apply_model_entry(&mut self, entry: ModelCatalogEntry) {
        let mut settings = self.active_settings.with_choice(entry.provider);
        settings.api_key_env = default_api_key_env(entry.provider).map(str::to_owned);
        settings.model = Some(entry.model.to_owned());
        if settings.choice != self.active_settings.choice {
            settings.api_key = None;
        }
        self.active_settings = settings;
        self.provider_name = provider_choice_name(entry.provider).to_owned();
        self.model = entry.model.to_owned();
        self.modal = None;
        self.push_chat(
            MessageRole::System,
            format!(
                "Model set to {} ({}) via {}. Type a task to begin.",
                entry.label, entry.model, self.provider_name
            ),
        );
        self.save_provider_to_config(entry.provider);
    }

    fn apply_manual_model(&mut self, model: String) {
        self.active_settings.model = Some(model.clone());
        self.model = model.clone();
        self.modal = None;
        self.push_chat(
            MessageRole::System,
            format!("Model set to {model}. Type a task to begin."),
        );
        self.save_provider_to_config(self.active_settings.choice);
    }

    fn save_provider_to_config(&self, choice: ProviderChoice) {
        let model = self
            .active_settings
            .model
            .as_deref()
            .unwrap_or_else(|| default_model_name(choice));
        let _ = write_helm_config(
            &self.runtime.config_path,
            &self.runtime.db_path,
            provider_choice_name(choice),
            model,
            self.active_settings.base_url.as_deref(),
            default_api_key_env(choice),
        );
    }

    fn slash_filtered(&self) -> Vec<CommandAction> {
        let raw = self.input.text.trim_start_matches('/').to_ascii_lowercase();
        let query = raw.split_whitespace().next().unwrap_or("");
        CommandAction::all()
            .into_iter()
            .filter(|a| a.slug().starts_with(query) || a.matches_slug(query))
            .collect()
    }

    fn update_slash_popup(&mut self) {
        if self.input.text.starts_with('/') {
            let filtered = self.slash_filtered();
            if filtered.is_empty() {
                self.slash_popup = None;
            } else {
                let max = filtered.len().saturating_sub(1);
                self.slash_popup = Some(self.slash_popup.unwrap_or(0).min(max));
            }
        } else {
            self.slash_popup = None;
        }
    }

    fn execute_slash_from_popup(&mut self) {
        let filtered = self.slash_filtered();
        if let Some(sel) = self.slash_popup {
            if let Some(cmd) = filtered.get(sel).copied() {
                self.input.clear();
                self.slash_popup = None;
                self.execute_command(cmd);
            }
        }
    }

    fn open_palette(&mut self) {
        self.command_palette = CommandPaletteState::new();
        self.modal = Some(ModalState::CommandPalette);
    }

    fn execute_command(&mut self, command: CommandAction) {
        self.modal = None;
        match command {
            CommandAction::NewSession => self.new_session(),
            CommandAction::Clear => self.clear_transcript(),
            CommandAction::Replay => self.push_chat(MessageRole::System, self.replay_hint()),
            CommandAction::Doctor => self.push_chat(MessageRole::System, self.doctor_hint()),
            CommandAction::Provider => {
                let current = provider_selector_list()
                    .iter()
                    .position(|(c, _)| *c == self.active_settings.choice)
                    .unwrap_or(0);
                self.modal = Some(ModalState::ProviderSelector { selected: current });
            }
            CommandAction::Model => {
                self.modal = Some(ModalState::ModelSelector {
                    query: String::new(),
                    selected: model_catalog()
                        .iter()
                        .position(|entry| {
                            entry.provider == self.active_settings.choice
                                && entry.model == self.model.as_str()
                        })
                        .unwrap_or(0),
                });
            }
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
            CommandAction::Init => match crate::generate_agents_md_for_dir(
                &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            ) {
                Ok(Some(path)) => self.push_chat(
                    MessageRole::System,
                    format!("Generated {}.", path.display()),
                ),
                Ok(None) => self.push_chat(
                    MessageRole::System,
                    "Current directory does not look like a project; no AGENTS.md generated.",
                ),
                Err(error) => self.push_chat(
                    MessageRole::Error,
                    format!("failed to generate AGENTS.md: {error}"),
                ),
            },
            CommandAction::Sessions => self.push_chat(
                MessageRole::System,
                "Sessions are not resumable yet. Use `helm episodes` and `helm replay <id>`.",
            ),
            CommandAction::Resume => self.push_chat(
                MessageRole::System,
                "Resume is not implemented yet. Use `helm episodes` and `helm replay <id>`.",
            ),
            CommandAction::Quit => self.shutdown = true,
            CommandAction::Help => self.modal = Some(ModalState::Help),
        }
    }

    fn render(&self, frame: &mut Frame<'_>) {
        render_app(self, frame.area(), frame.buffer_mut());
    }
}

async fn run_agent_task(
    runtime: Arc<TuiRuntimeInner>,
    settings: ProviderSettings,
    task: String,
    tx: mpsc::UnboundedSender<UiEvent>,
    run_id: u64,
    mode: AgentMode,
) -> Result<RunResult, HelmError> {
    let (provider, model) = build_provider(&settings, &runtime.secrets)
        .map_err(|error| HelmError::Provider(helm_core::ProviderError::Other(error.to_string())))?;
    let mut budget = Budget::default();
    if let Some(max) = runtime.max_iterations {
        budget.max_iterations = max;
    }
    budget.read_only = mode == AgentMode::Plan;
    budget.auto_approve = mode == AgentMode::AutoAccept;
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

    Block::default()
        .style(Style::default().bg(APP_BG).fg(APP_FG))
        .render(area, buf);

    let (main_area, sidebar_area) = if app.show_sidebar && area.width > 60 {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(30)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(input_height(&app.input.text, main_area.width)),
            Constraint::Length(1),
        ])
        .split(main_area);

    render_status(app, vertical[0], buf);
    render_chat(app, vertical[1], buf);
    render_input(app, vertical[2], buf);
    render_footer(app, vertical[3], buf);

    if let Some(sidebar_rect) = sidebar_area {
        render_sidebar(app, sidebar_rect, buf);
    }

    if app.slash_popup.is_some() && app.modal.is_none() {
        render_slash_popup(app, vertical[2], buf);
    }

    if let Some(toast) = &app.toast {
        render_toast(toast, area, buf);
    }

    if let Some(modal) = &app.modal {
        render_dim_overlay(area, buf);
        render_modal(app, modal, centered_rect(72, 52, area), buf);
    }
}

fn render_sidebar(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(HEADER_BORDER))
        .title(Span::styled(
            " Tool History ",
            Style::default().fg(DIM_FG).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    block.render(area, buf);

    let items: Vec<ListItem> = app
        .session
        .tool_timeline
        .iter()
        .rev()
        .map(|item| {
            let color = match item.status.as_str() {
                "queued" => DIM_FG,
                "starting" => TOOL_FG,
                "done" | "ok" => SUCCESS_FG,
                "failed" | "denied" => ERROR_FG,
                _ => DIM_FG,
            };
            let header = Line::from(vec![
                Span::styled(
                    format!("[{}] ", item.status),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(&item.tool, Style::default().fg(APP_FG)),
            ]);
            let detail = Line::styled(format!("  {}", item.detail), Style::default().fg(DIM_FG));
            ListItem::new(vec![header, detail])
        })
        .collect();

    let list = ratatui::widgets::List::new(items);
    list.render(inner, buf);
}

fn render_status(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    let spinner = if app.running {
        ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"][app.spinner % 8]
    } else {
        "H"
    };
    let episode = app.session.episode_id.as_deref().unwrap_or("-");
    let mode_style = match app.mode {
        AgentMode::Chat => Style::default().fg(Color::White).bg(HEADER_BORDER),
        AgentMode::Plan => Style::default().fg(Color::White).bg(INPUT_IDLE),
        AgentMode::AutoAccept => Style::default().fg(Color::White).bg(SUCCESS_FG),
    };
    let line = Line::from(vec![
        Span::styled(
            format!(" {spinner} HELM "),
            Style::default()
                .fg(Color::White)
                .bg(HEADER_BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} / {} ", app.provider_name, truncate(&app.model, 42)),
            Style::default().fg(APP_FG).bg(HEADER_BG),
        ),
        Span::styled(
            format!(" episode {} ", truncate(episode, 8)),
            Style::default().fg(DIM_FG).bg(HEADER_BG),
        ),
        Span::styled(
            format!(" [{}] ", app.mode.as_str().to_ascii_uppercase()),
            mode_style.add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", token_status(app)),
            Style::default().fg(DIM_FG).bg(HEADER_BG),
        ),
        Span::styled(
            format!(" {} ", truncate(&app.status_note, 36)),
            Style::default()
                .fg(if app.running { SUCCESS_FG } else { APP_FG })
                .bg(HEADER_BG),
        ),
    ]);
    Paragraph::new(line)
        .style(Style::default().bg(HEADER_BG))
        .render(chunks[0], buf);
    Paragraph::new("─".repeat(chunks[1].width as usize))
        .style(Style::default().fg(HEADER_BORDER).bg(APP_BG))
        .render(chunks[1], buf);
}

fn render_chat(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let lines = chat_lines(&app.session.chat);
    let viewport_height = area.height.saturating_sub(2);
    app.last_chat_height.set(viewport_height);
    let (top_offset, scroll_from_bottom, max_scroll) =
        transcript_scroll_offsets(lines.len(), viewport_height, app.session.transcript_scroll);
    let title = if scroll_from_bottom > 0 {
        format!("Transcript  ↑ {} lines newer below", scroll_from_bottom)
    } else if max_scroll > 0 {
        "Transcript  at latest".to_owned()
    } else {
        "Transcript".to_owned()
    };
    Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT)
                .title(title)
                .border_style(Style::default().fg(INPUT_IDLE))
                .style(Style::default().fg(APP_FG).bg(APP_BG)),
        )
        .scroll((top_offset.min(u16::MAX as usize) as u16, 0))
        .wrap(Wrap { trim: false })
        .render(area, buf);

    if max_scroll > 0 && top_offset > 0 {
        let indicator = Rect {
            x: area.x.saturating_add(2),
            y: area.y,
            width: 14.min(area.width.saturating_sub(4)),
            height: 1,
        };
        Paragraph::new("↑ earlier")
            .style(Style::default().fg(DIM_FG).bg(APP_BG))
            .render(indicator, buf);
    }
    if scroll_from_bottom > 0 {
        let indicator = Rect {
            x: area.x.saturating_add(2),
            y: area.y.saturating_add(area.height.saturating_sub(1)),
            width: 12.min(area.width.saturating_sub(4)),
            height: 1,
        };
        Paragraph::new("↓ latest")
            .style(Style::default().fg(HEADER_BORDER).bg(APP_BG))
            .render(indicator, buf);
    }
}

fn render_slash_popup(app: &TuiApp, input_area: Rect, buf: &mut Buffer) {
    let filtered = {
        let raw = app.input.text.trim_start_matches('/').to_ascii_lowercase();
        let query = raw.split_whitespace().next().unwrap_or("").to_owned();
        CommandAction::all()
            .into_iter()
            .filter(|a| a.slug().starts_with(query.as_str()))
            .collect::<Vec<_>>()
    };
    if filtered.is_empty() {
        return;
    }
    let selected = app.slash_popup.unwrap_or(0);
    let popup_h = (filtered.len() as u16 + 2).min(input_area.y.saturating_sub(1).max(4));
    let popup_w = 58_u16.min(input_area.width.saturating_sub(2));
    let popup_rect = Rect {
        x: input_area.x + 1,
        y: input_area.y.saturating_sub(popup_h),
        width: popup_w,
        height: popup_h,
    };
    let lines: Vec<Line> = filtered
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let style = if i == selected {
                Style::default()
                    .fg(Color::White)
                    .bg(HEADER_BORDER)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(APP_FG).bg(MODAL_BG)
            };
            Line::styled(
                format!(
                    " /{:<11} {:<18} {}",
                    cmd.slug(),
                    cmd.label(),
                    truncate(cmd.description(), 22)
                ),
                style,
            )
        })
        .collect();
    Clear.render(popup_rect, buf);
    Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(HEADER_BORDER))
                .style(Style::default().fg(MODAL_FG).bg(MODAL_BG))
                .title("Commands  ↑↓ Tab Enter Esc"),
        )
        .render(popup_rect, buf);
}

fn render_input(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let counter = format!("{} chars  Enter to send", app.input.text.chars().count());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Prompt ")
        .title_bottom(Line::styled(
            counter,
            Style::default().fg(DIM_FG).bg(INPUT_BG),
        ))
        .border_style(Style::default().fg(match app.focus {
            PanelFocus::Input => INPUT_FOCUS,
        }))
        .style(Style::default().fg(Color::White).bg(INPUT_BG));
    let body = if app.input.text.is_empty() {
        vec![Line::from(vec![
            Span::styled(
                "❯ ",
                Style::default().fg(USER_BAR).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Ask HELM to do something...",
                Style::default().fg(DIM_FG).add_modifier(Modifier::ITALIC),
            ),
        ])]
    } else {
        app.input
            .text
            .lines()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::styled(
                            "❯ ",
                            Style::default().fg(USER_BAR).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(line.to_owned()),
                    ])
                } else {
                    Line::from(vec![Span::raw("  "), Span::raw(line.to_owned())])
                }
            })
            .collect()
    };
    let paragraph = Paragraph::new(body).block(block).wrap(Wrap { trim: false });
    paragraph.render(area, buf);
}

fn render_footer(_app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let mode_label = match _app.mode {
        AgentMode::Chat => "CHAT",
        AgentMode::Plan => "PLAN",
        AgentMode::AutoAccept => "AUTO",
    };
    let mode_hint = match _app.mode {
        AgentMode::Chat => "Shift+Tab -> Plan",
        AgentMode::Plan => "READ-ONLY | Shift+Tab -> Auto",
        AgentMode::AutoAccept => "AUTO-ACCEPT | Shift+Tab -> Chat",
    };
    let line = Line::from(vec![
        Span::styled(
            format!(" [{mode_label}] "),
            Style::default()
                .fg(Color::White)
                .bg(HEADER_BORDER)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ^P Palette ", Style::default().fg(DIM_FG).bg(APP_BG)),
        Span::styled("|", Style::default().fg(INPUT_IDLE).bg(APP_BG)),
        Span::styled(" ^C Cancel ", Style::default().fg(DIM_FG).bg(APP_BG)),
        Span::styled("|", Style::default().fg(INPUT_IDLE).bg(APP_BG)),
        Span::styled(" ^L Clear ", Style::default().fg(DIM_FG).bg(APP_BG)),
        Span::styled("|", Style::default().fg(INPUT_IDLE).bg(APP_BG)),
        Span::styled(" / Commands ", Style::default().fg(DIM_FG).bg(APP_BG)),
        Span::styled("|", Style::default().fg(INPUT_IDLE).bg(APP_BG)),
        Span::styled(
            format!(" {mode_hint} "),
            Style::default().fg(DIM_FG).bg(APP_BG),
        ),
    ]);
    Paragraph::new(line)
        .style(Style::default().fg(DIM_FG).bg(APP_BG))
        .render(area, buf);
}

fn token_status(app: &TuiApp) -> String {
    let estimate = estimated_cost(
        app.active_settings.choice,
        app.session_tokens_in,
        app.session_tokens_out,
    );
    match estimate {
        Some(cost) => format!(
            "{} in / {} out | ${cost:.4}",
            app.session_tokens_in, app.session_tokens_out
        ),
        None => format!(
            "{} in / {} out",
            app.session_tokens_in, app.session_tokens_out
        ),
    }
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
                modal_kv("tool", tool_name),
                modal_kv("capability", &capability.to_string()),
                modal_kv("taint", taint),
                Line::from(""),
                Line::styled(detail.as_str(), Style::default().fg(MODAL_FG)),
                Line::from(""),
                Line::from(vec![
                    key_span("[Y] Once"),
                    Span::raw("  "),
                    key_span("[S] Session"),
                    Span::raw("  "),
                    key_span("[A] Always"),
                    Span::raw("  "),
                    Span::styled("[N/Esc] Deny", Style::default().fg(ERROR_FG)),
                ]),
            ];
            Paragraph::new(text)
                .block(modal_block(" Permission Required "))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::ProviderSelector { selected } => {
            let choices = provider_selector_list();
            let mut lines = vec![
                Line::from("Press 1-7 or Up/Down+Enter to switch provider. Esc to cancel."),
                Line::from(""),
            ];
            for (i, (choice, env_key)) in choices.iter().enumerate() {
                let name = provider_choice_name(*choice);
                let key_status = match env_key {
                    Some(k) => {
                        if std::env::var(k).is_ok() {
                            format!("{k} ✓")
                        } else {
                            format!("{k} (unset)")
                        }
                    }
                    None => "no key needed".to_owned(),
                };
                let label = format!("[{}] {:<16} {}", i + 1, name, key_status);
                let style = if i == *selected {
                    Style::default()
                        .fg(Color::White)
                        .bg(HEADER_BORDER)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(MODAL_FG).bg(MODAL_BG)
                };
                lines.push(Line::styled(label, style));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(
                "Tip: set the env key then press the number. Run `helm init --force` to persist.",
            ));
            Paragraph::new(lines)
                .block(modal_block(" Provider Selector "))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::ModelSelector { query, selected } => {
            let entries = filtered_model_catalog(query);
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Search ", Style::default().fg(Color::Gray)),
                    Span::raw(query.as_str()),
                    Span::styled("█", Style::default().fg(Color::Yellow)),
                ]),
                Line::from(""),
            ];
            let mut last_group = "";
            let visible_rows = 18usize;
            let start = if *selected >= visible_rows {
                selected.saturating_add(1).saturating_sub(visible_rows)
            } else {
                0
            };
            for (index, entry) in entries.iter().enumerate().skip(start).take(visible_rows) {
                if entry.group != last_group {
                    if !last_group.is_empty() {
                        lines.push(Line::from(""));
                    }
                    lines.push(Line::styled(
                        entry.group,
                        Style::default()
                            .fg(Color::LightMagenta)
                            .add_modifier(Modifier::BOLD),
                    ));
                    last_group = entry.group;
                }
                let note = entry.note.unwrap_or("");
                let row = format!(
                    "{:<34} {:<14} {}",
                    entry.label,
                    provider_choice_name(entry.provider),
                    note
                );
                let style = if index == *selected {
                    Style::default()
                        .fg(Color::White)
                        .bg(HEADER_BORDER)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(MODAL_FG).bg(MODAL_BG)
                };
                lines.push(Line::styled(row, style));
            }
            if entries.is_empty() && !query.trim().is_empty() {
                lines.push(Line::from("No catalog match."));
                lines.push(Line::from(format!(
                    "Press Enter to use `{}` for the current provider.",
                    query.trim()
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from("Enter select  Esc close  Type to filter"));
            Paragraph::new(lines)
                .block(modal_block(" Select Model "))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::ApiKeyInput { choice, input } => {
            let env_name = default_api_key_env(*choice).unwrap_or("API_KEY");
            let lines = vec![
                Line::from(format!(
                    "Paste your {} API key and press Enter. Esc to cancel.",
                    provider_choice_name(*choice)
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Key: ", Style::default().fg(Color::Gray)),
                    Span::raw("*".repeat(input.len())),
                    Span::styled("█", Style::default().fg(Color::Yellow)),
                ]),
                Line::from(""),
                Line::from(format!(
                    "Or set ${env_name} in your shell and switch provider again."
                )),
            ];
            Paragraph::new(lines)
                .block(modal_block(" API Key Setup "))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::AuthRequired {
            provider_name,
            env_name,
            input,
            error,
        } => {
            let mut lines = vec![
                Line::from(format!(
                    "Authentication failed for {}. Enter your API key:",
                    provider_name
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Key: ", Style::default().fg(Color::Gray)),
                    Span::raw("•".repeat(input.len())),
                    Span::styled("█", Style::default().fg(Color::Yellow)),
                ]),
                Line::from(""),
                Line::from(format!(
                    "Key will be saved to secrets store (also set ${env_name} to avoid this prompt)."
                )),
                Line::from(""),
                Line::from("Enter to save  Esc to cancel"),
            ];
            if let Some(error) = error {
                lines.push(Line::from(""));
                lines.push(Line::styled(
                    error.as_str(),
                    Style::default().fg(Color::Red),
                ));
            }
            Paragraph::new(lines)
                .block(modal_block(" API Key Required "))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::Error(message) => {
            Paragraph::new(message.as_str())
                .block(modal_block(" Error "))
                .style(Style::default().fg(ERROR_FG).bg(MODAL_BG))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::Help => {
            Paragraph::new("Enter submit | Alt+Enter newline | Ctrl+P commands | Ctrl+N new session | Ctrl+C cancel running task, then Ctrl+C again to quit | PageUp/PageDown scroll | Ctrl+T toggle sidebar | Shift+Tab toggle mode | Ctrl+H/? help")
                .block(modal_block(" Help "))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::ThemeSelector { selected } => {
            let themes = Theme::all();
            let mut lines = vec![Line::from(vec![Span::styled(
                "Theme Selector (↑↓ navigate, Enter apply) ",
                Style::default().fg(Color::Cyan),
            )])];
            for (i, theme) in themes.iter().enumerate() {
                let style = if i == *selected {
                    Style::default().fg(Color::White).bg(HEADER_BORDER)
                } else {
                    Style::default().fg(MODAL_FG)
                };
                lines.push(Line::styled(format!("  {} ", theme.name()), style));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "  /theme <name> — switch theme inline ",
                Style::default().fg(DIM_FG),
            )]));
            Paragraph::new(lines)
                .block(modal_block(" Theme "))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
        ModalState::CostMeter {
            session_tokens_in,
            session_tokens_out,
            session_cost_usd,
        } => {
            let content = format!(
                "Session Stats\n\nTokens in: {}\nTokens out: {}\nEst. cost: ${:.4}",
                session_tokens_in, session_tokens_out, session_cost_usd
            );
            Paragraph::new(content)
                .block(modal_block(" Cost Meter "))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
    }
}

fn render_palette(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let commands = app.command_palette.filtered();
    let mut lines = vec![Line::from(vec![
        Span::styled("query: ", Style::default().fg(Color::Gray)),
        Span::raw(app.command_palette.query.as_str()),
    ])];
    lines.push(Line::from(""));
    for (index, command) in commands.iter().enumerate() {
        let style = if index == app.command_palette.selected {
            Style::default()
                .fg(Color::White)
                .bg(HEADER_BORDER)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(MODAL_FG).bg(MODAL_BG)
        };
        lines.push(Line::styled(
            format!(
                "{:<20} /{:<12} {}",
                command.label(),
                command.slug(),
                command.description()
            ),
            style,
        ));
    }
    Paragraph::new(lines)
        .block(modal_block(" Command Palette "))
        .render(area, buf);
}

fn chat_lines(messages: &[ChatMessage]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for message in messages {
        let (bar, icon, icon_style, text_style) = match message.role {
            MessageRole::User => (
                USER_BAR,
                "▸",
                Style::default().fg(USER_BAR).add_modifier(Modifier::BOLD),
                Style::default().fg(USER_FG).bg(USER_BG),
            ),
            MessageRole::Assistant => (
                ASSISTANT_BAR,
                "H",
                Style::default()
                    .fg(ASSISTANT_BAR)
                    .add_modifier(Modifier::DIM),
                Style::default().fg(ASSISTANT_FG).bg(ASSISTANT_BG),
            ),
            MessageRole::System => (
                INPUT_IDLE,
                "•",
                Style::default().fg(DIM_FG),
                Style::default().fg(DIM_FG).bg(APP_BG),
            ),
            MessageRole::Activity => (
                TOOL_BG,
                tool_activity_icon(&message.text),
                Style::default().fg(TOOL_FG).bg(TOOL_BG),
                Style::default().fg(TOOL_FG).bg(TOOL_BG),
            ),
            MessageRole::Error => (
                ERROR_BAR,
                "✖",
                Style::default().fg(ERROR_FG).add_modifier(Modifier::BOLD),
                Style::default().fg(ERROR_FG).bg(ERROR_BG),
            ),
        };
        let mut emitted_any = false;
        for (index, line) in message.text.lines().enumerate() {
            emitted_any = true;
            if index == 0 {
                lines.push(Line::from(vec![
                    Span::styled("   ", Style::default().bg(bar)),
                    Span::styled(
                        " ",
                        Style::default().bg(match message.role {
                            MessageRole::User => USER_BG,
                            MessageRole::Assistant => ASSISTANT_BG,
                            MessageRole::Activity => TOOL_BG,
                            MessageRole::Error => ERROR_BG,
                            MessageRole::System => APP_BG,
                        }),
                    ),
                    Span::styled(format!("{icon} "), icon_style),
                    Span::styled(line.to_owned(), text_style),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("   ", Style::default().bg(bar)),
                    Span::styled("   ", text_style),
                    Span::styled(line.to_owned(), text_style),
                ]));
            }
        }
        if !emitted_any {
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default().bg(bar)),
                Span::styled("   ", text_style),
            ]));
        }
        if message.role != MessageRole::Activity {
            lines.push(Line::styled(
                "─".repeat(16),
                Style::default().fg(INPUT_IDLE).bg(APP_BG),
            ));
        }
    }
    lines
}

fn tool_activity_icon(text: &str) -> &'static str {
    if text.ends_with("...") {
        "⚙"
    } else if text.contains("failed") || text.contains("denied") {
        "✗"
    } else {
        "✓"
    }
}

fn modal_block(title: &'static str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::White)
                .bg(HEADER_BORDER)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(HEADER_BORDER))
        .style(Style::default().fg(MODAL_FG).bg(MODAL_BG))
}

fn modal_kv(label: &'static str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<11} "), Style::default().fg(DIM_FG)),
        Span::styled(value.to_owned(), Style::default().fg(MODAL_FG)),
    ])
}

fn key_span(text: &'static str) -> Span<'static> {
    Span::styled(
        text,
        Style::default()
            .fg(Color::White)
            .bg(HEADER_BORDER)
            .add_modifier(Modifier::BOLD),
    )
}

fn render_dim_overlay(area: Rect, buf: &mut Buffer) {
    let overlay = Block::default().style(Style::default().bg(Color::Black).fg(DIM_FG));
    overlay.render(area, buf);
}

fn render_toast(toast: &ToastState, area: Rect, buf: &mut Buffer) {
    let width = (toast.text.chars().count() as u16 + 6)
        .clamp(24, 52)
        .min(area.width.saturating_sub(2));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width + 2),
        y: area.y + area.height.saturating_sub(4),
        width,
        height: 3.min(area.height),
    };
    Clear.render(rect, buf);
    Paragraph::new(Line::from(vec![
        Span::styled("✓ ", Style::default().fg(SUCCESS_FG).bg(TOOL_BG)),
        Span::styled(toast.text.clone(), Style::default().fg(APP_FG).bg(TOOL_BG)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(HEADER_BORDER))
            .style(Style::default().bg(TOOL_BG)),
    )
    .render(rect, buf);
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

fn estimated_cost(provider: ProviderChoice, tokens_in: u32, tokens_out: u32) -> Option<f64> {
    let (input_per_million, output_per_million) = match provider {
        ProviderChoice::Anthropic => (3.0_f64, 15.0_f64),
        ProviderChoice::OpenaiCompat => (0.15_f64, 0.60_f64),
        ProviderChoice::Openrouter => (0.50_f64, 1.50_f64),
        ProviderChoice::Groq
        | ProviderChoice::Gemini
        | ProviderChoice::NvidiaNim
        | ProviderChoice::Ollama
        | ProviderChoice::Auto => return None,
    };
    Some(
        (f64::from(tokens_in) / 1_000_000.0_f64) * input_per_million
            + (f64::from(tokens_out) / 1_000_000.0_f64) * output_per_million,
    )
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

fn transcript_scroll_offsets(
    total_lines: usize,
    viewport_height: u16,
    scroll_from_bottom: usize,
) -> (usize, usize, usize) {
    let max_scroll = total_lines.saturating_sub(viewport_height as usize);
    let scroll_from_bottom = scroll_from_bottom.min(max_scroll);
    let top_offset = max_scroll.saturating_sub(scroll_from_bottom);
    (top_offset, scroll_from_bottom, max_scroll)
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

fn provider_selector_list() -> Vec<(ProviderChoice, Option<&'static str>)> {
    vec![
        (ProviderChoice::Groq, Some("GROQ_API_KEY")),
        (ProviderChoice::Anthropic, Some("ANTHROPIC_API_KEY")),
        (ProviderChoice::Openrouter, Some("OPENROUTER_API_KEY")),
        (ProviderChoice::Gemini, Some("GOOGLE_API_KEY")),
        (ProviderChoice::NvidiaNim, Some("NVIDIA_API_KEY")),
        (ProviderChoice::OpenaiCompat, Some("OPENAI_API_KEY")),
        (ProviderChoice::Ollama, None),
    ]
}

fn model_catalog() -> &'static [ModelCatalogEntry] {
    &[
        ModelCatalogEntry {
            group: "Recent",
            label: "Groq Llama 3.3 70B",
            provider: ProviderChoice::Groq,
            model: "llama-3.3-70b-versatile",
            note: Some("tools"),
        },
        ModelCatalogEntry {
            group: "Recent",
            label: "Kimi K2.6",
            provider: ProviderChoice::NvidiaNim,
            model: "moonshotai/kimi-k2.6",
            note: None,
        },
        ModelCatalogEntry {
            group: "Recent",
            label: "GLM 5.1",
            provider: ProviderChoice::NvidiaNim,
            model: "z-ai/glm-5.1",
            note: None,
        },
        ModelCatalogEntry {
            group: "Recent",
            label: "DeepSeek V4 Pro",
            provider: ProviderChoice::NvidiaNim,
            model: "deepseek-ai/deepseek-v4-pro",
            note: None,
        },
        ModelCatalogEntry {
            group: "Groq",
            label: "Llama 3.3 70B Versatile",
            provider: ProviderChoice::Groq,
            model: "llama-3.3-70b-versatile",
            note: Some("tools"),
        },
        ModelCatalogEntry {
            group: "Groq",
            label: "Llama 3.1 8B Instant",
            provider: ProviderChoice::Groq,
            model: "llama-3.1-8b-instant",
            note: Some("fast"),
        },
        ModelCatalogEntry {
            group: "Groq",
            label: "GPT OSS 120B",
            provider: ProviderChoice::Groq,
            model: "openai/gpt-oss-120b",
            note: None,
        },
        ModelCatalogEntry {
            group: "Groq",
            label: "GPT OSS 20B",
            provider: ProviderChoice::Groq,
            model: "openai/gpt-oss-20b",
            note: None,
        },
        ModelCatalogEntry {
            group: "Google",
            label: "Gemini 2.5 Flash",
            provider: ProviderChoice::Gemini,
            model: "gemini-2.5-flash",
            note: Some("free"),
        },
        ModelCatalogEntry {
            group: "Google",
            label: "Gemini 2.5 Flash Lite",
            provider: ProviderChoice::Gemini,
            model: "gemini-2.5-flash-lite",
            note: Some("free"),
        },
        ModelCatalogEntry {
            group: "Google",
            label: "Gemini 2.5 Pro",
            provider: ProviderChoice::Gemini,
            model: "gemini-2.5-pro",
            note: None,
        },
        ModelCatalogEntry {
            group: "Google",
            label: "Gemini 2.0 Flash",
            provider: ProviderChoice::Gemini,
            model: "gemini-2.0-flash",
            note: Some("free"),
        },
        ModelCatalogEntry {
            group: "Google",
            label: "Gemini 3 Pro Preview",
            provider: ProviderChoice::Gemini,
            model: "gemini-3-pro-preview",
            note: None,
        },
        ModelCatalogEntry {
            group: "NVIDIA",
            label: "Kimi K2.6",
            provider: ProviderChoice::NvidiaNim,
            model: "moonshotai/kimi-k2.6",
            note: None,
        },
        ModelCatalogEntry {
            group: "NVIDIA",
            label: "GLM 5.1",
            provider: ProviderChoice::NvidiaNim,
            model: "z-ai/glm-5.1",
            note: None,
        },
        ModelCatalogEntry {
            group: "NVIDIA",
            label: "DeepSeek V4 Pro",
            provider: ProviderChoice::NvidiaNim,
            model: "deepseek-ai/deepseek-v4-pro",
            note: None,
        },
        ModelCatalogEntry {
            group: "NVIDIA",
            label: "Nemotron 3 Super 120B",
            provider: ProviderChoice::NvidiaNim,
            model: "nvidia/nemotron-3-super-120b",
            note: None,
        },
        ModelCatalogEntry {
            group: "OpenRouter",
            label: "Kimi K2.6",
            provider: ProviderChoice::Openrouter,
            model: "moonshotai/kimi-k2.6",
            note: None,
        },
        ModelCatalogEntry {
            group: "OpenRouter",
            label: "DeepSeek Chat",
            provider: ProviderChoice::Openrouter,
            model: "deepseek/deepseek-chat",
            note: Some("free"),
        },
        ModelCatalogEntry {
            group: "OpenRouter",
            label: "DeepSeek Reasoner",
            provider: ProviderChoice::Openrouter,
            model: "deepseek/deepseek-r1",
            note: Some("free"),
        },
        ModelCatalogEntry {
            group: "OpenRouter",
            label: "Qwen 3 Coder",
            provider: ProviderChoice::Openrouter,
            model: "qwen/qwen3-coder",
            note: None,
        },
        ModelCatalogEntry {
            group: "OpenRouter",
            label: "Gemma 3 31B Free",
            provider: ProviderChoice::Openrouter,
            model: "google/gemma-3-31b-it:free",
            note: Some("free"),
        },
        ModelCatalogEntry {
            group: "Anthropic",
            label: "Claude Opus 4.1",
            provider: ProviderChoice::Anthropic,
            model: "claude-opus-4-1-20250805",
            note: None,
        },
        ModelCatalogEntry {
            group: "Anthropic",
            label: "Claude 3.5 Haiku",
            provider: ProviderChoice::Anthropic,
            model: "claude-3-5-haiku-20241022",
            note: Some("fast"),
        },
        ModelCatalogEntry {
            group: "Local",
            label: "Qwen3 4B",
            provider: ProviderChoice::Ollama,
            model: "qwen3:4b",
            note: Some("local"),
        },
    ]
}

fn filtered_model_catalog(query: &str) -> Vec<ModelCatalogEntry> {
    let query = query.trim().to_ascii_lowercase();
    model_catalog()
        .iter()
        .copied()
        .filter(|entry| {
            query.is_empty()
                || entry.label.to_ascii_lowercase().contains(&query)
                || entry.model.to_ascii_lowercase().contains(&query)
                || entry.group.to_ascii_lowercase().contains(&query)
                || provider_choice_name(entry.provider)
                    .to_ascii_lowercase()
                    .contains(&query)
        })
        .collect()
}

fn is_auth_error(error: &str) -> bool {
    error.contains("HTTP 401")
        || error.contains("invalid_api_key")
        || error.contains("Invalid API Key")
        || error.contains("Unauthorized")
        || error.contains("authentication_error")
}

fn friendly_error(error: &str) -> String {
    if error.contains("HTTP 401")
        || error.contains("invalid_api_key")
        || error.contains("Invalid API Key")
    {
        "Invalid API key. Set the provider's env key (e.g. GROQ_API_KEY) and press Ctrl+P → Provider to switch, or run `helm init --force`.".to_owned()
    } else if error.contains("HTTP 429") || error.contains("rate_limit") {
        "Rate limited. Wait for the provider reset, switch model (Ctrl+P → Model), or switch provider (Ctrl+P → Provider).".to_owned()
    } else if error.contains("model") && error.contains("not found") {
        "Model not found. For Ollama run `ollama pull qwen3:4b`. Use Ctrl+P → Model to change model.".to_owned()
    } else if error.contains("HTTP 400")
        || error.contains("tool_use")
        || error.contains("tool name")
    {
        "Provider rejected a tool call (HTTP 400). The model may not support tool use. Switch to a different model (Ctrl+P → Model) or provider (Ctrl+P → Provider).".to_owned()
    } else {
        error.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::sync::{Mutex, OnceLock};

    fn app_in_dir(dir: &tempfile::TempDir, choice: crate::ProviderChoice) -> TuiApp {
        let db = dir.path().join("helm.db");
        let memory = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(MemoryStore::open(&db))
                .unwrap(),
        );
        let secrets = SecretsStore::open_at(dir.path().join("secrets.toml")).unwrap();
        TuiApp::new(TuiRuntime {
            provider_settings: ProviderSettings {
                choice,
                base_url: if choice == crate::ProviderChoice::Ollama {
                    Some("http://localhost:11434".to_owned())
                } else {
                    Some("https://api.example.com/v1".to_owned())
                },
                model: Some(
                    match choice {
                        crate::ProviderChoice::Groq => "llama-3.3-70b-versatile",
                        _ => "qwen3:4b",
                    }
                    .to_owned(),
                ),
                api_key_env: default_api_key_env(choice).map(str::to_owned),
                api_key: None,
                source: if choice == crate::ProviderChoice::Ollama {
                    crate::ProviderSource::Fallback
                } else {
                    crate::ProviderSource::Cli
                },
            },
            db_path: db,
            config_path: dir.path().join("config.toml"),
            memory,
            max_iterations: Some(2),
            secrets,
            tui_paste_key_modal: true,
            auto_approve: false,
            read_only: false,
        })
    }

    fn app() -> TuiApp {
        let dir = tempfile::tempdir().unwrap();
        app_in_dir(&dir, crate::ProviderChoice::Ollama)
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
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
    fn auth_401_opens_modal_and_keeps_retry_task() {
        let mut app = app();
        app.active_settings.choice = ProviderChoice::Groq;
        app.provider_name = "groq".to_owned();
        let (tx, _rx) = mpsc::unbounded_channel();

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(app.handle_ui_event(
                UiEvent::AgentDone {
                    run_id: app.active_run_id,
                    task: "check services".to_owned(),
                    result: Err(HelmError::Provider(helm_core::ProviderError::HttpStatus {
                        status: 401,
                        body: "bad key".to_owned(),
                    })),
                },
                tx,
            ))
            .unwrap();

        assert!(matches!(app.modal, Some(ModalState::AuthRequired { .. })));
        assert_eq!(app.pending_auth_retry, Some("check services".to_owned()));
    }

    #[test]
    fn auth_modal_escape_dismisses_cleanly() {
        let mut app = app();
        app.pending_auth_retry = Some("retry me".to_owned());
        app.modal = Some(ModalState::AuthRequired {
            provider_name: "groq".to_owned(),
            env_name: "GROQ_API_KEY".to_owned(),
            input: String::new(),
            error: None,
        });
        let (tx, _rx) = mpsc::unbounded_channel();

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(app.handle_modal_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), tx))
            .unwrap();

        assert!(app.modal.is_none());
        assert!(app.pending_auth_retry.is_none());
    }

    #[test]
    fn auth_modal_never_renders_raw_key() {
        let mut app = app();
        app.modal = Some(ModalState::AuthRequired {
            provider_name: "groq".to_owned(),
            env_name: "GROQ_API_KEY".to_owned(),
            input: "SECRET_KEY_VALUE".to_owned(),
            error: None,
        });

        let buffer = render_to_buffer(app, 80, 24);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(!text.contains("SECRET_KEY_VALUE"));
        assert!(text.contains("••••"));
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
            "shell: shell `date && uname -a` -> /tmp/helm.txt ..."
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
                .contains("shell: shell `date && uname -a`")
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
    fn model_catalog_filters_across_providers() {
        let models = filtered_model_catalog("gemini 2.5 flash");
        assert!(models.iter().any(|entry| {
            entry.provider == ProviderChoice::Gemini && entry.model == "gemini-2.5-flash"
        }));

        let models = filtered_model_catalog("kimi");
        assert!(models.iter().any(|entry| {
            entry.provider == ProviderChoice::NvidiaNim && entry.model == "moonshotai/kimi-k2.6"
        }));
    }

    #[test]
    fn applying_catalog_model_switches_provider_and_model() {
        let mut app = app();
        let entry = filtered_model_catalog("deepseek v4")
            .into_iter()
            .find(|entry| entry.provider == ProviderChoice::NvidiaNim)
            .unwrap();

        app.apply_model_entry(entry);

        assert_eq!(app.active_settings.choice, ProviderChoice::NvidiaNim);
        assert_eq!(
            app.active_settings.model,
            Some("deepseek-ai/deepseek-v4-pro".to_owned())
        );
        assert_eq!(app.provider_name, "nvidia-nim");
    }

    #[test]
    fn transcript_scrolls_from_bottom() {
        let (top, from_bottom, max) = transcript_scroll_offsets(20, 5, 0);
        assert_eq!(top, 15);
        assert_eq!(from_bottom, 0);
        assert_eq!(max, 15);

        let (top, from_bottom, max) = transcript_scroll_offsets(20, 5, 5);
        assert_eq!(top, 10);
        assert_eq!(from_bottom, 5);
        assert_eq!(max, 15);

        let (top, from_bottom, max) = transcript_scroll_offsets(20, 5, usize::MAX / 2);
        assert_eq!(top, 0);
        assert_eq!(from_bottom, 15);
        assert_eq!(max, 15);
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

    #[test]
    fn startup_env_key_is_not_persisted_to_secrets_store() {
        let _guard = env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: this test serializes process-global env mutation with a mutex.
        unsafe {
            std::env::set_var("GROQ_API_KEY", "gsk_abcdefghijklmnopqrstuvwxyz123456");
        }

        let app = app_in_dir(&dir, crate::ProviderChoice::Groq);
        assert_eq!(
            app.active_settings.api_key.as_deref(),
            Some("gsk_abcdefghijklmnopqrstuvwxyz123456")
        );
        assert!(app.runtime.secrets.get("GROQ_API_KEY").unwrap().is_none());

        // SAFETY: paired with the guarded set_var above.
        unsafe {
            std::env::remove_var("GROQ_API_KEY");
        }
    }

    #[test]
    fn manual_provider_key_persists_without_plaintext_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in_dir(&dir, crate::ProviderChoice::Groq);

        app.apply_provider_with_key(
            crate::ProviderChoice::Groq,
            "gsk_abcdefghijklmnopqrstuvwxyz123456".to_owned(),
            true,
        );

        let stored = app.runtime.secrets.get("GROQ_API_KEY").unwrap().unwrap();
        assert_eq!(stored.expose(), "gsk_abcdefghijklmnopqrstuvwxyz123456");

        let config = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(config.contains("api_key_env = \"GROQ_API_KEY\""));
        assert!(!config.contains("api_key ="));
        assert!(!config.contains("gsk_abcdefghijklmnopqrstuvwxyz123456"));
    }
}
