//! Full-screen HELM terminal UI built with ratatui.

use std::{
    cell::Cell,
    collections::HashMap,
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use helm_agent::{AgentEvent, AgentEventSink, Budget, ReactAgent, RunResult, StructuredEvidence};
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
use serde::Deserialize;
use tokio::{runtime::Handle, sync::mpsc, task::JoinHandle};

use crate::{
    ProviderChoice, ProviderSettings, TroubleshootingPlanStore, build_provider, custom_commands,
    default_api_key_env, default_db_path, default_model_name, keybindings::KeyMap,
    provider_choice_name, remote::RemoteRegistry, wrap_for_remote, write_helm_config,
};
use crate::{sandbox::ResolvedSandbox, secrets::SecretsStore};

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
    pub(crate) diagnose_mode: bool,
    pub(crate) sandbox: Option<ResolvedSandbox>,
    pub(crate) remote_target: Option<String>,
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
        entries: Vec<ModelCatalogEntry>,
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
    ThemeSelector {
        selected: usize,
    },
    CostMeter {
        session_tokens_in: u64,
        session_tokens_out: u64,
        session_cost_usd: f64,
    },
    /// Command-by-command plan execution approval.
    PlanExecution {
        plan_id: String,
        plan_title: String,
        step_index: usize,
        step_count: usize,
        step_previews: Vec<String>,
        step_effects: Vec<String>,
        step_tools: Vec<String>,
        step_risks: Vec<String>,
        phase: PlanExecPhase,
        result_summary: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum PlanExecPhase {
    Loading,
    Approving,
    Running,
    Done,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelCatalogEntry {
    group: String,
    label: String,
    provider: ProviderChoice,
    model: String,
    note: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedModelCatalog {
    fetched_at: Instant,
    entries: Vec<ModelCatalogEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKeyStatus {
    NoKeyNeeded,
    Env,
    Stored,
    Session,
    Unset,
}

impl ProviderKeyStatus {
    fn label(self, env_name: Option<&str>) -> String {
        match self {
            Self::NoKeyNeeded => "no key needed".to_owned(),
            Self::Env => format!("{} via env", env_name.unwrap_or("API_KEY")),
            Self::Stored => format!("{} stored", env_name.unwrap_or("API_KEY")),
            Self::Session => "session override".to_owned(),
            Self::Unset => format!("{} unset", env_name.unwrap_or("API_KEY")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandAction {
    NewSession,
    Clear,
    Replay,
    Doctor,
    Remote,
    Provider,
    Model,
    Permissions,
    Audit,
    Skills,
    Browser,
    Init,
    Sessions,
    Resume,
    Config,
    Theme,
    Keybindings,
    Stats,
    Mcp,
    Compact,
    Diff,
    Tools,
    Undo,
    Redo,
    Cost,
    Quit,
    Help,
    Diagnose,
    Evidence,
    ApplyPlan,
    Dashboard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PaletteItem {
    BuiltIn(CommandAction),
    Custom(custom_commands::CustomCommand),
}

impl PaletteItem {
    fn label(&self) -> String {
        match self {
            Self::BuiltIn(action) => action.label().to_owned(),
            Self::Custom(command) => format!("Custom: {}", command.name),
        }
    }

    fn slug(&self) -> String {
        match self {
            Self::BuiltIn(action) => action.slug().to_owned(),
            Self::Custom(command) => command.name.clone(),
        }
    }

    fn description(&self) -> String {
        match self {
            Self::BuiltIn(action) => action.description().to_owned(),
            Self::Custom(command) => command.description.clone(),
        }
    }

    fn matches_query(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let query = query.to_ascii_lowercase();
        self.label().to_ascii_lowercase().contains(&query)
            || self.slug().to_ascii_lowercase().contains(&query)
            || self.description().to_ascii_lowercase().contains(&query)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SlashItem {
    BuiltIn(CommandAction),
    Custom(custom_commands::CustomCommand),
}

impl SlashItem {
    fn slug(&self) -> &str {
        match self {
            Self::BuiltIn(action) => action.slug(),
            Self::Custom(command) => &command.name,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::BuiltIn(action) => action.label().to_owned(),
            Self::Custom(command) => format!("Custom: {}", command.name),
        }
    }

    fn description(&self) -> String {
        match self {
            Self::BuiltIn(action) => action.description().to_owned(),
            Self::Custom(command) => command.description.clone(),
        }
    }
}

impl CommandAction {
    fn all() -> Vec<Self> {
        vec![
            Self::NewSession,
            Self::Clear,
            Self::Replay,
            Self::Doctor,
            Self::Remote,
            Self::Provider,
            Self::Model,
            Self::Permissions,
            Self::Audit,
            Self::Skills,
            Self::Browser,
            Self::Init,
            Self::Sessions,
            Self::Resume,
            Self::Config,
            Self::Theme,
            Self::Keybindings,
            Self::Stats,
            Self::Mcp,
            Self::Compact,
            Self::Diff,
            Self::Tools,
            Self::Undo,
            Self::Redo,
            Self::Cost,
            Self::Quit,
            Self::Help,
            Self::Diagnose,
            Self::Evidence,
            Self::ApplyPlan,
            Self::Dashboard,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Self::NewSession => "New Session",
            Self::Clear => "Clear Transcript",
            Self::Replay => "Replay Episode",
            Self::Doctor => "Doctor",
            Self::Remote => "Remote Target",
            Self::Provider => "Provider Selector",
            Self::Model => "Model Selector",
            Self::Permissions => "Permissions",
            Self::Audit => "Audit Verify",
            Self::Skills => "Skills",
            Self::Browser => "Browser Status",
            Self::Init => "Init AGENTS.md",
            Self::Sessions => "Sessions",
            Self::Resume => "Resume",
            Self::Config => "Config Editor",
            Self::Theme => "Theme Selector",
            Self::Keybindings => "Keybindings",
            Self::Stats => "Stats",
            Self::Mcp => "MCP Servers",
            Self::Compact => "Compact",
            Self::Diff => "Diff Last Edit",
            Self::Tools => "Tools",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cost => "Cost Meter",
            Self::Quit => "Quit",
            Self::Help => "Help",
            Self::Diagnose => "Diagnose Mode",
            Self::Evidence => "Evidence Report",
            Self::ApplyPlan => "Execute Plan",
            Self::Dashboard => "Dashboard",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::NewSession => "new",
            Self::Clear => "clear",
            Self::Replay => "replay",
            Self::Doctor => "doctor",
            Self::Remote => "remote",
            Self::Provider => "provider",
            Self::Model => "model",
            Self::Permissions => "permissions",
            Self::Audit => "audit",
            Self::Skills => "skills",
            Self::Browser => "browser",
            Self::Init => "init",
            Self::Sessions => "sessions",
            Self::Resume => "resume",
            Self::Config => "config",
            Self::Theme => "theme",
            Self::Keybindings => "keybindings",
            Self::Stats => "stats",
            Self::Mcp => "mcp",
            Self::Compact => "compact",
            Self::Diff => "diff",
            Self::Tools => "tools",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Cost => "cost",
            Self::Quit => "quit",
            Self::Help => "help",
            Self::Diagnose => "diagnose",
            Self::Evidence => "evidence",
            Self::ApplyPlan => "apply-plan",
            Self::Dashboard => "dashboard",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::NewSession => "clear transcript and start over",
            Self::Clear => "clear the visible transcript",
            Self::Replay => "show replay command for this episode",
            Self::Doctor => "show provider and system diagnostics hint",
            Self::Remote => "show or switch the active remote target",
            Self::Provider => "switch LLM backend",
            Self::Model => "edit active model id",
            Self::Permissions => "grant or inspect capabilities",
            Self::Audit => "verify audit log chain",
            Self::Skills => "inspect local skill library",
            Self::Browser => "browser automation status",
            Self::Init => "generate AGENTS.md in the current project",
            Self::Sessions => "show session-resume guidance",
            Self::Resume => "show resume guidance",
            Self::Config => "view or edit config inline (/config key=value)",
            Self::Theme => "switch theme (/theme dracula)",
            Self::Keybindings => "show keybinding overrides in the HELM XDG config directory",
            Self::Stats => "show token usage and cost since today",
            Self::Mcp => "list MCP servers",
            Self::Compact => "summarize transcript to reclaim context",
            Self::Diff => "show diff of the last fs_write",
            Self::Tools => "list loaded tools and capabilities",
            Self::Undo => "undo last agent file edit",
            Self::Redo => "redo last undone agent edit",
            Self::Cost => "open the session cost meter",
            Self::Quit => "exit HELM",
            Self::Help => "keyboard shortcuts and commands",
            Self::Diagnose => "switch to diagnose mode (read-only, limited tools)",
            Self::Evidence => "show evidence report for last tool call",
            Self::ApplyPlan => "execute a troubleshooting plan with step approval",
            Self::Dashboard => "switch to monitoring dashboard",
        }
    }

    fn matches_slug(self, slug: &str) -> bool {
        match self {
            Self::Quit => matches!(slug, "quit" | "exit" | "q"),
            Self::NewSession => matches!(slug, "new" | "n"),
            Self::Clear => slug == "clear",
            Self::Replay => slug == "replay",
            Self::Doctor => slug == "doctor",
            Self::Remote => slug == "remote",
            Self::Provider => slug == "provider",
            Self::Model => slug == "model",
            Self::Permissions => slug == "permissions",
            Self::Audit => slug == "audit",
            Self::Skills => slug == "skills",
            Self::Browser => slug == "browser",
            Self::Init => slug == "init",
            Self::Sessions => slug == "sessions",
            Self::Resume => slug == "resume",
            Self::Config => slug == "config",
            Self::Theme => slug == "theme",
            Self::Keybindings => matches!(slug, "keybindings" | "keybinds" | "keys"),
            Self::Stats => matches!(slug, "stats" | "usage"),
            Self::Mcp => slug == "mcp",
            Self::Compact => slug == "compact",
            Self::Diff => slug == "diff",
            Self::Tools => slug == "tools",
            Self::Undo => slug == "undo",
            Self::Redo => slug == "redo",
            Self::Cost => slug == "cost",
            Self::Help => slug == "help",
            Self::Diagnose => slug == "diagnose",
            Self::Evidence => slug == "evidence",
            Self::ApplyPlan => slug == "apply-plan",
            Self::Dashboard => matches!(slug, "dashboard" | "monitor"),
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
    Diagnose,
    Dashboard,
}

impl AgentMode {
    fn next(self) -> Self {
        match self {
            Self::Chat => Self::Plan,
            Self::Plan => Self::AutoAccept,
            Self::AutoAccept => Self::Diagnose,
            Self::Diagnose => Self::Dashboard,
            Self::Dashboard => Self::Chat,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Plan => "Plan",
            Self::AutoAccept => "Auto-Accept",
            Self::Diagnose => "Diagnose",
            Self::Dashboard => "Dashboard",
        }
    }
}

// ── Dashboard types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashPanel {
    Health,
    Findings,
    Services,
    Containers,
    Disk,
    Ports,
    Logs,
    Backups,
    Plans,
}

impl DashPanel {
    fn all() -> &'static [Self] {
        &[
            Self::Health,
            Self::Findings,
            Self::Services,
            Self::Containers,
            Self::Disk,
            Self::Ports,
            Self::Logs,
            Self::Backups,
            Self::Plans,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Health => "Health",
            Self::Findings => "Findings",
            Self::Services => "Services",
            Self::Containers => "Containers",
            Self::Disk => "Disk",
            Self::Ports => "Ports",
            Self::Logs => "Logs",
            Self::Backups => "Backups",
            Self::Plans => "Plans",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DashboardData {
    hostname: String,
    load_1m: f64,
    load_5m: f64,
    load_15m: f64,
    memory_used_pct: f64,
    disk_entries: Vec<String>,
    total_services: usize,
    failed_services: usize,
    total_containers: usize,
    running_containers: usize,
    listening_ports: usize,
    last_log_errors: u64,
    backup_count: usize,
    finding_count: usize,
    finding_warnings: usize,
    collected_at: String,
}

#[derive(Debug, Clone)]
struct DashboardState {
    data: DashboardData,
    selected: DashPanel,
    error: Option<String>,
}

impl DashboardState {
    fn new() -> Self {
        Self {
            data: DashboardData::default(),
            selected: DashPanel::Health,
            error: None,
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
    custom_commands: Vec<custom_commands::CustomCommand>,
    keymap: KeyMap,
    running: bool,
    shutdown: bool,
    mode: AgentMode,
    show_sidebar: bool,
    spinner: usize,
    provider_name: String,
    model: String,
    active_remote: Option<String>,
    status_note: String,
    catalog_cache: HashMap<ProviderChoice, CachedModelCatalog>,
    pending_tool_summaries: HashMap<String, String>,
    active_tool_cells: HashMap<String, usize>,
    toast: Option<ToastState>,
    last_chat_height: Cell<u16>,
    active_run_id: u64,
    agent_task: Option<JoinHandle<()>>,
    pending_auth_retry: Option<String>,
    last_evidence: Option<EvidenceSnapshot>,
    task_started: Option<Instant>,
    tool_start_times: HashMap<String, Instant>,
    session_tokens_in: u32,
    session_tokens_out: u32,
    resume_context: Option<String>,
    #[allow(dead_code)]
    theme: Theme,
    dashboard: DashboardState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToastVariant {
    Success,
    Error,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToastState {
    text: String,
    created: Instant,
    variant: ToastVariant,
}

struct TuiRuntimeInner {
    db_path: PathBuf,
    config_path: PathBuf,
    memory: Arc<MemoryStore>,
    max_iterations: Option<u32>,
    secrets: SecretsStore,
    tui_paste_key_modal: bool,
    sandbox: Option<ResolvedSandbox>,
}

/// Stored evidence snapshot for later display via /evidence.
#[allow(dead_code)]
struct EvidenceSnapshot {
    tool_name: String,
    evidence: StructuredEvidence,
    formatted: String,
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
        } else if runtime.diagnose_mode {
            AgentMode::Diagnose
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
                sandbox: runtime.sandbox,
            }),
            active_settings,
            session: SessionState::default(),
            input: InputState::new(),
            focus: PanelFocus::Input,
            modal: None,
            slash_popup: None,
            command_palette: CommandPaletteState::new(),
            custom_commands: custom_commands::load_all(),
            keymap: KeyMap::load(),
            running: false,
            shutdown: false,
            mode,
            show_sidebar: false,
            spinner: 0,
            provider_name,
            model,
            active_remote: runtime.remote_target,
            status_note: "ready".to_owned(),
            catalog_cache: HashMap::new(),
            pending_tool_summaries: HashMap::new(),
            active_tool_cells: HashMap::new(),
            toast: None,
            last_chat_height: Cell::new(10),
            active_run_id: 0,
            agent_task: None,
            pending_auth_retry: None,
            last_evidence: None,
            task_started: None,
            tool_start_times: HashMap::new(),
            session_tokens_in: 0,
            session_tokens_out: 0,
            resume_context: None,
            theme: Theme::default(),
            dashboard: DashboardState::new(),
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
                self.task_started = None;
                self.tool_start_times.clear();
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
                    self.execute_slash_from_popup(tx).await?;
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
            if self.key_matches(&key, "cancel", KeyCode::Char('c'), KeyModifiers::CONTROL) {
                if self.running {
                    self.cancel_running_task();
                    return Ok(false);
                }
                return Ok(true);
            }
            if self.key_matches(&key, "quit", KeyCode::Char('d'), KeyModifiers::CONTROL)
                && self.input.text.is_empty()
            {
                return Ok(true);
            }
            if self.key_matches(&key, "newline", KeyCode::Char('j'), KeyModifiers::CONTROL) {
                self.input.insert_newline();
                return Ok(false);
            }
            if self.key_matches(&key, "palette", KeyCode::Char('p'), KeyModifiers::CONTROL) {
                self.open_palette();
                return Ok(false);
            }
            if self.key_matches(
                &key,
                "tool_sidebar",
                KeyCode::Char('t'),
                KeyModifiers::CONTROL,
            ) {
                self.show_sidebar = !self.show_sidebar;
                self.toast(if self.show_sidebar {
                    "Sidebar visible"
                } else {
                    "Sidebar hidden"
                });
                return Ok(false);
            }
            if self.key_matches(&key, "history", KeyCode::Char('h'), KeyModifiers::CONTROL) {
                self.modal = Some(ModalState::Help);
                return Ok(false);
            }
            if self.key_matches(&key, "clear", KeyCode::Char('l'), KeyModifiers::CONTROL) {
                self.clear_transcript();
                return Ok(false);
            }

            match key.code {
                KeyCode::Char('n') => self.new_session(),
                KeyCode::Char('r') => self.push_chat(MessageRole::System, self.replay_hint()),
                KeyCode::Char('a') => {
                    self.modal = Some(ModalState::Permission {
                        capability: Capability::ShellShell,
                        tool_name: "pending".to_owned(),
                        taint: "user".to_owned(),
                        detail: "No pending permission request.".to_owned(),
                    });
                }
                KeyCode::Home => self.session.transcript_scroll = usize::MAX / 2,
                KeyCode::End => self.session.transcript_scroll = 0,
                KeyCode::Char('u') => self.input.clear(),
                _ => {}
            }
            return Ok(false);
        }

        if !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
            && self.key_matches(&key, "send", KeyCode::Enter, KeyModifiers::NONE)
        {
            self.submit(tx).await?;
            if self.shutdown {
                return Ok(true);
            }
            return Ok(false);
        }
        if self.input.text.is_empty()
            && self.key_matches(&key, "help", KeyCode::Char('?'), KeyModifiers::NONE)
        {
            self.modal = Some(ModalState::Help);
            return Ok(false);
        }

        match key.code {
            KeyCode::Enter
                if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.input.insert_newline()
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
            KeyCode::Tab => {
                if self.mode == AgentMode::Dashboard {
                    let panels = DashPanel::all();
                    let next = (self.dashboard.selected as usize + 1) % panels.len();
                    self.dashboard.selected = panels[next];
                } else {
                    self.focus = PanelFocus::Input
                }
            }
            KeyCode::BackTab => {
                if self.mode == AgentMode::Dashboard {
                    let panels = DashPanel::all();
                    let prev = (self.dashboard.selected as usize + panels.len() - 1) % panels.len();
                    self.dashboard.selected = panels[prev];
                } else {
                    self.mode = self.mode.next();
                    self.toast(format!("Mode changed to {}", self.mode.as_str()));
                }
            }
            KeyCode::Enter if self.mode == AgentMode::Dashboard => {
                let panel = self.dashboard.selected;
                match panel {
                    DashPanel::Findings => {
                        self.push_chat(MessageRole::System, "[dashboard] Finding details: run `helm monitor` or `helm explain <id>`");
                    }
                    DashPanel::Plans => {
                        let plans = crate::default_db_path()
                            .ok()
                            .and_then(|p| rusqlite::Connection::open(p).ok())
                            .and_then(|conn| {
                                helm_memory::TroubleshootingPlanStore::list(&conn, 10).ok()
                            });
                        if let Some(list) = plans {
                            if list.is_empty() {
                                self.push_chat(
                                    MessageRole::System,
                                    "[dashboard] No saved plans. Run `helm troubleshoot` first.",
                                );
                            } else {
                                let mut msg = String::from("[dashboard] Saved plans:\n");
                                for p in &list {
                                    msg.push_str(&format!("  {} | {}\n", p.id, p.source));
                                }
                                msg.push_str("\nUse `/apply-plan <id>` to execute a plan.");
                                self.push_chat(MessageRole::System, msg);
                            }
                        }
                    }
                    _ => {
                        self.push_chat(MessageRole::System, format!("[dashboard] {} panel: use `helm snapshot` or `helm monitor` for full details.", panel.label()));
                    }
                }
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
                    let commands = self.filtered_palette_items();
                    if let Some(command) = commands.get(self.command_palette.selected).cloned() {
                        self.execute_palette_item(command, tx).await?;
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
                    let max = self.filtered_palette_items().len().saturating_sub(1);
                    self.command_palette.selected = (self.command_palette.selected + 1).min(max);
                }
                _ => {}
            },
            Some(ModalState::Permission { capability, .. }) => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.push_chat(MessageRole::System, "Permission denied.");
                    self.toast_variant("Permission denied", ToastVariant::Error);
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
                    self.toast_variant("Permission granted once", ToastVariant::Success);
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
                    self.toast_variant("Permission granted for session", ToastVariant::Success);
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
                    self.toast_variant("Permission granted always", ToastVariant::Success);
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
                    if let Some(ModalState::ModelSelector {
                        query,
                        selected,
                        entries,
                    }) = self.modal.clone()
                    {
                        let filtered = filtered_model_catalog(&entries, &query);
                        if let Some(entry) = filtered.get(selected).cloned() {
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
                    if let Some(ModalState::ModelSelector {
                        query,
                        selected,
                        entries,
                    }) = &mut self.modal
                    {
                        let max = filtered_model_catalog(entries, query)
                            .len()
                            .saturating_sub(1);
                        *selected = (*selected + 1).min(max);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(ModalState::ModelSelector {
                        query, selected, ..
                    }) = &mut self.modal
                    {
                        query.pop();
                        *selected = 0;
                    }
                }
                KeyCode::Char(ch) => {
                    if let Some(ModalState::ModelSelector {
                        query, selected, ..
                    }) = &mut self.modal
                    {
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
            Some(ModalState::ThemeSelector { .. }) => match key.code {
                KeyCode::Esc => self.modal = None,
                KeyCode::Up => {
                    if let Some(ModalState::ThemeSelector { selected }) = &mut self.modal {
                        *selected = selected.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if let Some(ModalState::ThemeSelector { selected }) = &mut self.modal {
                        let max = Theme::all().len().saturating_sub(1);
                        *selected = (*selected + 1).min(max);
                    }
                }
                KeyCode::Enter => {
                    if let Some(ModalState::ThemeSelector { selected }) = self.modal.clone() {
                        let themes = Theme::all();
                        if let Some(theme) = themes.get(selected).cloned() {
                            let label = theme.name().to_owned();
                            self.theme = theme;
                            self.push_chat(
                                MessageRole::System,
                                format!("[theme] switched to {label}"),
                            );
                        }
                    }
                    self.modal = None;
                }
                _ => {}
            },
            Some(ModalState::PlanExecution {
                ref plan_id,
                ref step_index,
                ref step_count,
                ref phase,
                ..
            }) if *phase == PlanExecPhase::Approving => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let next = *step_index + 1;
                    let plan_id_clone = plan_id.clone();
                    if next >= *step_count {
                        // All approved — execute
                        if let Some(ModalState::PlanExecution { ref mut phase, .. }) = self.modal {
                            *phase = PlanExecPhase::Running;
                        }
                        self.push_chat(
                            MessageRole::System,
                            "[apply-plan] All steps approved. Executing...",
                        );
                        // Audit: approve this step
                        let _ = Self::write_apply_plan_audit(&plan_id_clone, "approved");
                        // Use the TUI's runtime handle to spawn the apply-plan execution
                        let handle = tokio::runtime::Handle::current();
                        std::thread::spawn(move || {
                            handle.block_on(async {
                                let args = crate::ApplyPlanArgs {
                                    plan_id: plan_id_clone,
                                    yes: true,
                                    json: false,
                                };
                                match crate::run_apply_plan_command(args).await {
                                    Ok(()) => eprintln!("[apply-plan] Plan executed successfully."),
                                    Err(e) => eprintln!("[apply-plan] Execution failed: {e}"),
                                }
                            });
                        });
                    } else {
                        let _ = Self::write_apply_plan_audit(&plan_id_clone, "approved");
                        // Move to next step
                        if let Some(ModalState::PlanExecution {
                            ref mut step_index, ..
                        }) = self.modal
                        {
                            *step_index = next;
                        }
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    let next = *step_index + 1;
                    let plan_id = plan_id.clone();
                    if next >= *step_count {
                        self.push_chat(
                            MessageRole::System,
                            "[apply-plan] All remaining steps skipped.",
                        );
                        self.modal = None;
                    } else if let Some(ModalState::PlanExecution {
                        ref mut step_index, ..
                    }) = self.modal
                    {
                        *step_index = next;
                    }
                    let _ = Self::write_apply_plan_audit(&plan_id, "denied");
                }
                KeyCode::Char('!') => {
                    // Approve all remaining
                    let plan_id = plan_id.clone();
                    if let Some(ModalState::PlanExecution { ref mut phase, .. }) = self.modal {
                        *phase = PlanExecPhase::Running;
                    }
                    self.push_chat(
                        MessageRole::System,
                        "[apply-plan] All steps approved. Executing...",
                    );
                    let handle = tokio::runtime::Handle::current();
                    std::thread::spawn(move || {
                        handle.block_on(async {
                            let args = crate::ApplyPlanArgs {
                                plan_id,
                                yes: true,
                                json: false,
                            };
                            match crate::run_apply_plan_command(args).await {
                                Ok(()) => eprintln!("[apply-plan] Plan executed successfully."),
                                Err(e) => eprintln!("[apply-plan] Execution failed: {e}"),
                            }
                        });
                    });
                }
                KeyCode::Esc => {
                    self.modal = None;
                    self.push_chat(MessageRole::System, "[apply-plan] Cancelled by user.");
                }
                _ => {}
            },
            Some(ModalState::PlanExecution {
                phase: PlanExecPhase::Running,
                ..
            }) => {
                // Phase changes are driven by the spawned task completion
            }
            Some(ModalState::PlanExecution {
                phase: PlanExecPhase::Done,
                ..
            }) => {
                self.modal = None;
            }
            Some(ModalState::PlanExecution {
                phase: PlanExecPhase::Loading,
                ..
            }) => {}
            Some(_) => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    self.modal = None;
                }
            }
            None => {}
        }
        Ok(false)
    }

    fn key_matches(
        &self,
        key: &KeyEvent,
        action: &str,
        default_code: KeyCode,
        default_modifiers: KeyModifiers,
    ) -> bool {
        self.keymap.matches(action, key.code, key.modifiers)
            || (key.code == default_code && key.modifiers == default_modifiers)
    }

    async fn submit(&mut self, tx: mpsc::UnboundedSender<UiEvent>) -> Result<()> {
        if self.running {
            return Ok(());
        }
        let Some(task) = self.input.take_submit() else {
            return Ok(());
        };
        if let Some(command_text) = task.trim().strip_prefix('/') {
            let mut parts = command_text.trim().splitn(2, char::is_whitespace);
            let slug = parts.next().unwrap_or("");
            let args = parts.next().unwrap_or("").trim();
            if slug == "remote" {
                self.apply_remote_target(args);
            } else if let Some(command) = CommandAction::from_slug(slug) {
                self.execute_command_with_args(command, args);
            } else if let Some(command) = self
                .custom_commands
                .iter()
                .find(|command| command.name == slug)
                .cloned()
            {
                self.execute_custom_command(command, args, tx).await?;
            } else {
                self.push_chat(
                    MessageRole::Error,
                    format!("Unknown command `{task}`. Type /help or press Ctrl+P."),
                );
            }
            return Ok(());
        }
        if let Some(raw_query) = task.strip_prefix('#') {
            self.push_chat(MessageRole::User, task.clone());
            self.push_chat(MessageRole::System, self.quick_memory_report(raw_query));
            return Ok(());
        }
        if let Some(raw_shell) = task.strip_prefix('!') {
            let command = raw_shell.trim();
            if command.is_empty() {
                self.push_chat(
                    MessageRole::Error,
                    "Shell mode expects a command after `!`.",
                );
                return Ok(());
            }
            let wrapped = format!(
                "Run this shell command exactly once using the shell tool. \
Do not rewrite it unless required for safety or environment compatibility.\n\n\
Command:\n{command}\n\n\
Report the exit status and the concise output."
            );
            return self.start_prepared_task(task, wrapped, tx).await;
        }
        self.start_task(task, tx, true).await
    }

    async fn start_prepared_task(
        &mut self,
        display_task: String,
        agent_task: String,
        tx: mpsc::UnboundedSender<UiEvent>,
    ) -> Result<()> {
        if self.running {
            return Ok(());
        }
        self.push_chat(MessageRole::User, display_task);
        self.start_task_internal(agent_task, tx).await
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
        self.start_task_internal(task, tx).await
    }

    async fn start_task_internal(
        &mut self,
        task: String,
        tx: mpsc::UnboundedSender<UiEvent>,
    ) -> Result<()> {
        self.record_tool_event("queued", "agent", "task submitted");
        self.running = true;
        self.task_started = Some(Instant::now());
        self.status_note = "running".to_owned();
        self.active_run_id = self.active_run_id.saturating_add(1);
        self.session.transcript_scroll = 0;
        let run_id = self.active_run_id;

        let runtime = Arc::clone(&self.runtime);
        let settings = self.active_settings.clone();
        let contextual_task = if let Some(context) = self.resume_context.as_deref() {
            format!("{context}\n\nUser asks now: {task}")
        } else {
            task.clone()
        };
        let effective_task = wrap_for_remote(&contextual_task, self.active_remote.as_ref());
        let task_for_event = effective_task.clone();
        let remote_target = self.active_remote.clone();
        let mode = self.mode;
        self.agent_task = Some(tokio::spawn(async move {
            let result = run_agent_task(
                runtime,
                settings,
                effective_task,
                tx.clone(),
                run_id,
                mode,
                remote_target,
            )
            .await;
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

    fn quick_memory_report(&self, raw_query: &str) -> String {
        let query = raw_query.trim();
        if query.is_empty() {
            return "[memory] usage: `# <query>` — searches recent episodes and graph entities."
                .to_owned();
        }
        let query = query.to_owned();
        let memory = Arc::clone(&self.runtime.memory);
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let query_lower = query.to_ascii_lowercase();
                let recent = memory.recent_episodes(20).await.unwrap_or_default();
                let episode_lines = recent
                    .into_iter()
                    .filter(|episode| episode.goal.to_ascii_lowercase().contains(&query_lower))
                    .take(5)
                    .map(|episode| {
                        format!(
                            "  - [{}] {}",
                            episode.id,
                            episode.goal.chars().take(80).collect::<String>()
                        )
                    })
                    .collect::<Vec<_>>();

                let graph_lines = dirs::data_local_dir()
                    .map(|root| root.join("helm").join("graph.db"))
                    .and_then(|path| helm_memory::EntityGraph::open(&path).ok())
                    .and_then(|graph| graph.find_entities(None, Some(&query)).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .take(5)
                    .map(|entity| format!("  - {} [{}]", entity.name, entity.kind))
                    .collect::<Vec<_>>();

                let episodes = if episode_lines.is_empty() {
                    "  - no recent episode matches".to_owned()
                } else {
                    episode_lines.join("\n")
                };
                let graph = if graph_lines.is_empty() {
                    "  - no graph entities match".to_owned()
                } else {
                    graph_lines.join("\n")
                };

                format!(
                    "[memory] query: {query}\nrecent episodes:\n{episodes}\ngraph entities:\n{graph}"
                )
            })
        })
    }

    fn remote_hint(&self) -> String {
        match self.active_remote.as_deref() {
            Some(target) => format!(
                "Active remote target: {target}. Use `/remote NAME` to switch or `/remote off` to return to local mode."
            ),
            None => "No remote target selected. Use `/remote NAME` to target a registered host or `/remote off` for local mode.".to_owned(),
        }
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
            AgentEvent::SkillSuggested {
                skill_id,
                skill_name,
                confidence,
                ..
            } => {
                let msg = format!(
                    "[skill] Suggested: {} (confidence: {:.0}%)",
                    skill_name,
                    confidence * 100.0
                );
                self.push_chat(MessageRole::Activity, msg.clone());
                self.record_tool_event(
                    "skill",
                    skill_id,
                    format!("{} ({})", skill_name, confidence),
                );
            }
            AgentEvent::ProviderFailover { from, to, reason } => {
                let msg = format!("[failover] {} → {} ({})", from, to, reason);
                self.push_chat(MessageRole::Activity, msg.clone());
                self.record_tool_event("failover", format!("{}->{}", from, to), reason);
            }
            AgentEvent::BudgetWarning {
                spent_usd,
                limit_usd,
            } => {
                let pct = ((spent_usd / limit_usd) * 100.0).round() as u32;
                let msg = format!(
                    "[budget warning] ${:.2} spent of ${:.2} ({pct}%)",
                    spent_usd, limit_usd
                );
                self.push_chat(MessageRole::Error, msg.clone());
                self.record_tool_event("budget", "warning", msg);
            }
            AgentEvent::BudgetExceeded {
                spent_usd,
                limit_usd,
            } => {
                let msg = format!(
                    "[budget exceeded] ${:.2} spent exceeds limit ${:.2}",
                    spent_usd, limit_usd
                );
                self.push_chat(MessageRole::Error, msg.clone());
                self.record_tool_event("budget", "exceeded", msg);
            }
            AgentEvent::PromptCacheHit { tokens_saved } => {
                let msg = format!("[cache hit] {} tokens saved", tokens_saved);
                self.record_tool_event("cache", "prompt", msg);
            }
            AgentEvent::PermissionDenied {
                tool_name,
                role,
                reason,
            } => {
                let msg = format!("[DENIED] {} ({}) — {}", tool_name, role, reason);
                self.push_chat(MessageRole::Error, msg.clone());
                self.record_tool_event("permission", "denied", msg);
            }
            AgentEvent::ValidationFailed { input: _, reason } => {
                let msg = format!("[VALIDATION ERROR] {}", reason);
                self.push_chat(MessageRole::Error, msg.clone());
                self.record_tool_event("validation", "failed", msg);
            }
            AgentEvent::BreakpointHit {
                step_index,
                tool_name,
            } => {
                let msg = format!("[BREAKPOINT] step {} — {}", step_index, tool_name);
                self.push_chat(MessageRole::Activity, msg.clone());
                self.record_tool_event("breakpoint", "hit", msg);
            }
            AgentEvent::ToolDryRun {
                id,
                name,
                synthetic_output,
            } => {
                self.push_chat(
                    MessageRole::System,
                    format!("[dry-run] {name} ({id}):\n{synthetic_output}"),
                );
            }
            AgentEvent::EvidenceReport {
                tool_name,
                evidence,
            } => {
                let formatted = format_evidence_report(&tool_name, &evidence);
                self.last_evidence = Some(EvidenceSnapshot {
                    tool_name: tool_name.clone(),
                    evidence: evidence.clone(),
                    formatted: formatted.clone(),
                });
                self.push_chat(MessageRole::System, formatted);
            }
            AgentEvent::RunFinished { .. }
            | AgentEvent::RunFailed { .. }
            | AgentEvent::TextDelta { .. }
            | AgentEvent::PlanCacheHit { .. }
            | AgentEvent::PlanStarted { .. }
            | AgentEvent::PlanFinished { .. } => {}
        }
    }

    fn push_chat(&mut self, role: MessageRole, text: impl Into<String>) {
        let text: String = text.into();
        self.session.chat.push(ChatMessage { role, text });
        self.session.transcript_scroll = 0;
    }

    /// Write a hash-chained audit event for a TUI apply-plan approval.
    fn write_apply_plan_audit(plan_id: &str, decision: &str) -> Result<()> {
        let db_path = crate::default_db_path()?;
        let conn = rusqlite::Connection::open(&db_path)?;
        let prev =
            helm_memory::latest_audit_hash(&conn, None).unwrap_or_else(|_| "GENESIS".to_string());
        let ts = chrono::Utc::now().timestamp_millis();
        let hash = helm_memory::audit_hash(helm_memory::AuditHashParts {
            previous_hash: &prev,
            episode_id: Some("tui-apply"),
            target: Some("tui"),
            timestamp: ts,
            tool_name: "apply-plan",
            input_hash: &helm_memory::stable_hash_hex(plan_id),
            output_hash: &helm_memory::stable_hash_hex(decision),
            capability: "shell",
            taint: "clean",
            cwd: "",
            decision,
        });
        conn.execute(
            "INSERT INTO audit_events (episode_id, target, timestamp, tool_name, input_hash, output_hash, capability, taint, cwd, decision, previous_hash, event_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params!["tui-apply", "tui", ts, "apply-plan", &helm_memory::stable_hash_hex(plan_id), &helm_memory::stable_hash_hex(decision), "shell", "clean", "", decision, &prev, &hash],
        )?;
        Ok(())
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast_variant(text, ToastVariant::Info);
    }

    fn toast_variant(&mut self, text: impl Into<String>, variant: ToastVariant) {
        self.toast = Some(ToastState {
            text: sanitize_one_line(&text.into()),
            created: Instant::now(),
            variant,
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
        let text = format!("◷ {name}: {summary} ...");
        self.status_note = format!("running {name}");
        self.session.chat.push(ChatMessage {
            role: MessageRole::Activity,
            text: sanitize_display_text(&text),
        });
        self.active_tool_cells
            .insert(id.to_owned(), self.session.chat.len().saturating_sub(1));
        self.tool_start_times.insert(id.to_owned(), Instant::now());
        self.session.transcript_scroll = 0;
    }

    fn finish_tool_cell(&mut self, id: &str, name: &str, success: bool, content: &str) {
        let summary = self
            .pending_tool_summaries
            .remove(id)
            .unwrap_or_else(|| name.to_owned());
        let duration = self
            .tool_start_times
            .remove(id)
            .map(|start| start.elapsed())
            .map(format_duration)
            .unwrap_or_default();
        let preview = tool_output_preview(content);
        let icon = if success { "✓" } else { "✗" };
        let text = if success {
            if preview.is_empty() {
                format!("{icon} {name}: {summary}  {duration}")
            } else {
                format!("{icon} {name}: {summary}  {duration}\n{preview}")
            }
        } else if preview.is_empty() {
            format!("{icon} {name} failed: {summary}  {duration}")
        } else {
            format!("{icon} {name} failed: {summary}  {duration}\n{preview}")
        };
        self.status_note = if success {
            format!("{name} ok {duration}")
        } else {
            format!("{name} failed {duration}")
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
        self.resume_context = None;
        self.input.clear();
        self.push_chat(MessageRole::System, "New session started.");
        self.modal = None;
    }

    fn clear_transcript(&mut self) {
        self.session.chat.clear();
        self.session.transcript_scroll = 0;
        self.push_chat(MessageRole::System, "Transcript cleared.");
    }

    fn env_candidates_for(&self, choice: ProviderChoice) -> Vec<String> {
        match choice {
            ProviderChoice::Gemini => {
                let mut names = Vec::new();
                let primary = if choice == self.active_settings.choice {
                    self.active_settings
                        .api_key_env
                        .clone()
                        .unwrap_or_else(|| "GOOGLE_API_KEY".to_owned())
                } else {
                    "GOOGLE_API_KEY".to_owned()
                };
                names.push(primary);
                if !names.iter().any(|name| name == "GEMINI_API_KEY") {
                    names.push("GEMINI_API_KEY".to_owned());
                }
                names
            }
            _ => default_api_key_env(choice)
                .map(|name| vec![name.to_owned()])
                .unwrap_or_default(),
        }
    }

    fn provider_key_status(&self, choice: ProviderChoice) -> ProviderKeyStatus {
        if choice == ProviderChoice::Ollama {
            return ProviderKeyStatus::NoKeyNeeded;
        }
        let env_names = self.env_candidates_for(choice);
        let has_env = env_names.iter().any(|name| std::env::var(name).is_ok());
        let has_stored = env_names
            .iter()
            .any(|name| self.runtime.secrets.get(name).ok().flatten().is_some());
        let has_session = choice == self.active_settings.choice
            && self
                .active_settings
                .api_key
                .as_ref()
                .is_some_and(|key| !key.trim().is_empty());

        if has_stored {
            return ProviderKeyStatus::Stored;
        }
        if has_env {
            return ProviderKeyStatus::Env;
        }
        if has_session {
            return ProviderKeyStatus::Session;
        }
        ProviderKeyStatus::Unset
    }

    fn resolved_provider_key(&self, choice: ProviderChoice) -> Option<String> {
        if choice == self.active_settings.choice
            && let Some(key) = self.active_settings.api_key.as_ref()
            && !key.trim().is_empty()
        {
            return Some(key.clone());
        }
        for env_name in self.env_candidates_for(choice) {
            if let Ok(Some(secret)) = self.runtime.secrets.get(&env_name) {
                return Some(secret.expose().to_owned());
            }
        }
        for env_name in self.env_candidates_for(choice) {
            if let Ok(value) = std::env::var(&env_name)
                && !value.trim().is_empty()
            {
                return Some(value);
            }
        }
        None
    }

    fn model_catalog_entries(&mut self) -> Vec<ModelCatalogEntry> {
        let now = Instant::now();
        let mut all = Vec::new();
        for (choice, _) in provider_selector_list() {
            let entries = if let Some(cached) = self.catalog_cache.get(&choice) {
                if now.duration_since(cached.fetched_at) <= Duration::from_secs(5 * 60) {
                    cached.entries.clone()
                } else {
                    self.refresh_catalog_for(choice)
                }
            } else {
                self.refresh_catalog_for(choice)
            };
            all.extend(entries);
        }
        all.sort_by(|left, right| {
            left.group
                .cmp(&right.group)
                .then_with(|| left.label.cmp(&right.label))
        });
        all
    }

    fn refresh_catalog_for(&mut self, choice: ProviderChoice) -> Vec<ModelCatalogEntry> {
        let entries = tokio::task::block_in_place(|| {
            Handle::current().block_on(fetch_model_catalog_for_provider(
                choice,
                &self.active_settings,
                self.resolved_provider_key(choice),
            ))
        })
        .unwrap_or_else(|_| static_model_catalog_for(choice));
        self.catalog_cache.insert(
            choice,
            CachedModelCatalog {
                fetched_at: Instant::now(),
                entries: entries.clone(),
            },
        );
        entries
    }

    fn apply_provider_choice(&mut self, choice: ProviderChoice) {
        let resolved_key = self.resolved_provider_key(choice);

        if choice == ProviderChoice::Ollama || resolved_key.is_some() {
            self.apply_provider_with_key(choice, resolved_key.unwrap_or_default(), false);
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
        settings.model = Some(entry.model.clone());
        if settings.choice != self.active_settings.choice {
            settings.api_key = None;
        }
        self.active_settings = settings;
        self.provider_name = provider_choice_name(entry.provider).to_owned();
        self.model = entry.model.clone();
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

    fn slash_filtered(&self) -> Vec<SlashItem> {
        let raw = self.input.text.trim_start_matches('/').to_ascii_lowercase();
        let query = raw.split_whitespace().next().unwrap_or("");
        let mut items = CommandAction::all()
            .into_iter()
            .filter(|action| action.slug().starts_with(query) || action.matches_slug(query))
            .map(SlashItem::BuiltIn)
            .collect::<Vec<_>>();
        items.extend(self.custom_commands.iter().cloned().map(SlashItem::Custom));
        items
            .into_iter()
            .filter(|item| item.slug().starts_with(query))
            .collect()
    }

    fn filtered_palette_items(&self) -> Vec<PaletteItem> {
        let query = self.command_palette.query.as_str();
        let mut items = CommandAction::all()
            .into_iter()
            .map(PaletteItem::BuiltIn)
            .collect::<Vec<_>>();
        items.extend(
            self.custom_commands
                .iter()
                .cloned()
                .map(PaletteItem::Custom),
        );
        items
            .into_iter()
            .filter(|item| item.matches_query(query))
            .collect()
    }

    async fn execute_palette_item(
        &mut self,
        item: PaletteItem,
        tx: mpsc::UnboundedSender<UiEvent>,
    ) -> Result<()> {
        match item {
            PaletteItem::BuiltIn(command) => self.execute_command(command),
            PaletteItem::Custom(command) => {
                let expanded = custom_commands::expand(&command, "");
                self.modal = None;
                self.start_task(expanded, tx, true).await?;
            }
        }
        Ok(())
    }

    async fn execute_custom_command(
        &mut self,
        command: custom_commands::CustomCommand,
        args: &str,
        tx: mpsc::UnboundedSender<UiEvent>,
    ) -> Result<()> {
        let expanded = custom_commands::expand(&command, args);
        self.start_task(expanded, tx, true).await
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

    async fn execute_slash_from_popup(&mut self, tx: mpsc::UnboundedSender<UiEvent>) -> Result<()> {
        let filtered = self.slash_filtered();
        if let Some(sel) = self.slash_popup {
            if let Some(cmd) = filtered.get(sel).cloned() {
                self.input.clear();
                self.slash_popup = None;
                match cmd {
                    SlashItem::BuiltIn(command) => self.execute_command(command),
                    SlashItem::Custom(command) => {
                        self.execute_custom_command(command, "", tx).await?
                    }
                }
            }
        }
        Ok(())
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
            CommandAction::Remote => self.push_chat(MessageRole::System, self.remote_hint()),
            CommandAction::Provider => {
                let current = provider_selector_list()
                    .iter()
                    .position(|(c, _)| *c == self.active_settings.choice)
                    .unwrap_or(0);
                self.modal = Some(ModalState::ProviderSelector { selected: current });
            }
            CommandAction::Model => {
                let entries = self.model_catalog_entries();
                self.modal = Some(ModalState::ModelSelector {
                    query: String::new(),
                    selected: entries
                        .iter()
                        .position(|entry| {
                            entry.provider == self.active_settings.choice
                                && entry.model == self.model.as_str()
                        })
                        .unwrap_or(0),
                    entries,
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
            CommandAction::Sessions => {
                let body = self.render_sessions_inline(20);
                self.push_chat(
                    MessageRole::System,
                    format!("Sessions (recent 20):\n{body}"),
                );
            }
            CommandAction::Resume => self.execute_resume_inline(""),
            CommandAction::Config => self.execute_config_inline(""),
            CommandAction::Theme => self.open_theme_selector(),
            CommandAction::Keybindings => self.execute_keybindings_inline(),
            CommandAction::Stats => self.execute_stats_inline(),
            CommandAction::Mcp => self.execute_mcp_list_inline(),
            CommandAction::Compact => self.execute_compact_inline(),
            CommandAction::Diff => self.execute_diff_inline(""),
            CommandAction::Tools => self.execute_tools_inline(),
            CommandAction::Undo => self.execute_undo_inline(false),
            CommandAction::Redo => self.execute_undo_inline(true),
            CommandAction::Cost => self.open_cost_meter(),
            CommandAction::Diagnose => {
                self.mode = AgentMode::Diagnose;
                self.push_chat(
                    MessageRole::System,
                    "Diagnose mode enabled — only read-only tools available. \
                     Run a task to begin."
                        .to_owned(),
                );
            }
            CommandAction::Evidence => match &self.last_evidence {
                Some(ev) => {
                    self.push_chat(
                        MessageRole::System,
                        format!("Evidence report for {}:\n{}", ev.tool_name, ev.formatted),
                    );
                }
                None => {
                    self.push_chat(
                        MessageRole::System,
                        "No evidence report available yet. \
                         Run a task with --evidence to see system state."
                            .to_owned(),
                    );
                }
            },
            CommandAction::Quit => self.shutdown = true,
            CommandAction::Help => self.modal = Some(ModalState::Help),
            CommandAction::ApplyPlan => {
                // ApplyPlan requires args; fallback to hint
                self.push_chat(
                    MessageRole::System,
                    "Usage: /apply-plan <plan_id>\n  Apply a troubleshooting plan with step-by-step approval.",
                );
            }
            CommandAction::Dashboard => {
                self.mode = AgentMode::Dashboard;
                self.refresh_dashboard();
                self.toast("Dashboard mode");
            }
        }
    }

    fn execute_command_with_args(&mut self, command: CommandAction, args: &str) {
        self.modal = None;
        let args = args.trim();
        match command {
            CommandAction::Config if !args.is_empty() => self.execute_config_inline(args),
            CommandAction::Theme if !args.is_empty() => self.apply_theme_by_name(args),
            CommandAction::Diff if !args.is_empty() => self.execute_diff_inline(args),
            CommandAction::Sessions if !args.is_empty() => {
                let limit = args.parse::<u32>().unwrap_or(20);
                let body = self.render_sessions_inline(limit);
                self.push_chat(
                    MessageRole::System,
                    format!("Sessions (recent {limit}):\n{body}"),
                );
            }
            CommandAction::Resume => self.execute_resume_inline(args),
            CommandAction::Compact if !args.is_empty() => {
                self.push_chat(
                    MessageRole::System,
                    format!("[compact] hint noted: {args}. Transcript truncated to recent turns."),
                );
                self.execute_compact_inline();
            }
            CommandAction::ApplyPlan if !args.is_empty() => {
                self.execute_apply_plan_inline(args);
            }
            _ => self.execute_command(command),
        }
    }

    fn execute_config_inline(&mut self, raw: &str) {
        let path = self.runtime.config_path.clone();
        if raw.is_empty() {
            match std::fs::read_to_string(&path) {
                Ok(text) if text.trim().is_empty() => self.push_chat(
                    MessageRole::System,
                    format!("[config] {} is empty. Use `/config key.path=value` to set.", path.display()),
                ),
                Ok(text) => {
                    let trimmed: String = text.lines().take(60).collect::<Vec<_>>().join("\n");
                    self.push_chat(
                        MessageRole::System,
                        format!(
                            "[config] {} (first 60 lines)\n{trimmed}\nEdit via `/config key.path=value` or run `helm config edit`.",
                            path.display()
                        ),
                    );
                }
                Err(_) => self.push_chat(
                    MessageRole::System,
                    format!(
                        "[config] {} does not exist yet. Set a value with `/config key.path=value` (file will be created).",
                        path.display()
                    ),
                ),
            }
            return;
        }
        let Some(eq_idx) = raw.find('=') else {
            self.push_chat(
                MessageRole::Error,
                "[config] expected `key.path=value`. Example: `/config providers.default=anthropic`",
            );
            return;
        };
        let (key, value) = raw.split_at(eq_idx);
        let key = key.trim();
        let value = value[1..].trim();
        if key.is_empty() {
            self.push_chat(MessageRole::Error, "[config] empty key");
            return;
        }
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let mut root: toml::Value = existing
            .parse()
            .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        let parts: Vec<&str> = key.split('.').collect();
        let parsed_val = if value == "true" {
            toml::Value::Boolean(true)
        } else if value == "false" {
            toml::Value::Boolean(false)
        } else if let Ok(i) = value.parse::<i64>() {
            toml::Value::Integer(i)
        } else if let Ok(f) = value.parse::<f64>() {
            toml::Value::Float(f)
        } else {
            toml::Value::String(value.to_owned())
        };
        if let Err(error) = crate::set_toml_path(&mut root, &parts, parsed_val) {
            self.push_chat(MessageRole::Error, format!("[config] {error}"));
            return;
        }
        let pretty = match toml::to_string_pretty(&root) {
            Ok(text) => text,
            Err(error) => {
                self.push_chat(MessageRole::Error, format!("[config] serialize: {error}"));
                return;
            }
        };
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            self.push_chat(MessageRole::Error, format!("[config] create dir: {error}"));
            return;
        }
        if let Err(error) = std::fs::write(&path, pretty) {
            self.push_chat(MessageRole::Error, format!("[config] write: {error}"));
            return;
        }
        self.push_chat(
            MessageRole::System,
            format!("[config] {key} = {value} → {}", path.display()),
        );
    }

    fn open_theme_selector(&mut self) {
        let themes = Theme::all();
        let current = themes
            .iter()
            .position(|t| t.name() == self.theme.name())
            .unwrap_or(0);
        self.modal = Some(ModalState::ThemeSelector { selected: current });
    }

    fn apply_theme_by_name(&mut self, name: &str) {
        let needle = name.trim().to_ascii_lowercase();
        if let Some(theme) = Theme::all()
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(&needle))
        {
            let label = theme.name().to_owned();
            self.theme = theme;
            self.push_chat(
                MessageRole::System,
                format!("[theme] switched to {label}. Persist via `helm config set theme = \"{label}\"`."),
            );
        } else {
            let names: Vec<&str> = Theme::all().iter().map(|t| t.name()).collect();
            self.push_chat(
                MessageRole::Error,
                format!("[theme] unknown `{name}`. Choose: {}", names.join(", ")),
            );
        }
    }

    fn execute_keybindings_inline(&mut self) {
        let path = match crate::keybindings::config_path() {
            Ok(p) => p,
            Err(error) => {
                self.push_chat(MessageRole::Error, format!("[keybindings] {error}"));
                return;
            }
        };
        let exists = path.exists();
        let count = self.keymap.map.len();
        let actions = [
            "send",
            "newline",
            "quit",
            "palette",
            "tool_sidebar",
            "history",
            "clear",
            "cancel",
            "help",
            "external_editor",
            "verbose_toggle",
        ];
        let known = actions.join(", ");
        let body = if exists {
            format!(
                "[keybindings] {} ({} override(s) loaded)\nKnown actions: {}\nEdit via `$EDITOR {}` and reload HELM.",
                path.display(),
                count,
                known,
                path.display()
            )
        } else {
            format!(
                "[keybindings] no overrides at {}.\nKnown actions: {}\nCreate a JSON map (`{{\"send\": \"Enter\"}}`) at that path to remap.",
                path.display(),
                known
            )
        };
        self.push_chat(MessageRole::System, body);
    }

    fn execute_stats_inline(&mut self) {
        let memory = Arc::clone(&self.runtime.memory);
        let summary = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let counts = match memory.episode_outcome_counts().await {
                    Ok(c) => c,
                    Err(error) => return format!("(stats unavailable: {error})"),
                };
                let recents = memory.recent_episodes(50).await.unwrap_or_default();
                let tokens_in: u64 = recents.iter().map(|e| e.tokens_in as u64).sum();
                let tokens_out: u64 = recents.iter().map(|e| e.tokens_out as u64).sum();
                format!(
                    "episodes: {} (ok {} / partial {} / fail {})\nlast {} runs · tokens in {} · tokens out {}",
                    counts.total,
                    counts.success,
                    counts.partial,
                    counts.failure,
                    recents.len(),
                    tokens_in,
                    tokens_out
                )
            })
        });
        self.push_chat(
            MessageRole::System,
            format!("[stats]\n{summary}\nRun `helm stats` for the daemon-side rollup."),
        );
    }

    fn execute_mcp_list_inline(&mut self) {
        let path_label = helm_tools::mcp::default_mcp_config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(no XDG config path)".to_owned());
        let body = match helm_tools::mcp::load_mcp_config() {
            Ok(cfg) if cfg.servers.is_empty() => {
                format!("no MCP servers configured at {path_label}. Run `helm mcp add`.")
            }
            Ok(cfg) => {
                let lines: Vec<String> = cfg
                    .servers
                    .iter()
                    .map(|s| format!("  {} → {}", s.name, s.command))
                    .collect();
                format!("MCP servers ({path_label}):\n{}", lines.join("\n"))
            }
            Err(error) => format!("(failed to load MCP config at {path_label}: {error})"),
        };
        self.push_chat(MessageRole::System, format!("[mcp] {body}"));
    }

    fn execute_compact_inline(&mut self) {
        let total = self.session.chat.len();
        const KEEP: usize = 12;
        if total <= KEEP {
            self.push_chat(
                MessageRole::System,
                format!("[compact] only {total} messages — nothing to compact."),
            );
            return;
        }
        let dropped = total - KEEP;
        self.session.chat.drain(..dropped);
        self.session.chat.insert(
            0,
            ChatMessage {
                role: MessageRole::System,
                text: format!("[compact] {dropped} earlier turns folded; last {KEEP} kept."),
            },
        );
        self.session.transcript_scroll = 0;
    }

    fn session_store(&self) -> Result<helm_memory::SessionStore, String> {
        let sessions_dir = self
            .runtime
            .db_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(helm_memory::SessionStore::open(
                    &self.runtime.db_path,
                    sessions_dir.join("snapshots"),
                ))
                .map_err(|error| error.to_string())
        })
    }

    fn render_sessions_inline(&self, limit: u32) -> String {
        match self.session_store() {
            Ok(store) => tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    match store.list_sessions(limit).await {
                        Ok(list) if list.is_empty() => "(no sessions yet)".to_owned(),
                        Ok(list) => list
                            .into_iter()
                            .map(|s| {
                                format!(
                                    "[{}] {} — {}",
                                    s.id,
                                    s.name,
                                    s.goal.chars().take(60).collect::<String>()
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        Err(error) => format!("error listing sessions: {error}"),
                    }
                })
            }),
            Err(error) => format!("error opening session store: {error}"),
        }
    }

    fn execute_resume_inline(&mut self, raw: &str) {
        let target = raw.trim();
        let store = match self.session_store() {
            Ok(store) => store,
            Err(error) => {
                self.push_chat(MessageRole::Error, format!("resume failed: {error}"));
                return;
            }
        };
        let target = target.to_owned();
        let memory = Arc::clone(&self.runtime.memory);
        let loaded = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let session = if target.is_empty() || target == "latest" {
                    store.latest_session().await
                } else {
                    store.get_session(&target).await
                }?;
                let Some(session) = session else {
                    return Ok::<
                        Option<(
                            helm_memory::SessionRecord,
                            Option<helm_memory::EpisodeRecord>,
                            Vec<helm_memory::StepRecord>,
                        )>,
                        helm_core::MemoryError,
                    >(None);
                };
                let episode = memory.episode_by_id(&session.episode_id).await?;
                let steps = memory.get_steps(&session.episode_id).await?;
                Ok(Some((session, episode, steps)))
            })
        });
        match loaded {
            Ok(Some((session, episode, steps))) => {
                let recap = crate::build_session_recap(&session, episode.as_ref(), &steps);
                self.resume_context = Some(recap.clone());
                self.session.episode_id = Some(session.episode_id.clone());
                self.push_chat(
                    MessageRole::System,
                    format!(
                        "{recap}\nFuture prompts will include this session context until `/new`."
                    ),
                );
            }
            Ok(None) => self.push_chat(MessageRole::System, "No sessions available to resume."),
            Err(error) => self.push_chat(MessageRole::Error, format!("resume failed: {error}")),
        }
    }

    fn execute_diff_inline(&mut self, path_arg: &str) {
        let target = path_arg.trim();
        if target.is_empty() {
            self.push_chat(
                MessageRole::System,
                "[diff] usage: `/diff <path>` — prints first 80 lines of the file. \
                 Snapshots are taken on every fs_write; restore with `helm undo`.",
            );
            return;
        }
        let target_path = std::path::PathBuf::from(target);
        match std::fs::read_to_string(&target_path) {
            Ok(text) => {
                let preview: String = text.lines().take(80).collect::<Vec<_>>().join("\n");
                self.push_chat(
                    MessageRole::System,
                    format!(
                        "[diff:{}] (first 80 lines)\n{preview}",
                        target_path.display()
                    ),
                );
            }
            Err(error) => self.push_chat(
                MessageRole::Error,
                format!("[diff] cannot read {}: {error}", target_path.display()),
            ),
        }
    }

    fn execute_tools_inline(&mut self) {
        let registry = helm_tools::ToolRegistry::default();
        let schemas = registry.schemas();
        let count = schemas.len();
        let mut names: Vec<String> = schemas.iter().map(|s| s.name.clone()).collect();
        names.sort();
        let body = names
            .chunks(4)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|n| format!("{:<14}", n))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.push_chat(
            MessageRole::System,
            format!("[tools] {count} loaded:\n{body}"),
        );
    }

    fn execute_undo_inline(&mut self, redo: bool) {
        let verb = if redo { "redo" } else { "undo" };
        self.push_chat(
            MessageRole::System,
            format!(
                "[{verb}] run from CLI: `helm undo --apply` (or `--apply --to <path>`). \
                 Each fs_write is auto-snapshot'd in the session store."
            ),
        );
    }

    fn execute_apply_plan_inline(&mut self, plan_id: &str) {
        // Load plan from database and show preview in transcript
        let db_path = match default_db_path() {
            Ok(p) => p,
            Err(e) => {
                self.push_chat(MessageRole::Error, format!("[apply-plan] DB error: {e}"));
                return;
            }
        };
        let conn = match rusqlite::Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => {
                self.push_chat(
                    MessageRole::Error,
                    format!("[apply-plan] DB open failed: {e}"),
                );
                return;
            }
        };
        let record = match TroubleshootingPlanStore::get(&conn, plan_id) {
            Ok(Some(r)) => r,
            Ok(None) => {
                self.push_chat(
                    MessageRole::Error,
                    format!(
                        "[apply-plan] plan '{plan_id}' not found. Run `helm troubleshoot` first."
                    ),
                );
                return;
            }
            Err(e) => {
                self.push_chat(
                    MessageRole::Error,
                    format!("[apply-plan] DB query error: {e}"),
                );
                return;
            }
        };

        // Parse and display steps
        let steps: Vec<serde_json::Value> =
            serde_json::from_str(&record.proposed_fix_steps_json).unwrap_or_default();
        let step_count = steps.len();
        if step_count == 0 {
            self.push_chat(
                MessageRole::System,
                "[apply-plan] Plan has no fix steps to execute.",
            );
            return;
        }

        let title = if record.source.starts_with("user:") {
            record.source.trim_start_matches("user:").trim().to_string()
        } else {
            record.source.clone()
        };

        let mut body = String::new();
        body.push_str(&format!("Plan: {title} ({plan_id})\n"));
        body.push_str(&format!(
            "Snapshot: {} | {} fix steps\n\n",
            record.snapshot_id, step_count
        ));
        for (i, s) in steps.iter().enumerate() {
            let cmd = s["command"]["command_text"]
                .as_str()
                .unwrap_or("(no command)");
            let effect = s["command"]["expected_effect"].as_str().unwrap_or("");
            let risk = s["command"]["risk"].as_str().unwrap_or("none");
            let tool = s["command"]["tool"].as_str().unwrap_or("shell");
            body.push_str(&format!("  {}. [{risk}] {tool}: {cmd}\n", i + 1));
            body.push_str(&format!("     Effect: {effect}\n"));
        }
        body.push_str("\nUse `y` to approve, `n` to skip, `!` to approve all, `Esc` to cancel.");

        self.push_chat(
            MessageRole::System,
            format!("[apply-plan] Loaded plan:\n{body}"),
        );

        // Enter approval modal with step-by-step flow
        let previews: Vec<String> = steps
            .iter()
            .map(|s| {
                s["command"]["command_text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        let effects: Vec<String> = steps
            .iter()
            .map(|s| {
                s["command"]["expected_effect"]
                    .as_str()
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        let tools: Vec<String> = steps
            .iter()
            .map(|s| s["command"]["tool"].as_str().unwrap_or("shell").to_string())
            .collect();
        let risks: Vec<String> = steps
            .iter()
            .map(|s| s["command"]["risk"].as_str().unwrap_or("none").to_string())
            .collect();

        self.modal = Some(ModalState::PlanExecution {
            plan_id: plan_id.to_string(),
            plan_title: title,
            step_index: 0,
            step_count,
            step_previews: previews,
            step_effects: effects,
            step_tools: tools,
            step_risks: risks,
            phase: PlanExecPhase::Approving,
            result_summary: String::new(),
        });
    }

    /// Reload dashboard data from the latest snapshot.
    fn refresh_dashboard(&mut self) {
        use helm_memory::SnapshotStore;
        use helm_monitor::SnapshotDomains;

        let db_path = match crate::default_db_path() {
            Ok(p) => p,
            Err(e) => {
                self.dashboard.error = Some(format!("db error: {e}"));
                return;
            }
        };
        let conn = match rusqlite::Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => {
                self.dashboard.error = Some(format!("db open: {e}"));
                return;
            }
        };
        let record = match SnapshotStore::latest(&conn) {
            Ok(Some(r)) => r,
            Ok(None) => {
                self.dashboard.error =
                    Some("no snapshots yet. Run `helm snapshot` or `helm monitor` first.".into());
                return;
            }
            Err(e) => {
                self.dashboard.error = Some(format!("snapshot error: {e}"));
                return;
            }
        };
        let domains: SnapshotDomains =
            serde_json::from_str(&record.domains_json).unwrap_or_default();

        let hostname = record.host_hostname;
        let load = &domains.load;
        let mem = &load.memory;
        let memory_used_pct = if mem.total > 0 {
            mem.used as f64 / mem.total as f64 * 100.0
        } else {
            0.0
        };
        let disk_entries: Vec<String> = domains
            .disks
            .filesystems
            .iter()
            .map(|fs| {
                let pct = if fs.total_bytes > 0 {
                    fs.used_bytes as f64 / fs.total_bytes as f64 * 100.0
                } else {
                    0.0
                };
                format!("{} {:.0}%", fs.mount_point, pct)
            })
            .collect();
        let total_services = domains.services.units.len();
        let failed_services = domains.services.failed_units.len();
        let total_containers = domains.containers.containers.len();
        let running_containers = domains
            .containers
            .containers
            .iter()
            .filter(|c| c.status == "running")
            .count();
        let listening_ports = domains.ports.listeners.len();
        let last_log_errors = domains.logs.journal_errors_last_hour;
        let backup_count = domains.backups.tools_detected.len();
        let findings: Vec<serde_json::Value> =
            serde_json::from_str(&record.findings_json).unwrap_or_default();
        let finding_count = findings.len();
        let finding_warnings = findings
            .iter()
            .filter(|f| f["severity"].as_str() == Some("warning"))
            .count();
        let collected_at = chrono::DateTime::from_timestamp(record.collected_at, 0)
            .map(|dt| dt.format("%H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "unknown".into());

        self.dashboard.data = DashboardData {
            hostname,
            load_1m: load.load_average.one,
            load_5m: load.load_average.five,
            load_15m: load.load_average.fifteen,
            memory_used_pct,
            disk_entries,
            total_services,
            failed_services,
            total_containers,
            running_containers,
            listening_ports,
            last_log_errors,
            backup_count,
            finding_count,
            finding_warnings,
            collected_at,
        };
        self.dashboard.error = None;
    }

    fn open_cost_meter(&mut self) {
        let memory = Arc::clone(&self.runtime.memory);
        let (tokens_in, tokens_out) = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let recents = memory.recent_episodes(20).await.unwrap_or_default();
                (
                    recents.iter().map(|e| e.tokens_in as u64).sum::<u64>(),
                    recents.iter().map(|e| e.tokens_out as u64).sum::<u64>(),
                )
            })
        });
        // Very rough USD estimate using $5 per 1M in, $15 per 1M out (Anthropic mid-tier baseline).
        let cost_usd =
            (tokens_in as f64 / 1_000_000.0) * 5.0 + (tokens_out as f64 / 1_000_000.0) * 15.0;
        self.modal = Some(ModalState::CostMeter {
            session_tokens_in: tokens_in,
            session_tokens_out: tokens_out,
            session_cost_usd: cost_usd,
        });
    }

    fn apply_remote_target(&mut self, args: &str) {
        let target = args.trim();
        if target.is_empty() {
            self.push_chat(MessageRole::System, self.remote_hint());
            return;
        }
        if matches!(target, "off" | "none" | "local") {
            self.active_remote = None;
            self.push_chat(
                MessageRole::System,
                "Remote target cleared. HELM will run locally.",
            );
            return;
        }
        let registry = match RemoteRegistry::load() {
            Ok(registry) => registry,
            Err(error) => {
                self.push_chat(
                    MessageRole::Error,
                    format!("Failed to load remote registry: {error}"),
                );
                return;
            }
        };
        if registry.get(target).is_none() {
            self.push_chat(
                MessageRole::Error,
                format!("Unknown remote target `{target}`. Use `helm remote list` to inspect registered targets."),
            );
            return;
        }
        self.active_remote = Some(target.to_owned());
        self.push_chat(
            MessageRole::System,
            format!("Remote target set to `{target}`. New tasks will run against that host."),
        );
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
    remote_target: Option<String>,
) -> Result<RunResult, HelmError> {
    let (provider, model) = build_provider(&settings, &runtime.secrets)
        .map_err(|error| HelmError::Provider(helm_core::ProviderError::Other(error.to_string())))?;
    let mut budget = Budget::default();
    if let Some(max) = runtime.max_iterations {
        budget.max_iterations = max;
    }
    budget.read_only = mode == AgentMode::Plan || mode == AgentMode::Diagnose;
    budget.dry_run = false;
    budget.auto_approve = mode == AgentMode::AutoAccept;
    let mut tool_context = helm_tools::ToolContext::new(
        runtime
            .sandbox
            .as_ref()
            .map(|resolved| resolved.root_dir.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
    );
    if let Some(policy) = runtime.sandbox.as_ref() {
        tool_context = tool_context.with_sandbox(policy.policy());
    }
    if let Some(remote_target) = remote_target {
        tool_context = tool_context.with_remote_target(remote_target);
    }
    let tool_registry = if mode == AgentMode::Diagnose {
        tool_context = tool_context.with_diagnose_mode();
        ToolRegistry::with_diagnose_tools()
    } else {
        ToolRegistry::default()
    };
    let agent = ReactAgent::with_tool_context(
        provider,
        tool_registry,
        Arc::clone(&runtime.memory),
        budget,
        model,
        tool_context,
    );
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
    if app.mode == AgentMode::Dashboard {
        render_dashboard(app, vertical[1], buf);
    } else {
        render_chat(app, vertical[1], buf);
    }
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

/// Render the monitoring dashboard with compact system panels.
fn render_dashboard(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    if area.width < 40 || area.height < 10 {
        Paragraph::new("Dashboard needs a larger terminal")
            .style(Style::default().fg(DIM_FG))
            .render(area, buf);
        return;
    }

    if let Some(error) = &app.dashboard.error {
        Paragraph::new(error.as_str())
            .style(Style::default().fg(ERROR_FG))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Dashboard ")
                    .border_style(Style::default().fg(HEADER_BORDER)),
            )
            .render(area, buf);
        return;
    }

    let d = &app.dashboard.data;
    let sel = app.dashboard.selected;
    let panels = DashPanel::all();

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(area);

    for (col, col_area) in cols.iter().enumerate() {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(33),
                Constraint::Percentage(34),
            ])
            .split(*col_area);

        for (row, row_area) in rows.iter().enumerate() {
            let idx = col * 3 + row;
            if idx >= panels.len() {
                continue;
            }
            let panel = panels[idx];
            let is_sel = panel == sel;
            let text = render_dash_panel(panel, d);
            let title_style = if is_sel {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM_FG)
            };
            let border_style = if is_sel {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(HEADER_BORDER)
            };
            let block = Block::default()
                .title(Span::styled(format!(" {} ", panel.label()), title_style))
                .borders(Borders::ALL)
                .border_style(border_style);
            Paragraph::new(text)
                .block(block)
                .style(Style::default().fg(APP_FG).bg(APP_BG))
                .render(*row_area, buf);
        }
    }
}

fn render_dash_panel(panel: DashPanel, d: &DashboardData) -> String {
    match panel {
        DashPanel::Health => {
            format!(
                "Host: {}\nCollected: {}\n\nLoad:  {:.1} {:.1} {:.1}\nMemory: {:.0}%",
                d.hostname, d.collected_at, d.load_1m, d.load_5m, d.load_15m, d.memory_used_pct
            )
        }
        DashPanel::Findings => {
            let mut out = format!(
                "Total: {}\nWarnings: {}",
                d.finding_count, d.finding_warnings
            );
            if d.finding_count > 0 {
                out.push_str("\n\n> Enter to view");
            }
            out
        }
        DashPanel::Services => {
            let mut out = format!("Total: {}", d.total_services);
            if d.failed_services > 0 {
                out.push_str(&format!("\nFAILED: {}", d.failed_services));
            } else {
                out.push_str("\nAll active");
            }
            out
        }
        DashPanel::Containers => {
            format!(
                "Total: {}\nRunning: {}",
                d.total_containers, d.running_containers
            )
        }
        DashPanel::Disk => {
            let mut out = String::new();
            for entry in d.disk_entries.iter().take(4) {
                out.push_str(entry);
                out.push('\n');
            }
            if d.disk_entries.len() > 4 {
                out.push_str(&format!("... {} more", d.disk_entries.len() - 4));
            }
            out
        }
        DashPanel::Ports => {
            format!("Listening: {}", d.listening_ports)
        }
        DashPanel::Logs => {
            format!("Errors (1h): {}", d.last_log_errors)
        }
        DashPanel::Backups => {
            format!("Tools: {}", d.backup_count)
        }
        DashPanel::Plans => {
            let count = crate::default_db_path()
                .ok()
                .and_then(|p| rusqlite::Connection::open(p).ok())
                .and_then(|conn| {
                    helm_memory::TroubleshootingPlanStore::list(&conn, 100)
                        .ok()
                        .map(|plans| plans.len())
                })
                .unwrap_or(0);
            format!("Saved plans: {count}\n\n> Enter to view")
        }
    }
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
    let elapsed = app
        .task_started
        .map(|start| format_duration(start.elapsed()))
        .unwrap_or_default();
    let mode_style = match app.mode {
        AgentMode::Chat => Style::default()
            .fg(Color::White)
            .bg(HEADER_BORDER)
            .add_modifier(Modifier::BOLD),
        AgentMode::Plan => Style::default()
            .fg(Color::White)
            .bg(Color::Rgb(75, 85, 99))
            .add_modifier(Modifier::BOLD),
        AgentMode::AutoAccept => Style::default()
            .fg(Color::White)
            .bg(SUCCESS_FG)
            .add_modifier(Modifier::BOLD),
        AgentMode::Diagnose => Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        AgentMode::Dashboard => Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(80, 200, 120))
            .add_modifier(Modifier::BOLD),
    };
    let mut line = Line::from(vec![
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
            format!(
                " {} ",
                app.active_remote
                    .as_deref()
                    .map(|target| format!("remote {}", truncate(target, 16)))
                    .unwrap_or_else(|| "local".to_owned())
            ),
            Style::default().fg(TOOL_FG).bg(HEADER_BG),
        ),
        Span::styled(
            format!(" episode {} ", truncate(episode, 8)),
            Style::default().fg(DIM_FG).bg(HEADER_BG),
        ),
        Span::styled(
            format!(" [{}] ", app.mode.as_str().to_ascii_uppercase()),
            mode_style,
        ),
        Span::styled(
            format!(" {} ", token_status(app)),
            Style::default().fg(DIM_FG).bg(HEADER_BG),
        ),
        Span::styled(
            format!(" {} ", truncate(&app.status_note, 28)),
            Style::default()
                .fg(if app.running { SUCCESS_FG } else { APP_FG })
                .bg(HEADER_BG),
        ),
    ]);
    if !elapsed.is_empty() {
        line.push_span(Span::styled(
            format!(" ⏱ {elapsed} "),
            Style::default().fg(TOOL_FG).bg(HEADER_BG),
        ));
    }
    Paragraph::new(line)
        .style(Style::default().bg(HEADER_BG))
        .render(chunks[0], buf);
    Paragraph::new("─".repeat(chunks[1].width as usize))
        .style(Style::default().fg(HEADER_BORDER).bg(APP_BG))
        .render(chunks[1], buf);
}

fn render_chat(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let chat_empty = app.session.chat.is_empty();
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
    if chat_empty && !app.running {
        let welcome = vec![
            Line::from(vec![Span::styled(
                "  HELM v1.6 — Linux Operations Agent",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Type a task or question to begin. Examples:",
                Style::default().fg(DIM_FG),
            )]),
            Line::from(vec![Span::styled(
                "    • \"check disk usage on /\"",
                Style::default().fg(APP_FG),
            )]),
            Line::from(vec![Span::styled(
                "    • \"what's listening on port 443?\"",
                Style::default().fg(APP_FG),
            )]),
            Line::from(vec![Span::styled(
                "    • \"show me nginx errors since last hour\"",
                Style::default().fg(APP_FG),
            )]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "  /",
                    Style::default()
                        .fg(HEADER_BORDER)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" for commands  ", Style::default().fg(DIM_FG)),
                Span::styled(
                    "Shift+Tab",
                    Style::default()
                        .fg(HEADER_BORDER)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" to change mode  ", Style::default().fg(DIM_FG)),
                Span::styled(
                    "Ctrl+P",
                    Style::default()
                        .fg(HEADER_BORDER)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" for palette", Style::default().fg(DIM_FG)),
            ]),
        ];
        Paragraph::new(welcome)
            .block(
                Block::default()
                    .borders(Borders::LEFT | Borders::RIGHT)
                    .title(" Welcome ")
                    .border_style(Style::default().fg(HEADER_BORDER))
                    .style(Style::default().fg(APP_FG).bg(APP_BG)),
            )
            .render(area, buf);
        return;
    }
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
        app.slash_filtered()
            .into_iter()
            .filter(|item| item.slug().starts_with(query.as_str()))
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
        let placeholder = match app.mode {
            AgentMode::Diagnose => "Diagnose a system problem (read-only)...",
            AgentMode::Plan => "Plan an approach (no writes yet)...",
            AgentMode::AutoAccept => "Run with auto-approved tools...",
            AgentMode::Chat => "Ask HELM to do something...",
            AgentMode::Dashboard => "Dashboard — type a task or /command",
        };
        vec![Line::from(vec![
            Span::styled(
                "❯ ",
                Style::default().fg(USER_BAR).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                placeholder,
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
        AgentMode::Diagnose => "DIAGNOSE",
        AgentMode::Dashboard => "DASHBOARD",
    };
    let mode_hint = match _app.mode {
        AgentMode::Chat => "Shift+Tab -> Plan",
        AgentMode::Plan => "READ-ONLY | Shift+Tab -> Auto",
        AgentMode::AutoAccept => "AUTO-ACCEPT | Shift+Tab -> Diagnose",
        AgentMode::Diagnose => "DIAGNOSE | Shift+Tab -> Dashboard",
        AgentMode::Dashboard => "DASHBOARD | Shift+Tab -> Chat",
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
                let key_status = app.provider_key_status(*choice).label(*env_key);
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
        ModalState::ModelSelector {
            query,
            selected,
            entries,
        } => {
            let entries = filtered_model_catalog(entries, query);
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
                if entry.group.as_str() != last_group {
                    if !last_group.is_empty() {
                        lines.push(Line::from(""));
                    }
                    lines.push(Line::styled(
                        entry.group.as_str(),
                        Style::default()
                            .fg(Color::LightMagenta)
                            .add_modifier(Modifier::BOLD),
                    ));
                    last_group = entry.group.as_str();
                }
                let note = entry.note.as_deref().unwrap_or("");
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
        ModalState::PlanExecution {
            plan_id,
            plan_title,
            step_index,
            step_count,
            step_previews,
            step_effects,
            step_tools,
            step_risks,
            phase,
            result_summary,
        } => {
            let mut lines = Vec::new();
            match phase {
                PlanExecPhase::Approving => {
                    let i = *step_index;
                    lines.push(Line::from(Span::styled(
                        format!(
                            " Plan: {} ({}) — Step {}/{}",
                            plan_title,
                            plan_id,
                            i + 1,
                            step_count
                        ),
                        Style::default().fg(Color::Cyan),
                    )));
                    lines.push(Line::from(""));
                    if let Some(tool) = step_tools.get(i) {
                        lines.push(Line::from(format!(" Tool:     {tool}")));
                    }
                    if let Some(cmd) = step_previews.get(i) {
                        lines.push(Line::from(format!(" Command:  {cmd}")));
                    }
                    if let Some(effect) = step_effects.get(i) {
                        lines.push(Line::from(format!(" Effect:   {effect}")));
                    }
                    if let Some(risk) = step_risks.get(i) {
                        lines.push(Line::from(format!(" Risk:     {risk}")));
                    }
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        " [Y] Execute step   [N] Skip step   [!] Approve all remaining   [Esc] Cancel",
                        Style::default().fg(Color::Yellow),
                    )));
                }
                PlanExecPhase::Running => {
                    lines.push(Line::from(Span::styled(
                        " Executing plan... ",
                        Style::default().fg(Color::Green),
                    )));
                    lines.push(Line::from(format!(
                        " {}/{} steps complete",
                        step_index, step_count
                    )));
                }
                PlanExecPhase::Done => {
                    lines.push(Line::from(Span::styled(
                        " Execution complete ",
                        Style::default().fg(Color::Green),
                    )));
                    if !result_summary.is_empty() {
                        lines.push(Line::from(""));
                        lines.push(Line::from(Span::raw(result_summary.as_str())));
                    }
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        " Press any key to close ",
                        Style::default().fg(Color::Gray),
                    )));
                }
                PlanExecPhase::Loading => {
                    lines.push(Line::from(Span::styled(
                        " Loading plan... ",
                        Style::default().fg(Color::Gray),
                    )));
                }
            }
            Paragraph::new(lines)
                .block(modal_block(" Plan Execution "))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        }
    }
}

fn render_palette(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let commands = app.filtered_palette_items();
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
    let (icon, border_color, bg) = match toast.variant {
        ToastVariant::Success => ("✓", SUCCESS_FG, TOOL_BG),
        ToastVariant::Error => ("✗", ERROR_FG, ERROR_BG),
        ToastVariant::Info => ("i", HEADER_BORDER, TOOL_BG),
    };
    Paragraph::new(Line::from(vec![
        Span::styled(format!("{icon} "), Style::default().fg(border_color).bg(bg)),
        Span::styled(toast.text.clone(), Style::default().fg(APP_FG).bg(bg)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(bg)),
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

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
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

fn catalog_entry(
    group: &str,
    label: &str,
    provider: ProviderChoice,
    model: &str,
    note: Option<&str>,
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        group: group.to_owned(),
        label: label.to_owned(),
        provider,
        model: model.to_owned(),
        note: note.map(str::to_owned),
    }
}

fn static_model_catalog_for(choice: ProviderChoice) -> Vec<ModelCatalogEntry> {
    match choice {
        ProviderChoice::Groq => vec![
            catalog_entry(
                "Groq",
                "Llama 3.3 70B Versatile",
                choice,
                "llama-3.3-70b-versatile",
                Some("default"),
            ),
            catalog_entry(
                "Groq",
                "Llama 3.1 8B Instant",
                choice,
                "llama-3.1-8b-instant",
                Some("fast"),
            ),
            catalog_entry("Groq", "GPT OSS 120B", choice, "openai/gpt-oss-120b", None),
            catalog_entry("Groq", "GPT OSS 20B", choice, "openai/gpt-oss-20b", None),
        ],
        ProviderChoice::Anthropic => vec![
            catalog_entry(
                "Anthropic",
                "Claude Opus 4.1",
                choice,
                "claude-opus-4-1-20250805",
                Some("default"),
            ),
            catalog_entry(
                "Anthropic",
                "Claude 3.5 Haiku",
                choice,
                "claude-3-5-haiku-20241022",
                Some("fast"),
            ),
        ],
        ProviderChoice::Ollama => vec![catalog_entry(
            "Ollama",
            "Qwen3 4B",
            choice,
            "qwen3:4b",
            Some("local"),
        )],
        ProviderChoice::Gemini => vec![
            catalog_entry(
                "Google",
                "Gemini 2.5 Flash",
                choice,
                "gemini-2.5-flash",
                Some("default"),
            ),
            catalog_entry(
                "Google",
                "Gemini 2.5 Flash Lite",
                choice,
                "gemini-2.5-flash-lite",
                Some("fast"),
            ),
            catalog_entry("Google", "Gemini 2.5 Pro", choice, "gemini-2.5-pro", None),
            catalog_entry(
                "Google",
                "Gemini 2.0 Flash",
                choice,
                "gemini-2.0-flash",
                None,
            ),
        ],
        ProviderChoice::Openrouter => vec![
            catalog_entry(
                "OpenRouter",
                "Kimi K2.6",
                choice,
                "moonshotai/kimi-k2.6",
                None,
            ),
            catalog_entry(
                "OpenRouter",
                "DeepSeek Chat",
                choice,
                "deepseek/deepseek-chat",
                Some("free"),
            ),
            catalog_entry(
                "OpenRouter",
                "DeepSeek Reasoner",
                choice,
                "deepseek/deepseek-r1",
                Some("free"),
            ),
            catalog_entry(
                "OpenRouter",
                "Qwen 3 Coder",
                choice,
                "qwen/qwen3-coder",
                None,
            ),
        ],
        ProviderChoice::NvidiaNim => vec![
            catalog_entry("NVIDIA", "Kimi K2.6", choice, "moonshotai/kimi-k2.6", None),
            catalog_entry("NVIDIA", "GLM 5.1", choice, "z-ai/glm-5.1", None),
            catalog_entry(
                "NVIDIA",
                "DeepSeek V4 Pro",
                choice,
                "deepseek-ai/deepseek-v4-pro",
                None,
            ),
            catalog_entry(
                "NVIDIA",
                "Nemotron 3 Super 120B",
                choice,
                "nvidia/nemotron-3-super-120b",
                None,
            ),
        ],
        ProviderChoice::OpenaiCompat => vec![catalog_entry(
            "OpenAI-Compatible",
            "GPT-4o Mini",
            choice,
            "gpt-4o-mini",
            Some("default"),
        )],
        ProviderChoice::Auto => Vec::new(),
    }
}

fn filtered_model_catalog(entries: &[ModelCatalogEntry], query: &str) -> Vec<ModelCatalogEntry> {
    let query = query.trim().to_ascii_lowercase();
    entries
        .iter()
        .filter(|entry| {
            query.is_empty()
                || entry.label.to_ascii_lowercase().contains(&query)
                || entry.model.to_ascii_lowercase().contains(&query)
                || entry.group.to_ascii_lowercase().contains(&query)
                || provider_choice_name(entry.provider)
                    .to_ascii_lowercase()
                    .contains(&query)
        })
        .cloned()
        .collect()
}

fn live_catalog_group(choice: ProviderChoice) -> &'static str {
    match choice {
        ProviderChoice::Groq => "Groq",
        ProviderChoice::Anthropic => "Anthropic",
        ProviderChoice::Ollama => "Ollama",
        ProviderChoice::Gemini => "Google",
        ProviderChoice::Openrouter => "OpenRouter",
        ProviderChoice::NvidiaNim => "NVIDIA",
        ProviderChoice::OpenaiCompat => "OpenAI-Compatible",
        ProviderChoice::Auto => "Auto",
    }
}

fn base_url_for_provider(choice: ProviderChoice, settings: &ProviderSettings) -> String {
    match choice {
        ProviderChoice::Groq => "https://api.groq.com/openai/v1".to_owned(),
        ProviderChoice::Anthropic => "https://api.anthropic.com".to_owned(),
        ProviderChoice::Ollama => settings
            .base_url
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_owned()),
        ProviderChoice::Gemini => settings
            .base_url
            .clone()
            .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_owned()),
        ProviderChoice::Openrouter => "https://openrouter.ai/api/v1".to_owned(),
        ProviderChoice::NvidiaNim => "https://integrate.api.nvidia.com/v1".to_owned(),
        ProviderChoice::OpenaiCompat => settings
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_owned()),
        ProviderChoice::Auto => settings
            .base_url
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_owned()),
    }
}

fn model_note_for(choice: ProviderChoice, model: &str, live: bool) -> Option<String> {
    if model == default_model_name(choice) {
        Some("default".to_owned())
    } else if choice == ProviderChoice::Ollama && model.ends_with(":cloud") {
        Some("cloud".to_owned())
    } else if choice == ProviderChoice::Ollama {
        Some("local".to_owned())
    } else if live {
        Some("live".to_owned())
    } else {
        None
    }
}

async fn fetch_model_catalog_for_provider(
    choice: ProviderChoice,
    settings: &ProviderSettings,
    resolved_key: Option<String>,
) -> Result<Vec<ModelCatalogEntry>> {
    let base_url = base_url_for_provider(choice, settings);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .context("building model catalog client")?;
    let models = match choice {
        ProviderChoice::Ollama => fetch_ollama_catalog(&client, &base_url).await?,
        ProviderChoice::Gemini => {
            let key = resolved_key.ok_or_else(|| anyhow!("missing Gemini API key"))?;
            fetch_gemini_catalog(&client, &base_url, &key).await?
        }
        ProviderChoice::Anthropic => {
            let key = resolved_key.ok_or_else(|| anyhow!("missing Anthropic API key"))?;
            fetch_anthropic_catalog(&client, &base_url, &key).await?
        }
        ProviderChoice::Groq
        | ProviderChoice::Openrouter
        | ProviderChoice::NvidiaNim
        | ProviderChoice::OpenaiCompat => {
            fetch_openai_style_catalog(&client, choice, &base_url, resolved_key.as_deref()).await?
        }
        ProviderChoice::Auto => Vec::new(),
    };

    if models.is_empty() {
        return Ok(static_model_catalog_for(choice));
    }

    Ok(models)
}

#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    #[serde(default)]
    data: Vec<OpenAiModelRecord>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelRecord {
    id: String,
}

async fn fetch_openai_style_catalog(
    client: &reqwest::Client,
    choice: ProviderChoice,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelCatalogEntry>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut request = client.get(url);
    if let Some(key) = api_key
        && !key.trim().is_empty()
    {
        request = request.bearer_auth(key);
    }
    if choice == ProviderChoice::Openrouter {
        request = request
            .header("HTTP-Referer", "https://github.com/Jatin-Mali/helm")
            .header("X-Title", "HELM");
    }
    let response = request.send().await?.error_for_status()?;
    let parsed: OpenAiModelsResponse = response.json().await?;
    let group = live_catalog_group(choice);
    let mut entries = parsed
        .data
        .into_iter()
        .map(|record| {
            let note = model_note_for(choice, &record.id, true);
            ModelCatalogEntry {
                group: group.to_owned(),
                label: record.id.clone(),
                provider: choice,
                model: record.id,
                note,
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(entries)
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaTagRecord>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagRecord {
    name: String,
}

async fn fetch_ollama_catalog(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<Vec<ModelCatalogEntry>> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = client.get(url).send().await?.error_for_status()?;
    let parsed: OllamaTagsResponse = response.json().await?;
    let mut entries = parsed
        .models
        .into_iter()
        .map(|record| ModelCatalogEntry {
            group: "Ollama".to_owned(),
            label: record.name.clone(),
            provider: ProviderChoice::Ollama,
            model: record.name.clone(),
            note: model_note_for(ProviderChoice::Ollama, &record.name, true),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(entries)
}

#[derive(Debug, Deserialize)]
struct GeminiModelsResponse {
    #[serde(default)]
    models: Vec<GeminiModelRecord>,
}

#[derive(Debug, Deserialize)]
struct GeminiModelRecord {
    name: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "supportedGenerationMethods")]
    supported_generation_methods: Option<Vec<String>>,
}

async fn fetch_gemini_catalog(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelCatalogEntry>> {
    let url = format!(
        "{}/v1beta/models?key={api_key}",
        base_url.trim_end_matches('/')
    );
    let response = client.get(url).send().await?.error_for_status()?;
    let parsed: GeminiModelsResponse = response.json().await?;
    let mut entries = parsed
        .models
        .into_iter()
        .filter(|record| {
            record
                .supported_generation_methods
                .as_ref()
                .is_none_or(|methods| methods.iter().any(|method| method == "generateContent"))
        })
        .map(|record| {
            let model_id = record.name.trim_start_matches("models/").to_owned();
            ModelCatalogEntry {
                group: "Google".to_owned(),
                label: record.display_name.unwrap_or_else(|| model_id.clone()),
                provider: ProviderChoice::Gemini,
                note: model_note_for(ProviderChoice::Gemini, &model_id, true),
                model: model_id,
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(entries)
}

#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    #[serde(default)]
    data: Vec<AnthropicModelRecord>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelRecord {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
}

async fn fetch_anthropic_catalog(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelCatalogEntry>> {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let response = client
        .get(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await?
        .error_for_status()?;
    let parsed: AnthropicModelsResponse = response.json().await?;
    let mut entries = parsed
        .data
        .into_iter()
        .map(|record| ModelCatalogEntry {
            group: "Anthropic".to_owned(),
            label: record.display_name.unwrap_or_else(|| record.id.clone()),
            provider: ProviderChoice::Anthropic,
            note: model_note_for(ProviderChoice::Anthropic, &record.id, true),
            model: record.id,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(entries)
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

fn format_evidence_report(tool_name: &str, ev: &StructuredEvidence) -> String {
    let mut out = String::new();
    out.push_str(&format!("[evidence] {tool_name}\n"));
    out.push_str(&format!(
        "  inspected sources: {}\n",
        ev.inspected_sources.join(", ")
    ));
    if !ev.findings.is_empty() {
        out.push_str("  findings:\n");
        for f in &ev.findings {
            out.push_str(&format!(
                "    - {}: {} (source: {})\n",
                f.label,
                f.value.lines().next().unwrap_or(""),
                f.source
            ));
        }
    }
    if !ev.assumptions.is_empty() {
        out.push_str(&format!("  assumptions: {}\n", ev.assumptions.join("; ")));
    }
    out.push_str(&format!("  uncertainty: {:?}\n", ev.uncertainty));
    if !ev.proposed_actions.is_empty() {
        out.push_str("  proposed actions:\n");
        for a in &ev.proposed_actions {
            out.push_str(&format!(
                "    - {} (tool: {}, input: {})\n",
                a.description, a.tool, a.tool_input
            ));
        }
    }
    if !ev.blast_radius.paths.is_empty()
        || !ev.blast_radius.services.is_empty()
        || !ev.blast_radius.hosts.is_empty()
    {
        out.push_str("  blast radius:\n");
        if !ev.blast_radius.paths.is_empty() {
            out.push_str(&format!(
                "    paths: {}\n",
                ev.blast_radius.paths.join(", ")
            ));
        }
        if !ev.blast_radius.services.is_empty() {
            out.push_str(&format!(
                "    services: {}\n",
                ev.blast_radius.services.join(", ")
            ));
        }
        if !ev.blast_radius.hosts.is_empty() {
            out.push_str(&format!(
                "    hosts: {}\n",
                ev.blast_radius.hosts.join(", ")
            ));
        }
    }
    if ev.rollback.available {
        out.push_str(&format!("  rollback: {}\n", ev.rollback.description));
    } else {
        out.push_str("  rollback: not available\n");
    }
    if !ev.exact_tool_calls.is_empty() {
        out.push_str("  exact tool calls:\n");
        for tc in &ev.exact_tool_calls {
            out.push_str(&format!(
                "    - {}: {}\n      input: {}\n",
                tc.tool, tc.summary, tc.tool_input
            ));
        }
    }
    out
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
            diagnose_mode: false,
            sandbox: None,
            remote_target: None,
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

    fn test_catalog() -> Vec<ModelCatalogEntry> {
        let mut entries = Vec::new();
        for choice in [
            ProviderChoice::Groq,
            ProviderChoice::Gemini,
            ProviderChoice::NvidiaNim,
        ] {
            entries.extend(static_model_catalog_for(choice));
        }
        entries
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
        let mut app = app();
        app.command_palette.query = "doctor".to_owned();
        let filtered = app.filtered_palette_items();
        assert_eq!(filtered, vec![PaletteItem::BuiltIn(CommandAction::Doctor)]);
    }

    #[test]
    fn command_palette_includes_custom_commands() {
        let mut app = app();
        app.custom_commands.push(custom_commands::CustomCommand {
            name: "triage".to_owned(),
            description: "Quick incident triage helper".to_owned(),
            body: "Investigate alerts for {{args}}".to_owned(),
        });
        app.command_palette.query = "triage".to_owned();

        let filtered = app.filtered_palette_items();

        assert_eq!(
            filtered,
            vec![PaletteItem::Custom(custom_commands::CustomCommand {
                name: "triage".to_owned(),
                description: "Quick incident triage helper".to_owned(),
                body: "Investigate alerts for {{args}}".to_owned(),
            })]
        );
    }

    #[test]
    fn slash_popup_includes_custom_commands() {
        let mut app = app();
        app.custom_commands.push(custom_commands::CustomCommand {
            name: "triage".to_owned(),
            description: "Quick incident triage helper".to_owned(),
            body: "Investigate alerts for {{args}}".to_owned(),
        });
        app.input.text = "/tri".to_owned();

        let filtered = app.slash_filtered();

        assert!(matches!(
            filtered.first(),
            Some(SlashItem::Custom(command)) if command.name == "triage"
        ));
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
            "◷ shell: shell `date && uname -a` -> /tmp/helm.txt ..."
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
        assert!(
            app.session.chat[0]
                .text
                .starts_with("✗ fs_read failed: read /etc/shadow")
        );
        assert!(
            app.session.chat[0]
                .text
                .ends_with("path denied: /etc/shadow")
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
        let catalog = test_catalog();
        let models = filtered_model_catalog(&catalog, "gemini 2.5 flash");
        assert!(models.iter().any(|entry| {
            entry.provider == ProviderChoice::Gemini && entry.model == "gemini-2.5-flash"
        }));

        let models = filtered_model_catalog(&catalog, "kimi");
        assert!(models.iter().any(|entry| {
            entry.provider == ProviderChoice::NvidiaNim && entry.model == "moonshotai/kimi-k2.6"
        }));
    }

    #[test]
    fn applying_catalog_model_switches_provider_and_model() {
        let mut app = app();
        let catalog = test_catalog();
        let entry = filtered_model_catalog(&catalog, "deepseek v4")
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
    fn provider_selector_reads_secret_store_status() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in_dir(&dir, crate::ProviderChoice::Ollama);
        app.runtime
            .secrets
            .set(
                "GROQ_API_KEY",
                helm_core::Secret::new("gsk_abcdefghijklmnopqrstuvwxyz123456".to_owned()),
            )
            .unwrap();

        assert_eq!(
            app.provider_key_status(ProviderChoice::Groq),
            ProviderKeyStatus::Stored
        );
    }

    #[test]
    fn provider_selector_reads_session_override_status() {
        let _guard = env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: guarded by env_lock for this test.
        unsafe {
            std::env::remove_var("GROQ_API_KEY");
        }
        let mut app = app_in_dir(&dir, crate::ProviderChoice::Groq);
        app.active_settings.api_key = Some("gsk_session_key_abcdefghijklmnopqrstuvwxyz".to_owned());

        assert_eq!(
            app.provider_key_status(ProviderChoice::Groq),
            ProviderKeyStatus::Session
        );
    }

    #[test]
    fn remote_command_switches_active_target() {
        let _guard = env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        std::fs::create_dir_all(home.join(".config").join("helm")).unwrap();
        std::fs::write(
            home.join(".config").join("helm").join("remotes.toml"),
            "[[remotes]]\nname = \"prod-1\"\nhost = \"prod.example.com\"\nport = 22\n",
        )
        .unwrap();
        // SAFETY: guarded by env_lock for this test.
        unsafe {
            std::env::set_var("HOME", &home);
        }
        let mut app = app_in_dir(&dir, crate::ProviderChoice::Ollama);

        app.apply_remote_target("prod-1");

        assert_eq!(app.active_remote.as_deref(), Some("prod-1"));

        // SAFETY: paired with the guarded set_var above.
        unsafe {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn remote_command_can_clear_target() {
        let mut app = app();
        app.active_remote = Some("prod-1".to_owned());

        app.apply_remote_target("off");

        assert!(app.active_remote.is_none());
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

    #[test]
    fn live_ollama_catalog_parses_mocked_tags() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let maybe_server = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.block_on(async { mockito::Server::new_async().await })
        }));
        let Ok(mut server) = maybe_server else {
            eprintln!("skipping mockito-backed catalog test: mock server unavailable");
            return;
        };

        runtime.block_on(async {
            let mock = server
                .mock("GET", "/api/tags")
                .with_status(200)
                .with_body(
                    serde_json::json!({
                        "models": [
                            {"name": "qwen3:4b"},
                            {"name": "llama3.3:70b"}
                        ]
                    })
                    .to_string(),
                )
                .create_async()
                .await;
            let client = reqwest::Client::builder().build().unwrap();

            let entries = fetch_ollama_catalog(&client, &server.url()).await.unwrap();

            assert!(entries.iter().any(|entry| entry.model == "qwen3:4b"));
            assert!(entries.iter().any(|entry| entry.model == "llama3.3:70b"));
            mock.assert_async().await;
        });
    }

    // ── Dashboard tests ────────────────────────────────────────────────

    #[test]
    fn dash_panel_all_returns_nine_panels() {
        assert_eq!(DashPanel::all().len(), 9);
    }

    #[test]
    fn dash_panel_labels_are_non_empty() {
        for panel in DashPanel::all() {
            assert!(!panel.label().is_empty(), "panel label should not be empty");
        }
    }

    #[test]
    fn dash_panel_cycle_forward_and_back() {
        let panels = DashPanel::all();
        let mut idx = 0usize;
        // forward
        idx = (idx + 1) % panels.len();
        assert_eq!(panels[idx], DashPanel::Findings);
        // backward
        idx = (idx + panels.len() - 1) % panels.len();
        assert_eq!(panels[idx], DashPanel::Health);
    }

    #[test]
    fn dashboard_state_initializes_clean() {
        let state = DashboardState::new();
        assert_eq!(state.selected, DashPanel::Health);
        assert!(state.error.is_none());
        assert_eq!(state.data.hostname, "");
    }

    #[test]
    fn dashboard_data_defaults_are_zero() {
        let d = DashboardData::default();
        assert_eq!(d.load_1m, 0.0);
        assert_eq!(d.total_services, 0);
        assert_eq!(d.finding_count, 0);
    }

    #[test]
    fn render_dash_panel_health_shows_percent() {
        let mut d = DashboardData::default();
        d.hostname = "testbox".into();
        d.memory_used_pct = 42.5;
        d.load_1m = 1.5;
        let text = render_dash_panel(DashPanel::Health, &d);
        assert!(text.contains("testbox"), "health panel should show hostname");
        assert!(text.contains("42"), "health panel should show memory %");
        assert!(text.contains("1.5"), "health panel should show load");
    }

    #[test]
    fn render_dash_panel_findings_shows_count() {
        let mut d = DashboardData::default();
        d.finding_count = 3;
        d.finding_warnings = 1;
        let text = render_dash_panel(DashPanel::Findings, &d);
        assert!(text.contains("3"), "findings panel should show total");
        assert!(text.contains("1"), "findings panel should show warning count");
    }

    #[test]
    fn render_dash_panel_services_shows_failed() {
        let mut d = DashboardData::default();
        d.total_services = 10;
        d.failed_services = 2;
        let text = render_dash_panel(DashPanel::Services, &d);
        assert!(text.contains("FAILED: 2"), "services panel should show failed count");
    }

    #[test]
    fn render_dash_panel_disk_shows_entries() {
        let mut d = DashboardData::default();
        d.disk_entries.push("/ 45%".into());
        d.disk_entries.push("/home 12%".into());
        let text = render_dash_panel(DashPanel::Disk, &d);
        assert!(text.contains("45%"), "disk panel should show usage");
        assert!(text.contains("/home"), "disk panel should show mount");
    }

    #[test]
    fn render_dash_panel_containers_shows_counts() {
        let mut d = DashboardData::default();
        d.total_containers = 5;
        d.running_containers = 3;
        let text = render_dash_panel(DashPanel::Containers, &d);
        assert!(text.contains("Total: 5"), "containers panel should show total");
        assert!(text.contains("Running: 3"), "containers panel should show running");
    }
}
