//! Command-line entry point for HELM.

mod agent_remote;
mod attach_tui;
mod bootstrap;
mod builtin_skills;
mod custom_commands;
mod hooks;
mod keybindings;
mod ndjson_sink;
mod paths;
mod remote;
mod sandbox;
mod secrets;
mod serve;
mod snapshot_sink;
mod telemetry;
mod tui;

use std::{
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Instant,
};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use helm_agent::{AgentEvent, AgentEventSink, Budget, CancellationToken, ReactAgent, RunResult};
use helm_core::{Capability, ContentBlock, GrantScope, HelmError, ProviderError, Secret};
use helm_memory::{
    AuditEventRecord, CapabilityGrantRecord, EpisodeRecord, MemoryStore, SessionStore, StepRecord,
    UserProfileStore,
};
use helm_monitor::{MonitorProfile, SystemSnapshot, collect_snapshot};
use helm_providers::{
    AnthropicProvider, ChatRequest, ChatResponse, GeminiProvider, OllamaProvider,
    OpenAiCompatProvider, Provider, StopReason, ToolSchema, quirks_for,
};
use helm_tools::{SkillTool, Tool, ToolContext, ToolRegistry};
use secrets::SecretsStore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt, fmt::MakeWriter, prelude::*};

#[derive(Debug, Parser)]
#[command(name = "helm", version, about = "Self-hosted Linux operations agent")]
struct Cli {
    #[arg(long, value_name = "PATH", global = true)]
    db_path: Option<PathBuf>,
    #[arg(long, value_name = "N", global = true)]
    max_iterations: Option<u32>,
    #[arg(
        long,
        value_name = "ID",
        global = true,
        help = "Model id. For Ollama default qwen3:4b, install with `ollama pull qwen3:4b`."
    )]
    model: Option<String>,
    #[arg(long, value_enum, global = true)]
    provider: Option<ProviderChoice>,
    #[arg(
        long,
        value_name = "KEY",
        global = true,
        help = "API key override for this process only"
    )]
    api_key: Option<String>,
    #[arg(
        long = "base-url",
        alias = "ollama-url",
        value_name = "URL",
        global = true
    )]
    base_url: Option<String>,
    #[arg(long, global = true)]
    verbose: bool,
    /// Enable structured JSON tracing output to stderr
    #[arg(long, global = true)]
    trace: bool,
    /// Auto-approve all tool permission requests (development only)
    #[arg(long = "yes", global = true, hide = true)]
    yes: bool,
    /// Plan mode: read-only analysis, no writes or executions
    #[arg(long = "read-only", alias = "plan", global = true)]
    read_only: bool,
    /// Dry-run: print intended commands without executing anything
    #[arg(long, global = true)]
    dry_run: bool,
    /// Show system evidence report before executing permission-sensitive tools
    #[arg(long, global = true)]
    evidence: bool,
    /// Confine local tool execution with Bubblewrap. Default root is the current working directory.
    #[arg(long, global = true)]
    sandbox: bool,
    /// Use this directory as the Bubblewrap sandbox root instead of the current directory.
    #[arg(long, value_name = "PATH", global = true)]
    sandbox_dir: Option<PathBuf>,
    /// Execute shell-style tool calls on the named remote host (registered via `helm remote add`).
    #[arg(long, value_name = "NAME", global = true)]
    remote: Option<String>,
    /// Resume a specific session id for the next `run`.
    #[arg(long, value_name = "ID", global = true)]
    resume: Option<String>,
    /// Resume the latest session when running a new task.
    #[arg(long = "continue", global = true)]
    continue_last: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run(RunArgs),
    Replay(ReplayArgs),
    Models,
    Doctor(DoctorArgs),
    Episodes(EpisodesArgs),
    Permissions(PermissionsArgs),
    Audit(AuditArgs),
    Skills(SkillsArgs),
    Secrets(SecretsArgs),
    Init(InitArgs),
    /// Manage configuration (get/set/edit/validate/path)
    Config(ConfigArgs),
    /// Generate shell completion scripts
    Completion(CompletionArgs),
    /// Manage MCP server configurations
    Mcp(McpArgs),
    Tui(TuiArgs),
    /// Manage sessions (list/delete/export/resume)
    Sessions(SessionsArgs),
    /// Manage remote target hosts (SSH)
    Remote(RemoteArgs),
    /// Run a bearer-auth HTTP server that accepts agent tasks
    Serve(ServeArgs),
    /// Export episode to file
    Export(ExportArgs),
    /// Manage file snapshots and undo/redo
    Undo(UndoArgs),
    /// Re-apply the last undone file snapshot for a session
    Redo(UndoArgs),
    /// Show cost and usage statistics
    Stats(StatsArgs),
    /// Manage knowledge graph and memory
    Memory(MemoryArgs),
    /// Manage user profile and preferences
    Profile(ProfileArgs),
    /// Bootstrap HELM onto a reachable Linux host over SSH
    Bootstrap(BootstrapArgs),
    /// Read-only diagnostic mode — cannot write or execute dangerous tools
    Diagnose(DiagnoseArgs),
    /// Show trust report: grants, audit, sandbox, secrets, integrity
    TrustReport(TrustReportArgs),
    /// Collect a typed read-only system snapshot
    Snapshot(SnapshotArgs),
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(value_name = "TASK")]
    task: String,
    /// Comma-separated fallback chain (e.g. "anthropic,openai,groq")
    #[arg(long, value_name = "PROVIDERS")]
    fallback: Option<String>,
    /// Maximum cost in USD; stop if exceeded
    #[arg(long, value_name = "USD")]
    budget: Option<f64>,
    /// Shell command to run before agent starts
    #[arg(long, value_name = "CMD")]
    pre_run: Option<String>,
    /// Shell command to run after agent finishes
    #[arg(long, value_name = "CMD")]
    post_run: Option<String>,
    /// Shell command run before each tool call (env: HELM_TOOL_NAME, HELM_TOOL_INPUT)
    #[arg(long, value_name = "CMD")]
    on_tool_call: Option<String>,
    /// Emit each agent event as a newline-delimited JSON line on stdout (used by `--remote`).
    #[arg(long)]
    emit_events: bool,
    /// When combined with --remote, force agent-on-remote execution via SSH (NDJSON stream).
    #[arg(long)]
    agent_on_remote: bool,
}

#[derive(Debug, Args)]
struct ReplayArgs {
    #[arg(value_name = "EPISODE_ID")]
    episode_id: String,
}

#[derive(Debug, Args)]
struct EpisodesArgs {
    #[arg(long, value_name = "N", default_value_t = 10)]
    limit: u32,
}

#[derive(Debug, Args)]
struct DoctorArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PermissionsArgs {
    #[command(subcommand)]
    command: PermissionsCommand,
}

#[derive(Debug, Subcommand)]
enum PermissionsCommand {
    List,
    Grant(PermissionGrantArgs),
    Revoke(PermissionRevokeArgs),
}

#[derive(Debug, Args)]
struct PermissionGrantArgs {
    #[arg(value_name = "CAPABILITY")]
    capability: String,
    #[arg(long, value_name = "SCOPE", default_value = "once")]
    scope: String,
}

#[derive(Debug, Args)]
struct PermissionRevokeArgs {
    #[arg(value_name = "CAPABILITY")]
    capability: String,
}

#[derive(Debug, Args)]
struct TuiArgs {
    /// Attach to a running `helm serve` instance instead of running locally.
    /// Format: HOST:PORT (token provided via --token or HELM_REMOTE_TOKEN env).
    #[arg(long, value_name = "HOST:PORT")]
    attach: Option<String>,
    /// Bearer token used when --attach is supplied.
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
}

#[derive(Debug, Args)]
struct RemoteArgs {
    #[command(subcommand)]
    command: RemoteCommand,
}

#[derive(Debug, Subcommand)]
enum RemoteCommand {
    /// Register a new SSH-reachable remote target.
    Add(RemoteAddArgs),
    /// List registered remotes.
    List,
    /// Test that the remote is reachable (`ssh remote true`).
    Test {
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Remove a registered remote.
    Remove {
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Debug, Args)]
struct RemoteAddArgs {
    #[arg(value_name = "NAME")]
    name: String,
    #[arg(long, value_name = "HOST")]
    host: String,
    #[arg(long, value_name = "USER")]
    user: Option<String>,
    #[arg(long, default_value_t = 22)]
    port: u16,
    /// Optional inline SSH options (e.g. "-i ~/.ssh/id_ed25519").
    #[arg(long)]
    ssh_opts: Option<String>,
}

#[derive(Debug, Args)]
struct BootstrapArgs {
    /// Hostname or `user@host` to bootstrap.
    #[arg(value_name = "HOST")]
    host: String,
    /// SSH user override (use this if HOST is bare).
    #[arg(long, value_name = "USER")]
    user: Option<String>,
    /// SSH port; default 22.
    #[arg(long, default_value_t = 22)]
    port: u16,
    /// Download the helm binary on the remote from this URL instead of uploading the local one.
    #[arg(long, value_name = "URL")]
    release_url: Option<String>,
    /// Register the bootstrapped target in the HELM XDG config remotes registry under this name.
    #[arg(long, value_name = "NAME")]
    register_as: Option<String>,
    /// Path to a local helm binary to upload. Defaults to the currently running binary.
    #[arg(long, value_name = "PATH")]
    local_binary: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Address to bind on (default 127.0.0.1:8765).
    #[arg(long, value_name = "ADDR", default_value = "127.0.0.1:8765")]
    bind: String,
    /// Required bearer token. If omitted a random token is generated and printed once.
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
}

#[derive(Debug, Args)]
struct SessionsArgs {
    #[command(subcommand)]
    command: SessionsCommand,
}

#[derive(Debug, Subcommand)]
enum SessionsCommand {
    List {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Delete {
        id: String,
    },
    Export {
        id: String,
        #[arg(long, default_value = "json")]
        format: String,
    },
    Resume {
        id: String,
        /// Continue the session by handing this task to the agent with prior conversation context.
        #[arg(long, value_name = "TASK")]
        task: Option<String>,
        /// Print transcript and exit without running a follow-up task.
        #[arg(long)]
        show: bool,
    },
}

#[derive(Debug, Args)]
struct ExportArgs {
    #[arg(value_name = "EPISODE_ID")]
    episode_id: String,
    #[arg(long, value_name = "FORMAT", default_value = "json")]
    format: String,
    #[arg(long, value_name = "OUTPUT")]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct UndoArgs {
    #[arg(value_name = "N", default_value_t = 1)]
    n: u32,
    #[arg(long)]
    session_id: Option<String>,
    /// Actually write the snapshot content back to disk (otherwise dry-run prints diff).
    #[arg(long)]
    apply: bool,
    /// Write to this path instead of the recorded file_path.
    #[arg(long, value_name = "PATH")]
    to: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct StatsArgs {
    /// Time range: "7d" (default), "30d", or "all"
    #[arg(long, default_value = "7d", value_name = "RANGE")]
    time_range: String,
}

#[derive(Debug, Args)]
struct MemoryArgs {
    #[command(subcommand)]
    command: MemoryCommand,
}

#[derive(Debug, Subcommand)]
enum MemoryCommand {
    /// List entities in the knowledge graph
    Graph {
        #[arg(long)]
        entity_type: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Export memory to JSON file
    Export {
        #[arg(short, long)]
        output: String,
    },
    /// Import memory from JSON file
    Import {
        #[arg(short, long)]
        input: String,
    },
    /// Prune stale relations older than N days or below confidence threshold
    Gc {
        #[arg(long, default_value = "90")]
        age_days: u32,
        #[arg(long, default_value = "0.1")]
        min_confidence: f32,
    },
}

#[derive(Debug, Args)]
struct ProfileArgs {
    #[command(subcommand)]
    command: ProfileCommand,
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
    /// Show top preferred tools and preferences
    Show,
    /// Set a preference key=value
    Set { key: String, value: String },
    /// Get a preference value
    Get { key: String },
    /// Show model routing success rates
    Routes,
}

#[derive(Debug, Args)]
struct AuditArgs {
    #[command(subcommand)]
    command: AuditCommand,
}

#[derive(Debug, Subcommand)]
enum AuditCommand {
    Verify(AuditVerifyArgs),
    Show(AuditShowArgs),
}

#[derive(Debug, Args)]
struct AuditVerifyArgs {
    #[arg(long, value_name = "TARGET")]
    target: Option<String>,
}

#[derive(Debug, Args)]
struct AuditShowArgs {
    #[arg(long, value_name = "EPISODE_ID")]
    episode: Option<String>,
    #[arg(long, value_name = "TARGET")]
    target: Option<String>,
}

#[derive(Debug, Args)]
struct SkillsArgs {
    #[command(subcommand)]
    command: SkillsCommand,
}

#[derive(Debug, Subcommand)]
enum SkillsCommand {
    List,
    Show(SkillShowArgs),
    Approve(SkillApproveArgs),
    Disable(SkillDisableArgs),
    Test(SkillTestArgs),
    Run(SkillRunArgs),
}

#[derive(Debug, Args)]
struct SkillShowArgs {
    #[arg(value_name = "ID")]
    id: String,
}

#[derive(Debug, Args)]
struct SkillApproveArgs {
    #[arg(value_name = "ID")]
    id: String,
}

#[derive(Debug, Args)]
struct SkillDisableArgs {
    #[arg(value_name = "ID")]
    id: String,
}

#[derive(Debug, Args)]
struct SkillTestArgs {
    #[arg(value_name = "ID")]
    id: String,
}

#[derive(Debug, Args)]
struct SkillRunArgs {
    #[arg(value_name = "ID")]
    id: String,
    /// JSON object of input values for `{{key}}` substitution (e.g. '{"branch":"main"}').
    #[arg(long, value_name = "JSON")]
    input: Option<String>,
    /// Print resolved commands without executing them.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct McpArgs {
    #[command(subcommand)]
    command: McpCommand,
}

#[derive(Debug, Subcommand)]
enum McpCommand {
    /// List configured MCP servers
    List,
    /// Add a new MCP server
    Add(McpAddArgs),
    /// Remove a configured MCP server
    Remove(McpRemoveArgs),
    /// Test a configured MCP server by listing its tools
    Test(McpTestArgs),
    /// Run a tool on a configured MCP server
    Run(McpRunArgs),
}

#[derive(Debug, Args)]
struct McpAddArgs {
    #[arg(value_name = "NAME")]
    name: String,
    #[arg(value_name = "COMMAND")]
    command: String,
    #[arg(value_name = "ARGS", trailing_var_arg = true)]
    args: Vec<String>,
}

#[derive(Debug, Args)]
struct McpRemoveArgs {
    #[arg(value_name = "NAME")]
    name: String,
}

#[derive(Debug, Args)]
struct McpTestArgs {
    #[arg(value_name = "NAME")]
    name: String,
}

#[derive(Debug, Args)]
struct McpRunArgs {
    #[arg(value_name = "SERVER")]
    server: String,
    #[arg(value_name = "TOOL")]
    tool: String,
    #[arg(long, value_name = "JSON", default_value = "{}")]
    arguments: String,
}

#[derive(Debug, Args)]
struct SecretsArgs {
    #[command(subcommand)]
    command: SecretsCommand,
}

#[derive(Debug, Subcommand)]
enum SecretsCommand {
    List,
    Set(SecretsSetArgs),
    Get(SecretsGetArgs),
    Delete(SecretsDeleteArgs),
    Path,
    ImportEnv,
}

#[derive(Debug, Args)]
struct SecretsSetArgs {
    #[arg(value_name = "NAME")]
    name: String,
    #[arg(
        long,
        value_name = "VALUE",
        help = "Set value directly (non-interactive)"
    )]
    value: Option<String>,
    #[arg(long, help = "Read value from stdin")]
    from_stdin: bool,
}

#[derive(Debug, Args)]
struct SecretsGetArgs {
    #[arg(value_name = "NAME")]
    name: String,
}

#[derive(Debug, Args)]
struct SecretsDeleteArgs {
    #[arg(value_name = "NAME")]
    name: String,
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long, help = "Overwrite an existing HELM config file")]
    force: bool,
    #[arg(long, help = "Skip API key validation")]
    no_validate: bool,
}

#[derive(Debug, Args)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Print a config value
    Get(ConfigGetArgs),
    /// Set a config value
    Set(ConfigSetArgs),
    /// Open config in $EDITOR
    Edit,
    /// Validate the config file
    Validate,
    /// Print config file path
    Path,
}

#[derive(Debug, Args)]
struct ConfigGetArgs {
    /// Dotted key path (e.g. provider.model)
    #[arg(value_name = "KEY")]
    key: String,
}

#[derive(Debug, Args)]
struct ConfigSetArgs {
    /// Dotted key path (e.g. provider.model)
    #[arg(value_name = "KEY")]
    key: String,
    /// New value
    #[arg(value_name = "VALUE")]
    value: String,
}

#[derive(Debug, Args)]
struct CompletionArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    shell: Shell,
}

#[derive(Debug, Args)]
struct DiagnoseArgs {
    #[arg(value_name = "QUESTION")]
    question: String,
    /// Output result as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct TrustReportArgs {
    /// Output as JSON
    #[arg(long)]
    json: bool,
    /// Verify audit chain against a specific remote target
    #[arg(long, value_name = "NAME")]
    target: Option<String>,
}

#[derive(Debug, Args)]
struct SnapshotArgs {
    /// Output as JSON
    #[arg(long)]
    json: bool,
    /// Collection depth (quick, standard, deep)
    #[arg(long, default_value = "standard")]
    profile: String,
    /// Output diff against previous snapshot
    #[arg(long)]
    diff: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Hash, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum ProviderChoice {
    Auto,
    Groq,
    Anthropic,
    Ollama,
    Gemini,
    Openrouter,
    NvidiaNim,
    #[value(alias = "openai-compatible")]
    #[serde(alias = "openai-compatible")]
    OpenaiCompat,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    provider: Option<FileProviderConfig>,
    security: Option<FileSecurityConfig>,
    telemetry: Option<FileTelemetryConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct FileProviderConfig {
    kind: Option<ProviderChoice>,
    base_url: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileSecurityConfig {
    tui_paste_key_modal: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct FileTelemetryConfig {
    enabled: Option<bool>,
    endpoint: Option<String>,
    service_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderSettings {
    choice: ProviderChoice,
    base_url: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    api_key: Option<String>,
    source: ProviderSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderSource {
    Cli,
    HelmProviderEnv,
    ConfigFile,
    EnvVar(&'static str),
    Fallback,
}

#[derive(Debug, Error)]
enum CliConfigError {
    #[error("failed to read config {path}: {message}")]
    Read { path: PathBuf, message: String },
    #[error("malformed config {path} at line {line}: {message}")]
    Malformed {
        path: PathBuf,
        line: usize,
        message: String,
    },
}

impl ProviderSettings {
    fn with_choice(&self, choice: ProviderChoice) -> Self {
        Self {
            choice,
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            api_key_env: self.api_key_env.clone(),
            api_key: self.api_key.clone(),
            source: self.source,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let rendered = format!("{error:#}");
            if rendered.trim_start().starts_with("Usage:") {
                println!("{rendered}");
                return ExitCode::SUCCESS;
            }
            eprintln!("{rendered}");
            match classify_exit_code(error.as_ref()) {
                2 => ExitCode::from(2),
                _ => ExitCode::from(1),
            }
        }
    }
}

fn apply_sandbox(
    enabled: bool,
    sandbox_dir: Option<&PathBuf>,
) -> Result<Option<sandbox::ResolvedSandbox>> {
    let resolved = sandbox::resolve(enabled, sandbox_dir)?;
    if let Some(policy) = &resolved {
        eprintln!(
            "[sandbox] bubblewrap enabled with root {}",
            policy.display_root().display()
        );
    }
    Ok(resolved)
}

pub(crate) fn wrap_for_remote(task: &str, remote: Option<&String>) -> String {
    match remote {
        Some(name) => format!(
            "[Operating against remote target `{name}`. Use the `ssh`, `scp`, and `rsync` tools and always pass `\"remote\": \"{name}\"` in their inputs. Do NOT use the local `shell` tool for anything that should run on the remote.]\n\n{task}"
        ),
        None => task.to_owned(),
    }
}

async fn run() -> Result<()> {
    let cli = parse_cli_from(env::args_os())?;
    let sandbox = apply_sandbox(cli.sandbox, cli.sandbox_dir.as_ref())?;
    let tui_log_path = if matches!(cli.command, Command::Tui(_)) {
        Some(default_log_path()?)
    } else {
        None
    };
    let config_path = default_config_path()?;
    let config = load_config(&config_path)?;
    let telemetry_config = {
        let t = config.as_ref().and_then(|c| c.telemetry.as_ref());
        telemetry::TelemetryConfig {
            enabled: t.and_then(|t| t.enabled).unwrap_or(false),
            endpoint: t
                .and_then(|t| t.endpoint.clone())
                .unwrap_or_else(|| "http://localhost:4317".to_string()),
            service_name: t
                .and_then(|t| t.service_name.clone())
                .unwrap_or_else(|| "helm".to_string()),
        }
    };
    init_tracing(
        cli.verbose,
        cli.trace,
        tui_log_path.as_deref(),
        &telemetry_config,
    )?;
    let provider_settings = resolve_provider_settings(
        config.as_ref(),
        cli.provider,
        cli.base_url,
        cli.model,
        cli.api_key,
    )?;
    let db_path = match cli.db_path.clone() {
        Some(p) => p,
        None => default_db_path()?,
    };
    ensure_parent_dir(&db_path)?;
    let secrets_store =
        SecretsStore::open_default().map_err(|e| anyhow!("failed to open secrets store: {e}"))?;
    let memory = Arc::new(
        MemoryStore::open(&db_path)
            .await
            .with_context(|| format!("failed to open memory database at {}", db_path.display()))?,
    );

    match cli.command {
        Command::Run(args) => {
            if args.agent_on_remote {
                let remote_name = cli.remote.as_deref().ok_or_else(|| {
                    anyhow!("--agent-on-remote requires --remote <name> to be set")
                })?;
                let registry = remote::RemoteRegistry::load()?;
                let entry = registry.get(remote_name).cloned().ok_or_else(|| {
                    anyhow!(
                        "remote target `{remote_name}` not in registry — register it via `helm remote add` or `helm bootstrap --register-as {remote_name}`"
                    )
                })?;
                let sink = CliProgressSink;
                let outcome = agent_remote::run_on_remote(&entry, &args.task, &sink, &[]).await?;
                if let Some(message) = outcome.final_message.as_deref() {
                    println!("{message}");
                } else if let Some(err) = outcome.error.as_deref() {
                    eprintln!("[remote] error: {err}");
                }
                eprintln!(
                    "[remote] tokens in {} · tokens out {} · iterations {}",
                    outcome.tokens_in, outcome.tokens_out, outcome.iterations
                );
                return if outcome.ok() {
                    Ok(())
                } else {
                    Err(anyhow!(
                        outcome.error.unwrap_or_else(|| "remote run failed".into())
                    ))
                };
            }
            if config.is_none() && provider_settings.source == ProviderSource::Fallback {
                eprintln!("HELM is not configured yet.");
                eprintln!("Run `helm init` to choose a provider and set your API key.");
                return Ok(());
            }
            if cli.yes {
                eprintln!("warning: --yes mode active — all tool permissions auto-approved");
            }
            if cli.read_only {
                eprintln!("info: --read-only mode — write/exec operations will be denied");
            }
            let provider_choice = resolve_provider_choice(provider_settings.choice);

            let resume_target = resolve_resume_target(cli.resume.as_deref(), cli.continue_last);

            // Parse fallback chain from --fallback arg.
            let fallback_chain = args
                .fallback
                .as_ref()
                .map(|f| parse_fallback_chain(f))
                .unwrap_or_default();

            let has_fallback = !fallback_chain.is_empty();

            // Try primary provider first, then fallback chain.
            let providers_to_try: Vec<ProviderChoice> = if has_fallback {
                let mut chain = vec![provider_choice];
                chain.extend(fallback_chain);
                chain
            } else {
                vec![provider_choice]
            };
            let mut budget = Budget::default();
            if let Some(max_iterations) = cli.max_iterations {
                budget.max_iterations = max_iterations;
            }
            if let Some(budget_usd) = args.budget {
                budget.max_cost_usd = Some(budget_usd);
                eprintln!("[budget] ${:.2} USD limit set", budget_usd);
            }
            budget.auto_approve = cli.yes;
            budget.read_only = cli.read_only;
            budget.dry_run = cli.dry_run;
            budget.require_evidence = cli.evidence;
            if cli.dry_run {
                eprintln!("info: --dry-run mode — tools will report synthetic success only");
            }
            if cli.evidence {
                eprintln!("info: --evidence mode — system state shown before each tool call");
            }

            // Open session store for snapshotting (U5, U7).
            let sessions_dir = paths::default_snapshots_path();
            let snapshots_dir = sessions_dir;
            let session_store = Arc::new(
                SessionStore::open(&db_path, snapshots_dir)
                    .await
                    .context("opening session store")?,
            );

            // Create session before run start (U7).
            let resumed_session = match resume_target {
                Some(target) => Some(
                    resolve_session_for_resume(&session_store, target)
                        .await
                        .context("resolving session to resume")?,
                ),
                None => None,
            };
            let tool_working_dir = sandbox
                .as_ref()
                .map(|resolved| resolved.root_dir.clone())
                .or_else(|| {
                    resumed_session
                        .as_ref()
                        .and_then(|session| session.working_dir.as_deref())
                        .map(PathBuf::from)
                })
                .unwrap_or(std::env::current_dir().unwrap_or_default());
            let working_dir = Some(tool_working_dir.to_string_lossy().into_owned());
            let (session_id, session_label) = if let Some(session) = resumed_session.as_ref() {
                eprintln!("[session] resuming {} ({})", session.id, session.name);
                (session.id.clone(), session.name.clone())
            } else {
                let session_name = derive_session_name(&args.task);
                let session_id = session_store
                    .create_session(
                        &session_name,
                        &args.task,
                        "".to_string(),
                        provider_settings
                            .model
                            .clone()
                            .or_else(|| Some(default_model_name(provider_choice).to_owned())),
                        None,
                        working_dir,
                    )
                    .await
                    .context("creating session")?;
                eprintln!("[session] created session {} for run", session_id);
                (session_id, session_name)
            };

            let hooks = hooks::load(
                args.pre_run.clone(),
                args.post_run.clone(),
                args.on_tool_call.clone(),
            );
            let cancel = CancellationToken::new();
            let signal_cancel = cancel.child();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                signal_cancel.cancel();
            });
            let hooks_ref = hooks.clone();
            let resumed_task = if let Some(session) = resumed_session.as_ref() {
                let prior_steps = memory.get_steps(&session.episode_id).await?;
                let prior_episode = memory.episode_by_id(&session.episode_id).await?;
                let continuation = format!(
                    "{}\n\nUser asks now: {}",
                    build_session_recap(session, prior_episode.as_ref(), &prior_steps),
                    args.task
                );
                continuation
            } else {
                args.task.clone()
            };
            let effective_task = wrap_for_remote(&resumed_task, cli.remote.as_ref());
            hooks::fire_pre_run(&hooks, &effective_task, cli.remote.as_deref()).await?;

            // Wrap sink for snapshots (U5).
            let base: ndjson_sink::DynSink = if args.emit_events {
                ndjson_sink::DynSink::new(ndjson_sink::TeeSink::new(
                    CliProgressSink,
                    ndjson_sink::NdjsonSink::new(),
                ))
            } else {
                ndjson_sink::DynSink::new(CliProgressSink)
            };
            let base_sink = hooks::HookEventSink::new(base, &hooks_ref, cli.remote.clone());
            let sink = snapshot_sink::SnapshotSink::new(
                base_sink,
                session_store.clone(),
                Some(session_id.clone()),
                tool_working_dir.clone(),
            );
            let mut last_error: Option<anyhow::Error> = None;
            let mut actual_provider_choice = provider_choice;
            let mut actual_model: Option<String> = None;
            let mut result: Option<RunResult> = None;

            for (index, choice) in providers_to_try.iter().copied().enumerate() {
                let attempt_settings = provider_settings.with_choice(choice);
                let (provider, model) = match build_provider(&attempt_settings, &secrets_store) {
                    Ok(provider_and_model) => provider_and_model,
                    Err(error) => {
                        let reason = error.to_string();
                        last_error = Some(anyhow!(reason.clone()));
                        if let Some(next_choice) = providers_to_try.get(index + 1).copied() {
                            eprintln!(
                                "[fallback] {} failed to initialize: {}, trying {}...",
                                provider_choice_name(choice),
                                reason,
                                provider_choice_name(next_choice)
                            );
                            sink.emit(AgentEvent::ProviderFailover {
                                from: provider_choice_name(choice).to_owned(),
                                to: provider_choice_name(next_choice).to_owned(),
                                reason,
                            });
                            continue;
                        }
                        return Err(error).context("all providers in fallback chain failed");
                    }
                };
                if has_fallback && choice != provider_choice {
                    eprintln!(
                        "[fallback] using {} with model {}",
                        provider_choice_name(choice),
                        model
                    );
                }
                let mut tool_context = ToolContext::new(tool_working_dir.clone());
                if let Some(policy) = sandbox.as_ref() {
                    tool_context = tool_context.with_sandbox(policy.policy());
                }
                if let Some(remote_target) = cli.remote.as_deref() {
                    tool_context = tool_context.with_remote_target(remote_target.to_owned());
                }
                let agent = ReactAgent::with_tool_context(
                    provider,
                    build_registry_with_skills(&builtin_skills::load_builtin_skills()),
                    memory.clone(),
                    budget,
                    model.clone(),
                    tool_context,
                )
                .with_cancel_token(cancel.child());
                match agent.run_with_events(&effective_task, &sink).await {
                    Ok(run_result) => {
                        actual_provider_choice = choice;
                        actual_model = Some(model);
                        result = Some(run_result);
                        break;
                    }
                    Err(HelmError::Provider(error)) => {
                        let reason = error.to_string();
                        last_error = Some(anyhow!(reason.clone()));
                        if let Some(next_choice) = providers_to_try.get(index + 1).copied() {
                            eprintln!(
                                "[fallback] {} runtime failure: {}, trying {}...",
                                provider_choice_name(choice),
                                reason,
                                provider_choice_name(next_choice)
                            );
                            sink.emit(AgentEvent::ProviderFailover {
                                from: provider_choice_name(choice).to_owned(),
                                to: provider_choice_name(next_choice).to_owned(),
                                reason,
                            });
                            continue;
                        }
                        return Err(HelmError::Provider(error).into());
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            let result = match result {
                Some(result) => result,
                None => {
                    return Err(last_error.unwrap_or_else(|| anyhow!("no providers configured")))
                        .context("all providers in fallback chain failed");
                }
            };
            hooks::fire_post_run(
                &hooks,
                &result.episode_id,
                result.iterations > 0,
                cli.remote.as_deref(),
            )
            .await?;
            let provider_name = match actual_provider_choice {
                ProviderChoice::Auto => "auto",
                ProviderChoice::Groq => "groq",
                ProviderChoice::Anthropic => "anthropic",
                ProviderChoice::Ollama => "ollama",
                ProviderChoice::Gemini => "gemini",
                ProviderChoice::Openrouter => "openrouter",
                ProviderChoice::NvidiaNim => "nvidia-nim",
                ProviderChoice::OpenaiCompat => "openai-compat",
            };

            // Update session metadata on completion (U7).
            if let Err(e) = session_store
                .update_session(
                    &session_id,
                    Some(&result.episode_id),
                    Some(provider_name),
                    actual_model.as_deref(),
                )
                .await
            {
                tracing::warn!("session update failed: {}", e);
            }

            eprintln!("[session] updated {} ({session_label})", session_id);
            print_run_result(&result);
        }
        Command::Replay(args) => {
            let transcript = render_replay(&memory, &args.episode_id).await?;
            print!("{transcript}");
        }
        Command::Models => {
            let base_url = provider_settings
                .base_url
                .clone()
                .unwrap_or_else(default_ollama_base_url);
            let report = render_models(&base_url).await?;
            print!("{report}");
        }
        Command::Doctor(args) => {
            let report = run_doctor(&provider_settings, &db_path, &memory, &secrets_store).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render_doctor(&report));
            }
        }
        Command::Diagnose(args) => {
            let (provider, model_name) = build_provider(&provider_settings, &secrets_store)?;
            let budget = Budget {
                read_only: true,
                dry_run: false,
                ..Default::default()
            };
            let tool_context =
                ToolContext::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
                    .with_diagnose_mode();
            let agent = ReactAgent::with_tool_context(
                provider,
                ToolRegistry::with_diagnose_tools(),
                memory.clone(),
                budget,
                model_name,
                tool_context,
            );
            eprintln!(
                "[diagnose] running in read-only mode with {} tools available",
                ToolRegistry::with_diagnose_tools().schemas().len()
            );
            let question = format!(
                "[diagnose mode — read-only, limited tools] Answer this question about the system using only available tools. Do not attempt to modify anything.\n\n{}",
                args.question
            );
            let result = agent.run(&question).await?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "answer": result.final_message,
                        "iterations": result.iterations,
                        "tokens_in": result.tokens_in,
                        "tokens_out": result.tokens_out
                    }))?
                );
            } else {
                println!("{}", result.final_message);
            }
        }
        Command::Episodes(args) => {
            let report = render_episodes(&memory, args.limit).await?;
            print!("{report}");
        }
        Command::Permissions(args) => match args.command {
            PermissionsCommand::List => {
                let report = render_permissions(&memory).await?;
                print!("{report}");
            }
            PermissionsCommand::Grant(args) => {
                let capability = parse_capability_arg(&args.capability)?;
                let scope = parse_scope_arg(&args.scope)?;
                let grant = memory.grant_capability(capability, scope).await?;
                println!(
                    "granted {} with scope {} (id {})",
                    grant.capability, grant.scope, grant.id
                );
            }
            PermissionsCommand::Revoke(args) => {
                let capability = parse_capability_arg(&args.capability)?;
                let revoked = memory.revoke_capability(capability).await?;
                println!("revoked {revoked} active grant(s) for {capability}");
            }
        },
        Command::Audit(args) => match args.command {
            AuditCommand::Verify(args) => {
                let verification = memory
                    .verify_audit_chain_for_target(args.target.as_deref())
                    .await?;
                if verification.ok {
                    match args.target.as_deref() {
                        Some(target) => {
                            println!(
                                "audit ok for {target}: checked {} event(s)",
                                verification.checked
                            )
                        }
                        None => println!("audit ok: checked {} event(s)", verification.checked),
                    }
                } else {
                    println!(
                        "audit FAILED at event {}: {}",
                        verification
                            .failed_at
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "unknown".to_owned()),
                        verification.reason.unwrap_or_else(|| "unknown".to_owned())
                    );
                }
            }
            AuditCommand::Show(args) => {
                let report =
                    render_audit_events(&memory, args.episode.as_deref(), args.target.as_deref())
                        .await?;
                print!("{report}");
            }
        },
        Command::Sessions(args) => match args.command {
            SessionsCommand::List { limit } => {
                let sessions_dir = dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                    .join("helm");
                let db_path = cli
                    .db_path
                    .clone()
                    .unwrap_or_else(|| sessions_dir.join("helm.db"));
                let store =
                    helm_memory::SessionStore::open(&db_path, sessions_dir.join("snapshots"))
                        .await?;
                let sessions = store.list_sessions(limit).await?;
                for s in sessions {
                    println!(
                        "[{}] {} | {} | {}",
                        s.id,
                        s.name,
                        s.goal.chars().take(50).collect::<String>(),
                        DateTime::from_timestamp_millis(s.updated_at)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_default()
                    );
                }
            }
            SessionsCommand::Delete { id } => {
                let sessions_dir = dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                    .join("helm");
                let db_path = cli
                    .db_path
                    .clone()
                    .unwrap_or_else(|| sessions_dir.join("helm.db"));
                let store =
                    helm_memory::SessionStore::open(&db_path, sessions_dir.join("snapshots"))
                        .await?;
                let deleted = store.delete_session(&id).await?;
                println!("deleted {} session(s)", deleted);
            }
            SessionsCommand::Export { id, format } => {
                let sessions_dir = dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                    .join("helm");
                let db_path = cli
                    .db_path
                    .clone()
                    .unwrap_or_else(|| sessions_dir.join("helm.db"));
                let store =
                    helm_memory::SessionStore::open(&db_path, sessions_dir.join("snapshots"))
                        .await?;
                let content = store.export_session(&id, &format).await?;
                println!("{content}");
            }
            SessionsCommand::Resume { id, task, show } => {
                let sessions_dir = dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                    .join("helm");
                let session_db = cli
                    .db_path
                    .clone()
                    .unwrap_or_else(|| sessions_dir.join("helm.db"));
                let store =
                    helm_memory::SessionStore::open(&session_db, sessions_dir.join("snapshots"))
                        .await?;
                let session = store
                    .get_session(&id)
                    .await?
                    .ok_or_else(|| anyhow!("session not found: {}", id))?;
                let prior_steps = memory.get_steps(&session.episode_id).await?;
                let prior_episode = memory.episode_by_id(&session.episode_id).await?;
                let recap = build_session_recap(&session, prior_episode.as_ref(), &prior_steps);
                println!("{recap}");
                if show || task.is_none() {
                    return Ok(());
                }
                let task = task.expect("checked is_none above");
                let provider_choice = resolve_provider_choice(provider_settings.choice);
                let (provider, model) = build_provider(
                    &provider_settings.with_choice(provider_choice),
                    &secrets_store,
                )?;
                let mut budget = Budget::default();
                if let Some(max_iterations) = cli.max_iterations {
                    budget.max_iterations = max_iterations;
                }
                budget.auto_approve = cli.yes;
                budget.read_only = cli.read_only;
                let cancel = CancellationToken::new();
                let agent = ReactAgent::new(
                    provider,
                    build_registry_with_skills(&builtin_skills::load_builtin_skills()),
                    memory.clone(),
                    budget,
                    model,
                )?
                .with_cancel_token(cancel.child());
                let signal_cancel = cancel.child();
                tokio::spawn(async move {
                    tokio::signal::ctrl_c().await.ok();
                    signal_cancel.cancel();
                });
                let continuation = format!(
                    "[Resumed from session {} ({})]\nPrior goal: {}\nPrior outcome: {}\n\nUser asks now: {}",
                    session.id,
                    session.name,
                    session.goal,
                    prior_episode
                        .as_ref()
                        .and_then(|e| e.outcome.as_deref())
                        .unwrap_or("(in progress)"),
                    task
                );
                let continuation = wrap_for_remote(&continuation, cli.remote.as_ref());
                let result = agent
                    .run_with_events(&continuation, &CliProgressSink)
                    .await?;
                print_run_result(&result);
            }
        },
        Command::Export(args) => {
            let episode = memory
                .episode_by_id(&args.episode_id)
                .await?
                .ok_or_else(|| anyhow!("episode not found"))?;
            let content = match args.format.as_str() {
                "json" => {
                    let obj = serde_json::json!({
                        "id": episode.id,
                        "goal": episode.goal,
                        "outcome": episode.outcome,
                        "started_at": episode.started_at,
                        "ended_at": episode.ended_at,
                        "tokens_in": episode.tokens_in,
                        "tokens_out": episode.tokens_out,
                        "final_message": episode.final_message,
                    });
                    serde_json::to_string_pretty(&obj).map_err(|e| anyhow!(e))?
                }
                "md" => {
                    let ts = DateTime::from_timestamp_millis(episode.started_at)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default();
                    format!(
                        "# Episode: {}\n\n**Goal:** {}\n**Outcome:** {}\n**Started:** {}\n\n```\n{}\n```",
                        episode.id,
                        episode.goal,
                        episode.outcome.as_deref().unwrap_or("unknown"),
                        ts,
                        episode.final_message.as_deref().unwrap_or("(no message)")
                    )
                }
                _ => return Err(anyhow!("unsupported format: {}", args.format)),
            };
            if let Some(path) = args.output {
                fs::write(&path, &content)?;
                println!("exported to {}", path.display());
            } else {
                print!("{content}");
            }
        }
        Command::Undo(args) => {
            run_snapshot_command(cli.db_path.clone(), args, false).await?;
        }
        Command::Redo(args) => {
            run_snapshot_command(cli.db_path.clone(), args, true).await?;
        }
        Command::Stats(_args) => {
            let counts = memory.episode_outcome_counts().await?;
            let _total = memory.episode_count().await?;
            println!(
                "Total episodes: {}\n  success: {}\n  partial: {}\n  failure: {}",
                counts.total, counts.success, counts.partial, counts.failure
            );
        }
        Command::Skills(args) => match args.command {
            SkillsCommand::List => {
                let mut skills = builtin_skills::load_builtin_skills();
                let manager = helm_memory::SkillsManager::new();
                for user_skill in manager.list().unwrap_or_default() {
                    if !skills.iter().any(|s| s.id == user_skill.id) {
                        skills.push(user_skill);
                    }
                }
                for skill in &skills {
                    let tag = if skill.source_path == std::path::Path::new("builtin") {
                        " [builtin]"
                    } else {
                        ""
                    };
                    println!("{}: {} (v{}){tag}", skill.id, skill.name, skill.version);
                }
            }
            SkillsCommand::Show(args) => {
                let builtins = builtin_skills::load_builtin_skills();
                let skill = builtins
                    .into_iter()
                    .find(|s| s.id == args.id)
                    .or_else(|| helm_memory::SkillsManager::new().show(&args.id).ok())
                    .ok_or_else(|| anyhow!("skill not found: {}", args.id))?;
                println!("ID: {}", skill.id);
                println!("Name: {}", skill.name);
                println!("Version: {}", skill.version);
                println!("Approved: {}", skill.approved);
                println!("\n{}", skill.content);
            }
            SkillsCommand::Approve(args) => {
                let manager = helm_memory::SkillsManager::new();
                manager.approve(&args.id)?;
                println!("Approved skill: {}", args.id);
            }
            SkillsCommand::Disable(args) => {
                let manager = helm_memory::SkillsManager::new();
                manager.disable(&args.id)?;
                println!("Disabled skill: {}", args.id);
            }
            SkillsCommand::Test(args) => {
                let manager = helm_memory::SkillsManager::new();
                let result = manager.test(&args.id)?;
                println!("{result}");
            }
            SkillsCommand::Run(args) => {
                let builtins = builtin_skills::load_builtin_skills();
                let skill = builtins
                    .into_iter()
                    .find(|s| s.id == args.id)
                    .or_else(|| helm_memory::SkillsManager::new().show(&args.id).ok())
                    .ok_or_else(|| anyhow!("skill not found: {}", args.id))?;
                let raw_cmds = builtin_skills::extract_bash_commands(&skill.content);
                if raw_cmds.is_empty() {
                    anyhow::bail!(
                        "skill '{}' has no executable bash commands in its SKILL.md",
                        args.id
                    );
                }
                let input_val: serde_json::Value = match args.input.as_deref() {
                    Some(s) => serde_json::from_str(s)
                        .map_err(|e| anyhow!("--input is not valid JSON: {e}"))?,
                    None => serde_json::Value::Object(serde_json::Map::new()),
                };
                if args.dry_run {
                    for cmd in &raw_cmds {
                        let mut resolved = cmd.clone();
                        if let Some(obj) = input_val.as_object() {
                            for (k, v) in obj {
                                let ph = format!("{{{{{k}}}}}");
                                let rep = match v {
                                    serde_json::Value::String(sv) => sv.clone(),
                                    other => other.to_string(),
                                };
                                resolved = resolved.replace(&ph, &rep);
                            }
                        }
                        println!("$ {resolved}");
                    }
                } else {
                    let tool = SkillTool::new(&skill.id, &skill.description, raw_cmds);
                    let ctx = ToolContext::new(std::env::current_dir()?);
                    let out = tool
                        .execute(input_val, &ctx)
                        .await
                        .map_err(|e| anyhow!("{e}"))?;
                    if !out.content.is_empty() {
                        print!("{}", out.content);
                    }
                    if !out.success {
                        anyhow::bail!("skill '{}' finished with errors", args.id);
                    }
                }
            }
        },
        Command::Mcp(args) => {
            run_mcp_command(args).await?;
        }
        Command::Secrets(args) => {
            run_secrets_command(args, &secrets_store)?;
        }
        Command::Init(args) => {
            interactive_init(
                &config_path,
                &db_path,
                args.force,
                args.no_validate,
                &secrets_store,
            )
            .await?;
        }
        Command::Config(args) => {
            run_config_command(args, &config_path)?;
        }
        Command::Completion(args) => {
            let mut cmd = Cli::command();
            generate(args.shell, &mut cmd, "helm", &mut io::stdout());
        }
        Command::Tui(args) => {
            if let Some(target) = args.attach.as_deref() {
                let token = args
                    .token
                    .clone()
                    .or_else(|| env::var("HELM_REMOTE_TOKEN").ok())
                    .ok_or_else(|| {
                        anyhow!("--attach requires --token or HELM_REMOTE_TOKEN env var")
                    })?;
                run_attach_session(target, &token).await?;
            } else {
                tui::run_tui(tui::TuiRuntime {
                    provider_settings,
                    db_path,
                    memory,
                    max_iterations: cli.max_iterations,
                    config_path,
                    secrets: secrets_store,
                    tui_paste_key_modal: config
                        .as_ref()
                        .and_then(|config| config.security.as_ref())
                        .and_then(|security| security.tui_paste_key_modal)
                        .unwrap_or(true),
                    auto_approve: cli.yes,
                    read_only: cli.read_only,
                    diagnose_mode: false,
                    sandbox: sandbox.clone(),
                    remote_target: cli.remote.clone(),
                })
                .await?;
            }
        }
        Command::Memory(args) => {
            run_memory_command(args, &memory).await?;
        }
        Command::Profile(args) => {
            run_profile_command(args, &db_path, &memory).await?;
        }
        Command::Remote(args) => {
            run_remote_command(args).await?;
        }
        Command::Serve(args) => {
            run_serve_command(
                args,
                provider_settings,
                memory,
                cli.max_iterations,
                cli.yes,
                cli.read_only,
                &secrets_store,
            )
            .await?;
        }
        Command::Bootstrap(args) => {
            run_bootstrap_command(args).await?;
        }
        Command::TrustReport(args) => {
            let report =
                run_trust_report(&memory, &secrets_store, &provider_settings, &args).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render_trust_report(&report));
            }
        }
        Command::Snapshot(args) => {
            run_collect_snapshot_command(args).await?;
        }
    }
    Ok(())
}

async fn run_bootstrap_command(args: BootstrapArgs) -> Result<()> {
    let (parsed_user, host) = match args.host.split_once('@') {
        Some((u, h)) => (Some(u.to_owned()), h.to_owned()),
        None => (None, args.host.clone()),
    };
    let user = args.user.or(parsed_user);
    let plan = bootstrap::BootstrapPlan {
        host,
        user,
        port: args.port,
        release_url: args.release_url.clone(),
        register_as: args.register_as.clone(),
    };
    let local_binary = match args.local_binary {
        Some(p) => p,
        None => env::current_exe().context("locating local helm binary")?,
    };
    let report = bootstrap::run(plan, local_binary).await?;
    println!("[bootstrap] host: {}", report.host);
    println!("[bootstrap] remote uname: {}", report.remote_uname);
    println!("[bootstrap] helm version: {}", report.remote_helm_version);
    println!("[bootstrap] installed at: {}", report.installed_path);
    if let Some(name) = report.registered_as.as_deref() {
        println!(
            "[bootstrap] registered as `{name}` in {}",
            crate::remote::registry_path()?.display()
        );
    } else {
        println!(
            "[bootstrap] (target not registered. Pass --register-as <name> to add to remotes registry.)"
        );
    }
    Ok(())
}

async fn run_mcp_command(args: McpArgs) -> Result<()> {
    let config_path = helm_tools::default_mcp_config_path()
        .ok_or_else(|| anyhow::anyhow!("could not determine HOME directory"))?;

    match args.command {
        McpCommand::List => {
            let config = helm_tools::load_mcp_config().map_err(|e| anyhow::anyhow!("{e}"))?;
            if config.servers.is_empty() {
                println!("No MCP servers configured.");
                println!("Add one with: helm mcp add <name> <command> [args...]");
            } else {
                for server in &config.servers {
                    let args_str = if server.args.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", server.args.join(" "))
                    };
                    println!("{}: {}{}", server.name, server.command, args_str);
                }
            }
        }
        McpCommand::Add(add_args) => {
            let mut config = if config_path.exists() {
                let raw = std::fs::read_to_string(&config_path)
                    .map_err(|e| anyhow::anyhow!("failed to read config: {e}"))?;
                toml::from_str::<helm_tools::McpConfig>(&raw)
                    .map_err(|e| anyhow::anyhow!("malformed config: {e}"))?
            } else {
                helm_tools::McpConfig::default()
            };

            if config.servers.iter().any(|s| s.name == add_args.name) {
                anyhow::bail!("server '{}' already configured", add_args.name);
            }

            config.servers.push(helm_tools::McpServerConfig {
                name: add_args.name.clone(),
                command: add_args.command,
                args: add_args.args,
                env: vec![],
            });

            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow::anyhow!("failed to create config dir: {e}"))?;
            }

            let mut out = String::new();
            for server in &config.servers {
                out.push_str("\n[[servers]]\n");
                out.push_str(&format!("name = {:?}\n", server.name));
                out.push_str(&format!("command = {:?}\n", server.command));
                if !server.args.is_empty() {
                    let args_toml: Vec<String> =
                        server.args.iter().map(|a| format!("{a:?}")).collect();
                    out.push_str(&format!("args = [{}]\n", args_toml.join(", ")));
                }
            }

            std::fs::write(&config_path, out.trim_start())
                .map_err(|e| anyhow::anyhow!("failed to write config: {e}"))?;
            println!("Added MCP server '{}'", add_args.name);
        }
        McpCommand::Remove(rem_args) => {
            let mut config = if config_path.exists() {
                let raw = std::fs::read_to_string(&config_path)
                    .map_err(|e| anyhow::anyhow!("failed to read config: {e}"))?;
                toml::from_str::<helm_tools::McpConfig>(&raw)
                    .map_err(|e| anyhow::anyhow!("malformed config: {e}"))?
            } else {
                helm_tools::McpConfig::default()
            };

            let before = config.servers.len();
            config.servers.retain(|s| s.name != rem_args.name);
            if config.servers.len() == before {
                anyhow::bail!("no server named '{}'", rem_args.name);
            }

            let mut out = String::new();
            for server in &config.servers {
                out.push_str("\n[[servers]]\n");
                out.push_str(&format!("name = {:?}\n", server.name));
                out.push_str(&format!("command = {:?}\n", server.command));
                if !server.args.is_empty() {
                    let args_toml: Vec<String> =
                        server.args.iter().map(|a| format!("{a:?}")).collect();
                    out.push_str(&format!("args = [{}]\n", args_toml.join(", ")));
                }
            }

            std::fs::write(&config_path, out.trim_start())
                .map_err(|e| anyhow::anyhow!("failed to write config: {e}"))?;
            println!("Removed MCP server '{}'", rem_args.name);
        }
        McpCommand::Test(test_args) => {
            let cwd = env::current_dir().context("failed to determine current directory")?;
            let output = helm_tools::McpTool
                .execute(
                    json!({
                        "action": "list_tools",
                        "server": test_args.name,
                    }),
                    &ToolContext::new(cwd),
                )
                .await
                .map_err(|e| anyhow!("{e}"))?;
            println!("{}", output.content);
        }
        McpCommand::Run(run_args) => {
            let arguments: serde_json::Value = serde_json::from_str(&run_args.arguments)
                .with_context(|| format!("invalid JSON for --arguments: {}", run_args.arguments))?;
            if !arguments.is_object() {
                anyhow::bail!("--arguments must be a JSON object");
            }
            let cwd = env::current_dir().context("failed to determine current directory")?;
            let output = helm_tools::McpTool
                .execute(
                    json!({
                        "action": "call",
                        "server": run_args.server,
                        "tool": run_args.tool,
                        "arguments": arguments,
                    }),
                    &ToolContext::new(cwd),
                )
                .await
                .map_err(|e| anyhow!("{e}"))?;
            println!("{}", output.content);
        }
    }
    Ok(())
}

fn run_secrets_command(args: SecretsArgs, store: &SecretsStore) -> Result<()> {
    match args.command {
        SecretsCommand::List => {
            let names = store.list_names().map_err(|e| anyhow!("{e}"))?;
            if names.is_empty() {
                println!("no secrets stored");
            } else {
                for name in names {
                    println!("{name}");
                }
            }
        }
        SecretsCommand::Set(args) => {
            let value = if args.from_stdin {
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf.trim_end_matches('\n').trim_end_matches('\r').to_owned()
            } else if let Some(v) = args.value {
                eprintln!(
                    "warning: --value can be recorded in shell history; prefer masked prompt or --from-stdin"
                );
                v
            } else if !io::stdin().is_terminal() {
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf.trim_end_matches('\n').trim_end_matches('\r').to_owned()
            } else {
                rpassword::prompt_password(format!("Value for {} (masked): ", args.name))
                    .map_err(|e| anyhow!("failed to read password: {e}"))?
            };
            let chars = value.chars().count();
            if value.is_empty() {
                return Err(anyhow!("value cannot be empty"));
            }
            store
                .set(&args.name, Secret::new(value))
                .map_err(|e| anyhow!("{e}"))?;
            println!("set {} ({} chars, mode 0600)", args.name, chars);
        }
        SecretsCommand::Get(args) => match store.get(&args.name).map_err(|e| anyhow!("{e}"))? {
            Some(s) => println!("{}", s.expose()),
            None => return Err(anyhow!("no secret stored for {}", args.name)),
        },
        SecretsCommand::Delete(args) => {
            if io::stdin().is_terminal() {
                let answer = prompt(&format!("Delete {} from secrets store? [y/N] ", args.name))?;
                if !answer.eq_ignore_ascii_case("y") {
                    println!("aborted");
                    return Ok(());
                }
            }
            if store.delete(&args.name).map_err(|e| anyhow!("{e}"))? {
                println!("deleted {}", args.name);
            } else {
                println!("no secret stored for {}", args.name);
            }
        }
        SecretsCommand::Path => {
            println!("{}", store.path().display());
        }
        SecretsCommand::ImportEnv => {
            let env_vars = [
                "ANTHROPIC_API_KEY",
                "GROQ_API_KEY",
                "OPENAI_API_KEY",
                "OPENROUTER_API_KEY",
                "NVIDIA_API_KEY",
                "GOOGLE_API_KEY",
                "GEMINI_API_KEY",
            ];
            let mut imported = 0usize;
            for var in env_vars {
                if let Ok(v) = env::var(var) {
                    if !v.is_empty() {
                        if store.get(var).map_err(|e| anyhow!("{e}"))?.is_some() {
                            eprintln!("warning: overwriting stored {var}");
                        }
                        store.set(var, Secret::new(v)).map_err(|e| anyhow!("{e}"))?;
                        println!("imported {var}");
                        imported += 1;
                    }
                }
            }
            println!("imported {imported} secret(s)");
        }
    }
    Ok(())
}

fn run_config_command(args: ConfigArgs, config_path: &Path) -> Result<()> {
    match args.command {
        ConfigCommand::Get(args) => {
            let config_text = fs::read_to_string(config_path)
                .with_context(|| format!("failed to read config from {}", config_path.display()))?;
            let value = config_text.parse::<toml::Value>()?;
            let mut current = &value;
            for part in args.key.split('.') {
                current = current
                    .get(part)
                    .ok_or_else(|| anyhow!("Key not found: {}", args.key))?;
            }
            if let Some(s) = current.as_str() {
                println!("{}", s);
            } else if let Some(i) = current.as_integer() {
                println!("{}", i);
            } else if let Some(b) = current.as_bool() {
                println!("{}", b);
            } else if let Some(f) = current.as_float() {
                println!("{}", f);
            } else {
                println!("{}", current);
            }
        }
        ConfigCommand::Set(args) => {
            let config_text = if config_path.exists() {
                fs::read_to_string(config_path).unwrap_or_default()
            } else {
                String::new()
            };
            let mut value: toml::Value = config_text
                .parse()
                .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

            let parts: Vec<&str> = args.key.split('.').collect();
            if parts.is_empty() {
                return Err(anyhow!("Invalid key"));
            }

            let parsed_val = if args.value == "true" {
                toml::Value::Boolean(true)
            } else if args.value == "false" {
                toml::Value::Boolean(false)
            } else if let Ok(i) = args.value.parse::<i64>() {
                toml::Value::Integer(i)
            } else if let Ok(f) = args.value.parse::<f64>() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(args.value.clone())
            };

            set_toml_path(&mut value, &parts, parsed_val)?;
            let new_text = toml::to_string_pretty(&value)?;
            ensure_parent_dir(config_path)?;
            fs::write(config_path, new_text)
                .with_context(|| format!("failed to write config to {}", config_path.display()))?;
            println!("Set {} = {}", args.key, args.value);
        }
        ConfigCommand::Edit => {
            let editor = env::var("EDITOR").unwrap_or_else(|_| "nano".to_owned());
            ensure_parent_dir(config_path)?;
            if !config_path.exists() {
                fs::write(config_path, "")?;
            }
            let status = std::process::Command::new(editor)
                .arg(config_path)
                .status()
                .with_context(|| "Failed to open editor")?;
            if !status.success() {
                return Err(anyhow!("Editor exited with non-zero status"));
            }
        }
        ConfigCommand::Validate => {
            if !config_path.exists() {
                println!("Config file does not exist at {}", config_path.display());
                return Ok(());
            }
            let config_text = fs::read_to_string(config_path)?;
            match toml::from_str::<FileConfig>(&config_text) {
                Ok(config) => {
                    resolve_provider_settings_with_env(
                        Some(&config),
                        None,
                        None,
                        None,
                        None,
                        |_| None,
                    )?;
                    println!("Config is valid.");
                }
                Err(e) => println!("Config is invalid: {}", e),
            }
        }
        ConfigCommand::Path => {
            println!("{}", config_path.display());
        }
    }
    Ok(())
}

fn parse_cli_from<I, T>(args: I) -> Result<Cli>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let raw = args.into_iter().map(Into::into).collect::<Vec<_>>();
    if let Some(error) = unknown_subcommand_error(&raw) {
        return Err(anyhow!(error));
    }
    Cli::try_parse_from(normalize_args(raw)).map_err(|error| anyhow!(error.to_string()))
}

fn load_config(path: &Path) -> Result<Option<FileConfig>, CliConfigError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CliConfigError::Read {
                path: path.to_path_buf(),
                message: error.to_string(),
            });
        }
    };
    toml::from_str::<FileConfig>(&source)
        .map(Some)
        .map_err(|error| CliConfigError::Malformed {
            path: path.to_path_buf(),
            line: toml_error_line(&source, &error),
            message: error.message().to_owned(),
        })
}

fn toml_error_line(source: &str, error: &toml::de::Error) -> usize {
    error
        .span()
        .map(|span| {
            source[..span.start]
                .chars()
                .filter(|ch| *ch == '\n')
                .count()
                + 1
        })
        .unwrap_or(1)
}

fn resolve_provider_settings(
    config: Option<&FileConfig>,
    cli_provider: Option<ProviderChoice>,
    cli_base_url: Option<String>,
    cli_model: Option<String>,
    cli_api_key: Option<String>,
) -> Result<ProviderSettings> {
    resolve_provider_settings_with_env(
        config,
        cli_provider,
        cli_base_url,
        cli_model,
        cli_api_key,
        |name| env::var(name).ok(),
    )
}

fn resolve_provider_settings_with_env<F>(
    config: Option<&FileConfig>,
    cli_provider: Option<ProviderChoice>,
    cli_base_url: Option<String>,
    cli_model: Option<String>,
    cli_api_key: Option<String>,
    env_lookup: F,
) -> Result<ProviderSettings>
where
    F: Fn(&str) -> Option<String>,
{
    let provider_config = config.and_then(|config| config.provider.as_ref());
    let base_url = cli_base_url.or_else(|| {
        provider_config.and_then(|provider| provider.base_url.as_ref().map(ToOwned::to_owned))
    });
    let model = cli_model.or_else(|| env_lookup("HELM_MODEL")).or_else(|| {
        provider_config.and_then(|provider| provider.model.as_ref().map(ToOwned::to_owned))
    });
    let api_key_env = provider_config.and_then(|provider| provider.api_key_env.clone());
    let stored_api_key = cli_api_key;

    let selected = if let Some(choice) = cli_provider {
        Some((choice, ProviderSource::Cli))
    } else if let Some(value) = env_lookup("HELM_PROVIDER") {
        Some((
            parse_provider_choice(&value)?,
            ProviderSource::HelmProviderEnv,
        ))
    } else {
        provider_config
            .and_then(|provider| provider.kind)
            .map(|choice| (choice, ProviderSource::ConfigFile))
    };

    let mut settings = match selected {
        Some((ProviderChoice::Auto, _)) | None => {
            auto_detect_provider_settings(base_url, model, api_key_env, stored_api_key, &env_lookup)
        }
        Some((choice, source)) => ProviderSettings {
            choice,
            base_url,
            model,
            api_key_env,
            api_key: stored_api_key,
            source,
        },
    };

    apply_provider_defaults(&mut settings);
    Ok(settings)
}

fn parse_provider_choice(value: &str) -> Result<ProviderChoice> {
    ProviderChoice::from_str(value, true)
        .map_err(|_| anyhow!("invalid HELM_PROVIDER value: {value}"))
}

fn auto_detect_provider_settings<F>(
    base_url: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    stored_api_key: Option<String>,
    env_lookup: &F,
) -> ProviderSettings
where
    F: Fn(&str) -> Option<String>,
{
    for (env_name, choice) in [
        ("GROQ_API_KEY", ProviderChoice::Groq),
        ("ANTHROPIC_API_KEY", ProviderChoice::Anthropic),
        ("OPENAI_API_KEY", ProviderChoice::OpenaiCompat),
        ("OPENROUTER_API_KEY", ProviderChoice::Openrouter),
        ("NVIDIA_API_KEY", ProviderChoice::NvidiaNim),
        ("GOOGLE_API_KEY", ProviderChoice::Gemini),
        ("GEMINI_API_KEY", ProviderChoice::Gemini),
    ] {
        if env_lookup(env_name).is_some() {
            info!(
                provider = ?choice,
                env = env_name,
                "auto-detected provider from environment"
            );
            let detected_base_url = if env_name == "OPENAI_API_KEY" {
                Some("https://api.openai.com/v1".to_owned())
            } else {
                base_url.clone()
            };
            return ProviderSettings {
                choice,
                base_url: detected_base_url,
                model,
                api_key_env: Some(env_name.to_owned()),
                api_key: stored_api_key,
                source: ProviderSource::EnvVar(env_name),
            };
        }
    }
    ProviderSettings {
        choice: ProviderChoice::Ollama,
        base_url: base_url.or_else(|| Some(default_ollama_base_url())),
        model,
        api_key_env,
        api_key: stored_api_key,
        source: ProviderSource::Fallback,
    }
}

fn apply_provider_defaults(settings: &mut ProviderSettings) {
    if settings.api_key_env.is_none() {
        settings.api_key_env = default_api_key_env(settings.choice).map(str::to_owned);
    }
    if settings.choice == ProviderChoice::OpenaiCompat
        && settings.api_key_env.as_deref() == Some("OPENAI_API_KEY")
        && settings.base_url.is_none()
    {
        settings.base_url = Some("https://api.openai.com/v1".to_owned());
    }
    if settings.choice == ProviderChoice::Ollama && settings.base_url.is_none() {
        settings.base_url = Some(default_ollama_base_url());
    }
}

fn normalize_args(args: Vec<OsString>) -> Vec<OsString> {
    if args.len() <= 1 {
        let mut normalized = args;
        normalized.push(OsString::from("tui"));
        return normalized;
    }
    let mut normalized = Vec::with_capacity(args.len().saturating_add(1));
    if let Some(program) = args.first() {
        normalized.push(program.clone());
    }
    let mut index = 1_usize;
    while index < args.len() {
        let arg = &args[index];
        if is_value_taking_flag(arg) {
            normalized.push(arg.clone());
            if let Some(value) = args.get(index.saturating_add(1)) {
                normalized.push(value.clone());
                index = index.saturating_add(2);
                continue;
            }
        }
        if is_long_flag_with_value(arg) || arg.to_string_lossy().starts_with('-') {
            normalized.push(arg.clone());
            index = index.saturating_add(1);
            continue;
        }
        if is_known_command(arg) {
            normalized.extend(args[index..].iter().cloned());
        } else {
            normalized.push(OsString::from("run"));
            normalized.extend(args[index..].iter().cloned());
        }
        return normalized;
    }
    normalized
}

fn is_known_command(arg: &OsString) -> bool {
    matches!(
        arg.to_str(),
        Some(
            "run"
                | "replay"
                | "models"
                | "doctor"
                | "episodes"
                | "permissions"
                | "audit"
                | "skills"
                | "secrets"
                | "init"
                | "config"
                | "completion"
                | "mcp"
                | "help"
                | "tui"
                | "sessions"
                | "export"
                | "undo"
                | "redo"
                | "stats"
                | "memory"
                | "profile"
                | "remote"
                | "serve"
                | "bootstrap"
                | "diagnose"
                | "trust-report"
        )
    )
}

fn is_value_taking_flag(arg: &OsString) -> bool {
    matches!(
        arg.to_str(),
        Some(
            "--db-path"
                | "--max-iterations"
                | "--model"
                | "--provider"
                | "--api-key"
                | "--base-url"
                | "--ollama-url"
                | "--sandbox-dir"
                | "--remote"
                | "--resume",
        )
    )
}

fn is_long_flag_with_value(arg: &OsString) -> bool {
    arg.to_string_lossy().starts_with("--") && arg.to_string_lossy().contains('=')
}

fn unknown_subcommand_error(args: &[OsString]) -> Option<String> {
    let mut positionals = Vec::new();
    let mut index = 1_usize;
    while index < args.len() {
        let arg = &args[index];
        if is_value_taking_flag(arg) {
            index = index.saturating_add(2);
            continue;
        }
        if is_long_flag_with_value(arg) || arg.to_string_lossy().starts_with('-') {
            index = index.saturating_add(1);
            continue;
        }
        positionals.push(arg);
        index = index.saturating_add(1);
    }
    let first = positionals.first()?;
    if is_known_command(first) || positionals.len() <= 1 {
        return None;
    }
    Some(format!(
        "unknown subcommand: {}; use `helm run \"...\"` for task text",
        first.to_string_lossy()
    ))
}

/// Prints tool-start/finish lines to stderr while the agent runs.
struct CliProgressSink;

impl AgentEventSink for CliProgressSink {
    fn emit(&self, event: AgentEvent) {
        match event {
            AgentEvent::ToolCallStarted { name, .. } => {
                eprintln!("[tool] {name} …");
            }
            AgentEvent::ToolCallFinished {
                name,
                success,
                content,
                ..
            } => {
                let status = if success { "ok" } else { "err" };
                let preview: String = content.chars().take(80).collect();
                eprintln!("[{status}] {name}: {preview}");
            }
            AgentEvent::TextDelta { chunk } => {
                // Progressive output to stdout when the terminal is interactive.
                if std::io::stdout().is_terminal() {
                    print!("{chunk}");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
            }
            _ => {}
        }
    }
}

fn print_run_result(result: &RunResult) {
    let stdout = render_run_stdout(result);
    print!("{stdout}");
    if !stdout.ends_with('\n') {
        println!();
    }
    eprintln!(
        "[episode {}] {} iters, {}/{} tokens",
        result.episode_id, result.iterations, result.tokens_in, result.tokens_out
    );
    if result.model_capability_warning.is_some() {
        eprintln!("{}", model_capability_warning_text());
    }
}

fn render_run_stdout(result: &RunResult) -> String {
    let final_message = result.final_message.trim();
    if !final_message.is_empty() && final_message != "(no final message)" {
        return format!("{}\n", result.final_message);
    }
    if let Some(last_text) = result
        .last_assistant_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        return format!("[last assistant message]\n{last_text}\n");
    }
    "[no assistant text was produced - the model may not support tool calling. run `helm models` to check.]\n"
        .to_owned()
}

fn model_capability_warning_text() -> &'static str {
    "warning: the model emitted tool-shaped JSON in plain text. this usually means\n\
the model does not support native tool calling. try a tools-capable model:\n\
  qwen3:4b, qwen3:8b, llama3.3:8b, hermes4:8b, mistral-small3:24b"
}

pub(crate) fn build_session_recap(
    session: &helm_memory::SessionRecord,
    episode: Option<&EpisodeRecord>,
    steps: &[StepRecord],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "resuming session {} ({})\nepisode: {}\ngoal: {}\n",
        session.id, session.name, session.episode_id, session.goal,
    ));
    if let Some(ep) = episode {
        out.push_str(&format!(
            "started: {}\nprior outcome: {} ({} iters, {}/{} tokens)\n",
            format_timestamp(ep.started_at),
            ep.outcome.as_deref().unwrap_or("running"),
            ep.iterations,
            ep.tokens_in,
            ep.tokens_out,
        ));
    }
    out.push_str(&format!("prior steps: {}\n", steps.len()));
    out
}

async fn render_replay(memory: &MemoryStore, episode_id: &str) -> Result<String> {
    let episode = memory
        .get_episode(episode_id)
        .await?
        .ok_or_else(|| anyhow!("episode not found: {episode_id}"))?;
    let steps = memory.get_steps(episode_id).await?;
    Ok(format_transcript(&episode, &steps))
}

fn resolve_resume_target(resume: Option<&str>, continue_last: bool) -> Option<&str> {
    if let Some(resume) = resume {
        Some(resume)
    } else if continue_last {
        Some("latest")
    } else {
        None
    }
}

async fn resolve_session_for_resume(
    store: &SessionStore,
    target: &str,
) -> Result<helm_memory::SessionRecord> {
    if target == "latest" {
        return store
            .latest_session()
            .await?
            .ok_or_else(|| anyhow!("no sessions available to resume"));
    }
    store
        .get_session(target)
        .await?
        .ok_or_else(|| anyhow!("session not found: {target}"))
}

fn derive_session_name(goal: &str) -> String {
    let mut parts = Vec::new();
    for word in goal.split_whitespace() {
        let clean: String = word
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .map(|ch| ch.to_ascii_lowercase())
            .collect();
        if !clean.is_empty() {
            parts.push(clean);
        }
        if parts.len() >= 6 {
            break;
        }
    }
    if parts.is_empty() {
        format!("run-{}", chrono::Utc::now().format("%Y%m%d-%H%M"))
    } else {
        parts.join("-")
    }
}

async fn run_snapshot_command(
    db_override: Option<PathBuf>,
    args: UndoArgs,
    redo: bool,
) -> Result<()> {
    let sessions_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("helm");
    let db_path = db_override.unwrap_or_else(|| sessions_dir.join("helm.db"));
    let store = helm_memory::SessionStore::open(&db_path, sessions_dir.join("snapshots")).await?;
    let Some(session_id) = args.session_id.as_deref() else {
        let verb = if redo { "redo" } else { "undo" };
        println!("use --session-id to specify a session for {verb}");
        return Ok(());
    };
    let snapshots = if redo {
        store.list_redo_snapshots(session_id).await?
    } else {
        store.list_snapshots(session_id).await?
    };
    if snapshots.is_empty() {
        let verb = if redo { "redo" } else { "undo" };
        println!("no {verb} snapshots for session {}", session_id);
        return Ok(());
    }
    let idx = (args.n.saturating_sub(1) as usize).min(snapshots.len() - 1);
    let snap = &snapshots[idx];
    if args.apply {
        if !redo {
            let current_content = fs::read_to_string(&snap.file_path)
                .with_context(|| format!("reading {}", snap.file_path.display()))?;
            store
                .take_redo_snapshot(
                    session_id,
                    snap.step_index,
                    &current_content,
                    &snap.file_path,
                )
                .await?;
        }
        let written = if redo {
            store.apply_redo_snapshot(&snap.id, args.to.clone()).await?
        } else {
            store.apply_snapshot(&snap.id, args.to.clone()).await?
        };
        let verb = if redo { "reapplied" } else { "restored" };
        println!(
            "{verb} snapshot {} (step {}) to {}",
            snap.id,
            snap.step_index,
            written.display()
        );
    } else {
        let content = if redo {
            store.restore_redo_snapshot(&snap.id).await?
        } else {
            store.restore_snapshot(&snap.id).await?
        };
        let label = if redo { "redo snapshot" } else { "snapshot" };
        println!(
            "{label} {} (step {}, file {}):\n{}\n[dry-run; pass --apply to write to disk]",
            snap.id,
            snap.step_index,
            snap.file_path.display(),
            content
        );
    }
    Ok(())
}

#[allow(dead_code)]
async fn create_session_for_run(
    db_path: &Path,
    episode_id: &str,
    goal: &str,
    model: Option<&str>,
    provider: Option<&str>,
) -> Result<()> {
    let sessions_dir = dirs::data_local_dir()
        .ok_or_else(|| anyhow!("could not resolve data directory"))?
        .join("helm");
    let snapshots_dir = sessions_dir.join("snapshots");
    let store = helm_memory::SessionStore::open(db_path, snapshots_dir).await?;
    let session_name = derive_session_name(goal);
    store
        .create_session(
            &session_name,
            goal,
            episode_id.to_owned(),
            model.map(|s| s.to_owned()),
            provider.map(|s| s.to_owned()),
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
        )
        .await?;
    eprintln!(
        "[session] created session {} for episode {}",
        session_name, episode_id
    );
    Ok(())
}

fn format_transcript(episode: &EpisodeRecord, steps: &[StepRecord]) -> String {
    let mut output = String::new();
    output.push_str(&format!("episode {}\n", episode.id));
    output.push_str(&format!("goal: {}\n", episode.goal));
    output.push_str(&format!(
        "started: {}\n",
        format_timestamp(episode.started_at)
    ));
    output.push_str(&format!(
        "outcome: {} ({} iters, {}/{} tokens)\n",
        episode.outcome.as_deref().unwrap_or("running"),
        episode.iterations,
        episode.tokens_in,
        episode.tokens_out
    ));
    if episode.corrections_used > 0 {
        output.push_str(&format!("corrections_used: {}\n", episode.corrections_used));
    }
    if episode.format_recovery_used {
        output.push_str("format_recovery_used: true\n");
    }
    if let Some(warning) = &episode.model_capability_warning {
        output.push_str(&format!("warning: model_capability_warning: {warning}\n"));
    }
    let mut tool_names_by_id = std::collections::HashMap::new();
    for step in steps {
        output.push('\n');
        output.push_str(&format_step_header(step, &tool_names_by_id));
        output.push('\n');
        output.push_str(&format_step_content(step, &mut tool_names_by_id));
        if !output.ends_with('\n') {
            output.push('\n');
        }
    }
    output
}

fn format_step_header(
    step: &StepRecord,
    tool_names_by_id: &std::collections::HashMap<String, String>,
) -> String {
    let tool_name = step.content.iter().find_map(|block| match block {
        ContentBlock::ToolResult { tool_use_id, .. } => tool_names_by_id.get(tool_use_id.as_str()),
        _ => None,
    });
    match tool_name {
        Some(name) if step.role == "tool" => {
            format!("[step {}] tool ({name})", step.step_index)
        }
        _ => format!("[step {}] {}", step.step_index, step.role),
    }
}

fn format_step_content(
    step: &StepRecord,
    tool_names_by_id: &mut std::collections::HashMap<String, String>,
) -> String {
    let mut parts = Vec::new();
    for block in &step.content {
        match block {
            ContentBlock::Text(text) => parts.push(text.clone()),
            ContentBlock::ToolUse { id, name, input } => {
                tool_names_by_id.insert(id.clone(), name.clone());
                parts.push(json!({"name": name, "arguments": input}).to_string());
            }
            ContentBlock::ToolResult { content, .. } => parts.push(content.clone()),
        }
    }
    parts.join("\n")
}

async fn render_episodes(memory: &MemoryStore, limit: u32) -> Result<String> {
    let episodes = memory.recent_episodes(limit).await?;
    let mut output = String::from(
        "EPISODE                              OUTCOME   ITERS  TOKENS       STARTED              GOAL\n",
    );
    for episode in episodes {
        output.push_str(&format!(
            "{:<36} {:<9} {:<6} {:>5}/{:<5} {:<19} {}\n",
            episode.id,
            episode.outcome.as_deref().unwrap_or("running"),
            episode.iterations,
            episode.tokens_in,
            episode.tokens_out,
            format_timestamp(episode.started_at),
            truncate_goal(&episode.goal)
        ));
    }
    Ok(output)
}

async fn render_permissions(memory: &MemoryStore) -> Result<String> {
    let grants = memory.list_capability_grants().await?;
    Ok(format_permissions(&grants))
}

fn format_permissions(grants: &[CapabilityGrantRecord]) -> String {
    let mut output =
        String::from("ID                                   CAPABILITY      SCOPE    STATUS\n");
    for grant in grants {
        output.push_str(&format!(
            "{:<36} {:<15} {:<8} {}\n",
            grant.id,
            grant.capability,
            grant.scope,
            grant_status(grant)
        ));
    }
    output
}

fn grant_status(grant: &CapabilityGrantRecord) -> &'static str {
    if grant.revoked_at.is_some() {
        "revoked"
    } else if grant
        .expires_at
        .is_some_and(|expires_at| expires_at <= Utc::now().timestamp_millis())
    {
        "expired"
    } else {
        "active"
    }
}

async fn render_audit_events(
    memory: &MemoryStore,
    episode: Option<&str>,
    target: Option<&str>,
) -> Result<String> {
    let events = memory.audit_events(episode, target).await?;
    Ok(format_audit_events(&events))
}

pub(crate) fn set_toml_path(
    root: &mut toml::Value,
    parts: &[&str],
    value: toml::Value,
) -> Result<()> {
    if parts.is_empty() {
        return Err(anyhow!("invalid empty key path"));
    }
    if !root.is_table() {
        *root = toml::Value::Table(toml::map::Map::new());
    }
    let mut current = root;
    for part in &parts[..parts.len().saturating_sub(1)] {
        let Some(table) = current.as_table_mut() else {
            return Err(anyhow!("cannot descend into non-table for key `{part}`"));
        };
        if !table.contains_key(*part) {
            table.insert(
                (*part).to_owned(),
                toml::Value::Table(toml::map::Map::new()),
            );
        }
        let Some(next) = table.get_mut(*part) else {
            return Err(anyhow!("failed to create key path `{part}`"));
        };
        if !next.is_table() {
            *next = toml::Value::Table(toml::map::Map::new());
        }
        current = next;
    }
    let Some(last) = parts.last() else {
        return Err(anyhow!("invalid key path"));
    };
    let Some(table) = current.as_table_mut() else {
        return Err(anyhow!("cannot write into non-table at `{last}`"));
    };
    table.insert((*last).to_owned(), value);
    Ok(())
}

pub(crate) fn write_helm_config(
    config_path: &Path,
    db_path: &Path,
    kind: &str,
    model: &str,
    base_url: Option<&str>,
    api_key_env: Option<&str>,
) -> Result<()> {
    ensure_parent_dir(config_path)?;
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut provider_table = toml::map::Map::new();
    provider_table.insert("kind".to_owned(), toml::Value::String(kind.to_owned()));
    provider_table.insert("model".to_owned(), toml::Value::String(model.to_owned()));
    if let Some(url) = base_url {
        provider_table.insert("base_url".to_owned(), toml::Value::String(url.to_owned()));
    }
    if let Some(env_name) = api_key_env {
        provider_table.insert(
            "api_key_env".to_owned(),
            toml::Value::String(env_name.to_owned()),
        );
    }
    let mut root = toml::map::Map::new();
    root.insert("provider".to_owned(), toml::Value::Table(provider_table));
    let mut security_table = toml::map::Map::new();
    security_table.insert("tui_paste_key_modal".to_owned(), toml::Value::Boolean(true));
    root.insert("security".to_owned(), toml::Value::Table(security_table));
    let mut telemetry_table = toml::map::Map::new();
    telemetry_table.insert("enabled".to_owned(), toml::Value::Boolean(false));
    root.insert("telemetry".to_owned(), toml::Value::Table(telemetry_table));
    let config_text = toml::to_string_pretty(&toml::Value::Table(root))?;
    fs::write(config_path, &config_text)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_owned())
}

fn provider_key_url(choice: ProviderChoice) -> Option<&'static str> {
    match choice {
        ProviderChoice::Groq => Some("https://console.groq.com/keys"),
        ProviderChoice::Anthropic => Some("https://console.anthropic.com/"),
        ProviderChoice::Gemini => Some("https://aistudio.google.com/app/apikey"),
        ProviderChoice::Openrouter => Some("https://openrouter.ai/keys"),
        ProviderChoice::NvidiaNim => Some("https://build.nvidia.com/"),
        _ => None,
    }
}

pub(crate) fn generate_agents_md_for_dir(dir: &Path) -> Result<Option<PathBuf>> {
    let target = dir.join("AGENTS.md");
    if target.exists() {
        return Ok(Some(target));
    }

    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !matches!(name.as_ref(), ".git" | "target" | "node_modules")
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    let has_project_shape = entries.iter().any(|entry| {
        matches!(
            entry.file_name().to_string_lossy().as_ref(),
            ".git"
                | "Cargo.toml"
                | "package.json"
                | "pyproject.toml"
                | "go.mod"
                | "src"
                | "crates"
                | "app"
                | "lib"
        )
    });
    if !has_project_shape {
        return Ok(None);
    }

    let mut body =
        String::from("# AGENTS.md\n\nProject context for HELM.\n\n## Top-level entries\n\n");
    for entry in entries.iter().take(32) {
        let path = entry.path();
        let kind = match entry.file_type() {
            Ok(ft) if ft.is_dir() => "dir",
            Ok(_) => "file",
            Err(_) => "entry",
        };
        body.push_str(&format!(
            "- `{}` ({kind})\n",
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".to_owned())
        ));
    }
    body.push_str(
        "\n## Notes\n\n- Add architecture, conventions, and workflow rules here.\n- HELM reads this file automatically at run start.\n",
    );

    fs::write(&target, body).with_context(|| format!("failed to write {}", target.display()))?;
    Ok(Some(target))
}

async fn interactive_init(
    config_path: &Path,
    db_path: &Path,
    force: bool,
    no_validate: bool,
    store: &SecretsStore,
) -> Result<()> {
    if config_path.exists() && !force {
        let answer = prompt(&format!(
            "Config already exists at {}. Overwrite? [y/N] ",
            config_path.display()
        ))?;
        if !answer.eq_ignore_ascii_case("y") {
            println!("Aborted. Use `helm init --force` to overwrite.");
            return Ok(());
        }
    }

    println!("\nHELM — setup wizard\n");
    println!("Choose your LLM provider:\n");
    println!("  [1] Groq         (free, fast)  llama-3.3-70b-versatile");
    println!("  [2] Anthropic    (Claude)       claude-3-5-haiku-20241022");
    println!("  [3] Gemini       (Google)       gemini-2.0-flash");
    println!("  [4] OpenRouter   (multi-model)  meta-llama/llama-3.3-70b-instruct");
    println!("  [5] NVIDIA NIM   (hosted)       meta/llama-3.3-70b-instruct");
    println!("  [6] Ollama       (local, free)  qwen3:4b");
    println!("  [7] OpenAI-compat (custom URL)");
    println!();

    let choice = loop {
        let answer = prompt("Enter number [1-7]: ")?;
        match answer.as_str() {
            "1" => break ProviderChoice::Groq,
            "2" => break ProviderChoice::Anthropic,
            "3" => break ProviderChoice::Gemini,
            "4" => break ProviderChoice::Openrouter,
            "5" => break ProviderChoice::NvidiaNim,
            "6" => break ProviderChoice::Ollama,
            "7" => break ProviderChoice::OpenaiCompat,
            _ => println!("  Enter a number 1-7."),
        }
    };

    let mut base_url: Option<String> = None;
    let mut secret_key: Option<Secret> = None;
    let mut secret_key_name: Option<&str> = None;

    match choice {
        ProviderChoice::Ollama => {
            let url = prompt("Ollama URL [http://localhost:11434]: ")?;
            if !url.is_empty() {
                base_url = Some(url);
            }
        }
        ProviderChoice::OpenaiCompat => {
            let url = prompt("Base URL (e.g. https://api.openai.com/v1): ")?;
            if !url.is_empty() {
                base_url = Some(url);
            }
            let key = rpassword::prompt_password("API key (masked, leave blank if not required): ")
                .map_err(|e| anyhow!("failed to read password: {e}"))?;
            if !key.is_empty() {
                secret_key = Some(Secret::new(key));
                secret_key_name = Some("OPENAI_API_KEY");
            }
        }
        _ => {
            let env_name = default_api_key_env(choice).unwrap_or("API_KEY");
            if let Some(url) = provider_key_url(choice) {
                println!("\nGet your API key at: {url}");
            }
            let key = rpassword::prompt_password(format!("Paste {env_name} (masked): "))
                .map_err(|e| anyhow!("failed to read password: {e}"))?;
            if key.is_empty() {
                println!("  (no key entered — you can set {env_name} in your shell later)");
            } else {
                secret_key = Some(Secret::new(key));
                secret_key_name = Some(env_name);
            }
        }
    }

    // Store the API key in secrets.toml (not in config.toml).
    if let (Some(key), Some(name)) = (&secret_key, secret_key_name) {
        store
            .set(name, key.clone())
            .map_err(|e| anyhow!("failed to store secret: {e}"))?;
        if !no_validate {
            print!("  validating key… ");
            let _ = io::stdout().flush();
            let dummy_settings = ProviderSettings {
                choice,
                base_url: base_url.clone(),
                model: None,
                api_key_env: Some(name.to_owned()),
                api_key: None,
                source: ProviderSource::Cli,
            };
            match build_provider(&dummy_settings, store) {
                Ok((provider, model)) => {
                    let req = helm_providers::ChatRequest {
                        model: model.clone(),
                        system: None,
                        messages: vec![helm_core::Message::user("Reply with one word.")],
                        tools: vec![],
                        max_tokens: 1,
                        temperature: 0.0,
                    };
                    match provider.chat(req).await {
                        Ok(_) => println!("ok"),
                        Err(e) => {
                            println!("failed ({e})");
                            let answer = prompt("  Save anyway? [y/N] ")?;
                            if !answer.eq_ignore_ascii_case("y") {
                                store.delete(name).map_err(|error| {
                                    anyhow!("failed to remove invalid stored key: {error}")
                                })?;
                                return Err(anyhow!("API key validation failed"));
                            }
                            eprintln!(
                                "  Key stored but validation failed. Run `helm doctor` after setup."
                            );
                        }
                    }
                }
                Err(e) => println!("(skipped — {e})"),
            }
        }
    }

    let default_model = default_model_name(choice);
    let model_input = prompt(&format!("\nModel [{}]: ", default_model))?;
    let model = if model_input.is_empty() {
        default_model.to_owned()
    } else {
        model_input
    };

    let telemetry_enabled =
        prompt("\nAllow anonymous crash reports? [y/N] ")?.eq_ignore_ascii_case("y");

    let kind = provider_choice_name(choice);
    // Write config without the plain-text key (key lives in secrets.toml).
    write_helm_config(
        config_path,
        db_path,
        kind,
        &model,
        base_url.as_deref(),
        default_api_key_env(choice),
    )?;
    if telemetry_enabled {
        let mut value: toml::Value = fs::read_to_string(config_path)?
            .parse()
            .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        set_toml_path(
            &mut value,
            &["telemetry", "enabled"],
            toml::Value::Boolean(true),
        )?;
        fs::write(config_path, toml::to_string_pretty(&value)?)?;
    }

    if let Ok(cwd) = env::current_dir() {
        let answer = prompt("Generate AGENTS.md for this project? [Y/n] ")?;
        if !answer.eq_ignore_ascii_case("n")
            && let Some(path) = generate_agents_md_for_dir(&cwd)?
        {
            println!("  agents   : wrote {}", path.display());
        }
    }

    println!("\nConfig written: {}", config_path.display());
    println!("  provider : {kind}");
    println!("  model    : {model}");
    if secret_key_name.is_some() {
        println!("  key      : stored in {}", store.path().display());
    }
    println!();
    println!("Next steps:");
    println!("  helm doctor       — verify everything is working");
    println!("  helm              — open the interactive terminal UI");
    println!("  helm \"<task>\"     — run an agent task");
    println!();

    Ok(())
}

fn format_audit_events(events: &[AuditEventRecord]) -> String {
    let mut output = String::from(
        "ID  DECISION CAPABILITY      TAINT              TOOL       TARGET           EPISODE\n",
    );
    for event in events {
        output.push_str(&format!(
            "{:<3} {:<8} {:<15} {:<18} {:<10} {:<16} {}\n",
            event.id,
            event.decision,
            event.capability,
            event.taint,
            event.tool_name,
            event.target.as_deref().unwrap_or("-"),
            event.episode_id.as_deref().unwrap_or("-")
        ));
    }
    output
}

fn parse_capability_arg(value: &str) -> Result<Capability> {
    value.parse::<Capability>().map_err(anyhow::Error::msg)
}

fn parse_scope_arg(value: &str) -> Result<GrantScope> {
    value.parse::<GrantScope>().map_err(anyhow::Error::msg)
}

fn truncate_goal(goal: &str) -> String {
    let mut chars = goal.chars();
    let head = chars.by_ref().take(28).collect::<String>();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

fn format_timestamp(unix_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(unix_ms)
        .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "invalid timestamp".to_owned())
}

async fn render_models(base_url: &str) -> Result<String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to reach Ollama at {base_url}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read Ollama response")?;
    if !status.is_success() {
        return Err(anyhow!("Ollama returned HTTP {}: {body}", status.as_u16()));
    }
    let tags: OllamaTagsResponse =
        serde_json::from_str(&body).context("failed to parse Ollama /api/tags response")?;
    Ok(format_models(&tags.models))
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    version: String,
    provider: DoctorProviderReport,
    memory: DoctorMemoryReport,
    tools: Vec<DoctorToolReport>,
    ollama: DoctorOllamaReport,
    other_providers_detected: Vec<DoctorEnvReport>,
    quirks: DoctorQuirksReport,
    secrets: DoctorSecretsReport,
}

#[derive(Debug, Serialize)]
struct DoctorSecretsReport {
    store_path: String,
    store_exists: bool,
    store_permissions_ok: bool,
    keys_stored: Vec<String>,
    env_overrides: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DoctorQuirksReport {
    expected_format: String,
    force_temperature: Option<f32>,
    system_prompt_addendum: bool,
    user_note: Option<String>,
}

#[derive(Debug, Serialize)]
struct DoctorProviderReport {
    resolved: String,
    source: String,
    model: String,
    reachable: DoctorCheck,
    tool_calls: DoctorCheck,
}

#[derive(Debug, Serialize)]
struct DoctorMemoryReport {
    database: String,
    schema_version: u32,
    episodes: u32,
    success: u32,
    partial: u32,
    failure: u32,
}

#[derive(Debug, Serialize)]
struct DoctorToolReport {
    name: String,
    status: String,
}

#[derive(Debug, Serialize)]
struct DoctorOllamaReport {
    reachable: DoctorCheck,
    base_url: String,
    installed_models: Vec<DoctorOllamaModel>,
    cloud_models: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DoctorOllamaModel {
    name: String,
    note: String,
    tools_capable: bool,
}

#[derive(Debug, Serialize)]
struct DoctorEnvReport {
    name: String,
    set: bool,
}

#[derive(Debug, Serialize)]
struct DoctorCheck {
    ok: bool,
    detail: String,
    latency_ms: Option<u128>,
}

async fn run_doctor(
    settings: &ProviderSettings,
    db_path: &Path,
    memory: &MemoryStore,
    store: &SecretsStore,
) -> Result<DoctorReport> {
    let provider_report = run_provider_doctor(settings, store).await;
    let memory_report = run_memory_doctor(db_path, memory).await?;
    let tools = run_tools_doctor();
    let ollama = run_ollama_doctor(
        &settings
            .base_url
            .clone()
            .unwrap_or_else(default_ollama_base_url),
    )
    .await;
    let q = quirks_for(
        provider_choice_name(settings.choice),
        provider_report.model.as_str(),
    );
    let quirks = DoctorQuirksReport {
        expected_format: format!("{:?}", q.expected_format),
        force_temperature: q.force_temperature,
        system_prompt_addendum: q.system_prompt_addendum.is_some(),
        user_note: q.user_note,
    };
    let secrets = run_secrets_doctor(store);
    Ok(DoctorReport {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        provider: provider_report,
        memory: memory_report,
        tools,
        ollama,
        other_providers_detected: provider_env_reports(),
        quirks,
        secrets,
    })
}

fn run_secrets_doctor(store: &SecretsStore) -> DoctorSecretsReport {
    let store_path = store.path().display().to_string();
    let store_exists = store.path().exists();
    let keys_stored = store.list_names().unwrap_or_default();
    let store_permissions_ok = if store_exists {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(store.path())
                .map(|m| m.permissions().mode() & 0o777 == 0o600)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            true
        }
    } else {
        true
    };
    let tracked_env_vars = [
        "ANTHROPIC_API_KEY",
        "GROQ_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "NVIDIA_API_KEY",
        "GOOGLE_API_KEY",
        "GEMINI_API_KEY",
    ];
    let env_overrides = tracked_env_vars
        .iter()
        .filter(|&&v| env::var(v).is_ok() && keys_stored.iter().any(|stored| stored == v))
        .map(|&v| v.to_owned())
        .collect();
    DoctorSecretsReport {
        store_path,
        store_exists,
        store_permissions_ok,
        keys_stored,
        env_overrides,
    }
}

async fn run_provider_doctor(
    settings: &ProviderSettings,
    store: &SecretsStore,
) -> DoctorProviderReport {
    let resolved = provider_choice_name(settings.choice).to_owned();
    let source = settings.source.human().to_owned();
    match build_provider(settings, store) {
        Ok((provider, model)) => {
            let (reachable, tool_calls) = probe_provider(provider.as_ref(), &model).await;
            DoctorProviderReport {
                resolved,
                source,
                model,
                reachable,
                tool_calls,
            }
        }
        Err(error) => DoctorProviderReport {
            resolved,
            source,
            model: settings
                .model
                .clone()
                .unwrap_or_else(|| default_model_name(settings.choice).to_owned()),
            reachable: DoctorCheck {
                ok: false,
                detail: error.to_string(),
                latency_ms: None,
            },
            tool_calls: DoctorCheck {
                ok: false,
                detail: "not probed because provider build failed".to_owned(),
                latency_ms: None,
            },
        },
    }
}

async fn probe_provider(provider: &dyn Provider, model: &str) -> (DoctorCheck, DoctorCheck) {
    let start = Instant::now();
    let reachable_response = provider
        .chat(ChatRequest {
            model: model.to_owned(),
            system: None,
            messages: vec![helm_core::Message::user("Reply with one short word.")],
            tools: Vec::new(),
            max_tokens: 1,
            temperature: 0.0,
        })
        .await;
    let latency_ms = start.elapsed().as_millis();
    match reachable_response {
        Ok(_) => {
            let reachable = DoctorCheck {
                ok: true,
                detail: "yes".to_owned(),
                latency_ms: Some(latency_ms),
            };
            let tool_calls = probe_tool_calls(provider, model).await;
            (reachable, tool_calls)
        }
        Err(error) => (
            DoctorCheck {
                ok: false,
                detail: error.to_string(),
                latency_ms: Some(latency_ms),
            },
            DoctorCheck {
                ok: false,
                detail: "not probed because reachability failed".to_owned(),
                latency_ms: None,
            },
        ),
    }
}

async fn probe_tool_calls(provider: &dyn Provider, model: &str) -> DoctorCheck {
    let start = Instant::now();
    let response = provider
        .chat(ChatRequest {
            model: model.to_owned(),
            system: None,
            messages: vec![helm_core::Message::user(
                "Call the noop tool with an empty object.",
            )],
            tools: vec![ToolSchema {
                name: "noop".to_owned(),
                description: "No-op diagnostic tool.".to_owned(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            }],
            max_tokens: 64,
            temperature: 0.0,
        })
        .await;
    let latency_ms = start.elapsed().as_millis();
    match response {
        Ok(response) if response_has_tool_use(&response) => DoctorCheck {
            ok: true,
            detail: "yes".to_owned(),
            latency_ms: Some(latency_ms),
        },
        Ok(_) => DoctorCheck {
            ok: false,
            detail: "no - model may not be tool-capable".to_owned(),
            latency_ms: Some(latency_ms),
        },
        Err(error) => DoctorCheck {
            ok: false,
            detail: error.to_string(),
            latency_ms: Some(latency_ms),
        },
    }
}

fn response_has_tool_use(response: &ChatResponse) -> bool {
    response.stop_reason == StopReason::ToolUse
        || response
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
}

async fn run_memory_doctor(db_path: &Path, memory: &MemoryStore) -> Result<DoctorMemoryReport> {
    let counts = memory.episode_outcome_counts().await?;
    Ok(DoctorMemoryReport {
        database: db_path.display().to_string(),
        schema_version: memory.schema_version().await?,
        episodes: counts.total,
        success: counts.success,
        partial: counts.partial,
        failure: counts.failure,
    })
}

/// Build a `ToolRegistry` seeded with all default tools plus one `SkillTool` per skill.
fn build_registry_with_skills(skills: &[helm_memory::Skill]) -> ToolRegistry {
    let mut registry = ToolRegistry::with_default_tools();
    for skill in skills {
        let cmds = builtin_skills::extract_bash_commands(&skill.content);
        if !cmds.is_empty() {
            registry.register(Box::new(SkillTool::new(
                &skill.id,
                &skill.description,
                cmds,
            )));
        }
    }
    registry
}

fn run_tools_doctor() -> Vec<DoctorToolReport> {
    let mut tools = build_registry_with_skills(&builtin_skills::load_builtin_skills())
        .schemas()
        .into_iter()
        .map(|schema| DoctorToolReport {
            name: schema.name,
            status: "ok".to_owned(),
        })
        .collect::<Vec<_>>();
    tools.sort_by_key(|tool| match tool.name.as_str() {
        "shell" => 0,
        "fs_read" => 1,
        "fs_write" => 2,
        _ => 3,
    });
    tools
}

async fn run_ollama_doctor(base_url: &str) -> DoctorOllamaReport {
    match fetch_ollama_models(base_url).await {
        Ok(models) => {
            let installed_models = models
                .iter()
                .map(|model| {
                    let families = model.details.families.clone().unwrap_or_default();
                    let tools_capable = supports_tools(&model.name, &families);
                    DoctorOllamaModel {
                        name: model.name.clone(),
                        note: model_note(&model.name, tools_capable).to_owned(),
                        tools_capable,
                    }
                })
                .collect::<Vec<_>>();
            let cloud_models = models
                .iter()
                .filter(|model| is_cloud_model(&model.name.to_ascii_lowercase()))
                .map(|model| model.name.clone())
                .collect();
            DoctorOllamaReport {
                reachable: DoctorCheck {
                    ok: true,
                    detail: "yes".to_owned(),
                    latency_ms: None,
                },
                base_url: base_url.to_owned(),
                installed_models,
                cloud_models,
            }
        }
        Err(error) => DoctorOllamaReport {
            reachable: DoctorCheck {
                ok: false,
                detail: error.to_string(),
                latency_ms: None,
            },
            base_url: base_url.to_owned(),
            installed_models: Vec::new(),
            cloud_models: Vec::new(),
        },
    }
}

async fn fetch_ollama_models(base_url: &str) -> Result<Vec<OllamaModel>> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to reach Ollama at {base_url}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read Ollama response")?;
    if !status.is_success() {
        return Err(anyhow!("Ollama returned HTTP {}: {body}", status.as_u16()));
    }
    let tags: OllamaTagsResponse =
        serde_json::from_str(&body).context("failed to parse Ollama /api/tags response")?;
    Ok(tags.models)
}

fn provider_env_reports() -> Vec<DoctorEnvReport> {
    [
        "GROQ_API_KEY",
        "GOOGLE_API_KEY",
        "GEMINI_API_KEY",
        "NVIDIA_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
    ]
    .iter()
    .map(|name| DoctorEnvReport {
        name: (*name).to_owned(),
        set: env::var_os(name).is_some(),
    })
    .collect()
}

fn render_doctor(report: &DoctorReport) -> String {
    let mut output = String::new();
    output.push_str(&format!("HELM v{} - system check\n\n", report.version));
    output.push_str("[provider]\n");
    output.push_str(&format!(
        "  resolved: {} ({})\n",
        report.provider.resolved, report.provider.source
    ));
    output.push_str(&format!("  model: {}\n", report.provider.model));
    output.push_str(&format!(
        "  reachable: {}\n",
        format_doctor_check(&report.provider.reachable)
    ));
    output.push_str(&format!(
        "  tool calls: {}\n\n",
        format_doctor_check(&report.provider.tool_calls)
    ));
    output.push_str("[memory]\n");
    output.push_str(&format!("  database: {}\n", report.memory.database));
    output.push_str(&format!(
        "  schema_version: {}\n",
        report.memory.schema_version
    ));
    output.push_str(&format!(
        "  episodes: {} ({} success, {} partial, {} failure)\n\n",
        report.memory.episodes, report.memory.success, report.memory.partial, report.memory.failure
    ));
    output.push_str("[tools]\n");
    for tool in &report.tools {
        output.push_str(&format!("  {}: {}\n", tool.name, tool.status));
    }
    output.push('\n');
    output.push_str("[ollama]\n");
    output.push_str(&format!(
        "  reachable: {} ({})\n",
        yes_no(report.ollama.reachable.ok),
        report.ollama.base_url
    ));
    if !report.ollama.reachable.ok {
        output.push_str(&format!("  error: {}\n", report.ollama.reachable.detail));
    }
    output.push_str(&format!(
        "  installed models: {}\n",
        format_doctor_ollama_models(&report.ollama.installed_models)
    ));
    output.push_str(&format!(
        "  cloud models: {}\n\n",
        format_list_or_none(&report.ollama.cloud_models)
    ));
    output.push_str("[other providers detected via env]\n");
    for env_report in &report.other_providers_detected {
        output.push_str(&format!(
            "  {}: {}\n",
            env_report.name,
            if env_report.set { "set" } else { "not set" }
        ));
    }
    output.push('\n');
    output.push_str("[quirks]\n");
    output.push_str(&format!(
        "  expected_format: {}\n",
        report.quirks.expected_format
    ));
    if let Some(t) = report.quirks.force_temperature {
        output.push_str(&format!("  force_temperature: {t}\n"));
    }
    if report.quirks.system_prompt_addendum {
        output.push_str("  system_prompt_addendum: yes\n");
    }
    if let Some(note) = &report.quirks.user_note {
        output.push_str(&format!("  note: {note}\n"));
    }
    output.push('\n');
    output.push_str("[secrets]\n");
    output.push_str(&format!("  store: {}", report.secrets.store_path));
    if report.secrets.store_exists {
        output.push_str(&format!(
            " ({})\n",
            if report.secrets.store_permissions_ok {
                "0600"
            } else {
                "INSECURE: not 0600"
            }
        ));
    } else {
        output.push_str(" (missing)\n");
    }
    if report.secrets.keys_stored.is_empty() {
        output.push_str("  keys present: none\n");
    } else {
        output.push_str(&format!(
            "  keys present: {}\n",
            report.secrets.keys_stored.join(", ")
        ));
    }
    if !report.secrets.env_overrides.is_empty() {
        output.push_str(&format!(
            "  keys also present via env: {}\n",
            report.secrets.env_overrides.join(", ")
        ));
        output.push_str("  warning: secrets store takes precedence over env fallback\n");
    }
    output
}

// ── TrustReport ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct TrustReport {
    version: String,
    provider: TrustProviderSummary,
    grants: TrustGrantsSummary,
    audit: TrustAuditSummary,
    sandbox: TrustSandboxSummary,
    secrets: TrustSecretsSummary,
    permissions: TrustPermissionsSummary,
    diagnose: TrustDiagnoseSummary,
}

#[derive(Debug, Serialize)]
struct TrustProviderSummary {
    /// Which provider is active (e.g. "groq", "anthropic", "ollama").
    kind: String,
    /// Whether this is a local provider (ollama) vs a remote API.
    is_local: bool,
    /// The base URL being used, if configured.
    base_url: Option<String>,
    /// The model name being used.
    model: Option<String>,
}

#[derive(Debug, Serialize)]
struct TrustGrantsSummary {
    active: usize,
    total: usize,
}

#[derive(Debug, Serialize)]
struct TrustAuditSummary {
    total_events: usize,
    chain_ok: bool,
}

#[derive(Debug, Serialize)]
struct TrustSandboxSummary {
    enabled: bool,
    bwrap_found: bool,
    bwrap_path: String,
    root_dir: Option<String>,
}

#[derive(Debug, Serialize)]
struct TrustSecretsSummary {
    /// Whether the secrets store exists and is usable.
    store_exists: bool,
    /// Number of stored secrets.
    stored: usize,
    /// Whether any keys are currently missing from the store.
    keys_missing: bool,
}

#[derive(Debug, Serialize)]
struct TrustPermissionsSummary {
    /// Which capabilities require explicit grants.
    restricted_by_default: Vec<String>,
    /// Which capabilities are auto-granted.
    auto_granted: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TrustDiagnoseSummary {
    tools_available: usize,
    write_tools_blocked: usize,
    /// Whether the diagnose registry blocks all write ops at runtime.
    write_ops_gated: bool,
}

async fn run_trust_report(
    memory: &std::sync::Arc<MemoryStore>,
    secrets: &SecretsStore,
    provider_settings: &ProviderSettings,
    args: &TrustReportArgs,
) -> Result<TrustReport> {
    let grants = memory.list_capability_grants().await?;
    let active = grants.iter().filter(|g| g.revoked_at.is_none()).count();

    let events = if let Some(target) = &args.target {
        memory.audit_events(None, Some(target)).await?
    } else {
        memory.audit_events(None, None).await?
    };
    let chain_ok = memory.verify_audit_chain().await?.ok;

    // Provider summary
    let is_local = matches!(provider_settings.choice, ProviderChoice::Ollama);
    let provider_kind = provider_choice_name(provider_settings.choice);

    // Sandbox
    let bwrap_path = which::which("bwrap").unwrap_or_default();
    let bwrap_found = !bwrap_path.as_os_str().is_empty();

    // Secrets
    let stored = secrets.list_names().map(|l| l.len()).unwrap_or(0);
    let env_key_name = default_api_key_env(provider_settings.choice);
    let keys_missing = match env_key_name {
        Some(name) if provider_settings.api_key.is_none() => {
            secrets.get(name).ok().flatten().is_none()
        }
        _ => false,
    };
    let store_exists = dirs::config_dir()
        .map(|p| p.join("helm").join("secrets.toml").exists())
        .unwrap_or(false);

    // Permissions
    let restricted_by_default: Vec<String> = Capability::all()
        .into_iter()
        .filter(|c| c.requires_grant_by_default())
        .map(|c| c.to_string())
        .collect();
    let auto_granted: Vec<String> = Capability::all()
        .into_iter()
        .filter(|c| !c.requires_grant_by_default())
        .map(|c| c.to_string())
        .collect();

    // Diagnose
    let diagnose_registry = ToolRegistry::with_diagnose_tools();
    let full_registry = ToolRegistry::with_default_tools();
    let tools_available = diagnose_registry.schemas().len();
    let write_tools_blocked = full_registry
        .schemas()
        .len()
        .saturating_sub(tools_available);
    // Verify every diagnose-registry tool gates its mutating sub-actions
    // at runtime (add/commit/push, post/put/delete/patch, kill, shell mode).
    let write_ops_gated = diagnose_registry.verify_diagnose_write_gates();

    Ok(TrustReport {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        provider: TrustProviderSummary {
            kind: provider_kind.to_string(),
            is_local,
            base_url: provider_settings.base_url.clone(),
            model: provider_settings.model.clone(),
        },
        grants: TrustGrantsSummary {
            active,
            total: grants.len(),
        },
        audit: TrustAuditSummary {
            total_events: events.len(),
            chain_ok,
        },
        sandbox: TrustSandboxSummary {
            enabled: false, // sandbox not enabled by default in trust-report
            bwrap_found,
            bwrap_path: bwrap_path.display().to_string(),
            root_dir: None,
        },
        secrets: TrustSecretsSummary {
            store_exists,
            stored,
            keys_missing,
        },
        permissions: TrustPermissionsSummary {
            restricted_by_default,
            auto_granted,
        },
        diagnose: TrustDiagnoseSummary {
            tools_available,
            write_tools_blocked,
            write_ops_gated,
        },
    })
}

fn render_trust_report(report: &TrustReport) -> String {
    let mut output = String::new();
    output.push_str(&format!("HELM v{} — Trust Report\n\n", report.version));

    output.push_str("[provider]\n");
    output.push_str(&format!("  kind: {}\n", report.provider.kind));
    output.push_str(&format!(
        "  boundary: {}\n",
        if report.provider.is_local {
            "local"
        } else {
            "remote API"
        }
    ));
    if let Some(url) = &report.provider.base_url {
        output.push_str(&format!("  base_url: {url}\n"));
    }
    if let Some(model) = &report.provider.model {
        output.push_str(&format!("  model: {model}\n"));
    }
    output.push('\n');

    output.push_str("[grants]\n");
    output.push_str(&format!(
        "  active: {} / total: {}\n\n",
        report.grants.active, report.grants.total
    ));

    output.push_str("[audit]\n");
    output.push_str(&format!("  events: {}\n", report.audit.total_events));
    output.push_str(&format!(
        "  chain: {}\n\n",
        if report.audit.chain_ok {
            "valid"
        } else {
            "INTEGRITY BREACH"
        }
    ));

    output.push_str("[sandbox]\n");
    output.push_str(&format!(
        "  enabled: {}\n",
        if report.sandbox.enabled { "yes" } else { "no" }
    ));
    output.push_str(&format!(
        "  bwrap: {}\n",
        if report.sandbox.bwrap_found {
            &report.sandbox.bwrap_path
        } else {
            "not found"
        }
    ));
    if let Some(root) = &report.sandbox.root_dir {
        output.push_str(&format!("  root_dir: {root}\n"));
    }
    output.push('\n');

    output.push_str("[secrets]\n");
    output.push_str(&format!(
        "  store: {}\n",
        if report.secrets.store_exists {
            "exists"
        } else {
            "missing"
        }
    ));
    output.push_str(&format!("  stored: {}\n", report.secrets.stored));
    output.push_str(&format!(
        "  keys_missing: {}\n\n",
        if report.secrets.keys_missing {
            "yes"
        } else {
            "no"
        }
    ));

    output.push_str("[permissions]\n");
    output.push_str(&format!(
        "  restricted: {}\n",
        report.permissions.restricted_by_default.join(", ")
    ));
    output.push_str(&format!(
        "  auto-granted: {}\n\n",
        report.permissions.auto_granted.join(", ")
    ));

    output.push_str("[diagnose mode]\n");
    output.push_str(&format!(
        "  read-only tools: {}\n",
        report.diagnose.tools_available
    ));
    output.push_str(&format!(
        "  write tools blocked: {}\n",
        report.diagnose.write_tools_blocked
    ));
    output.push_str(&format!(
        "  write sub-ops gated: {}\n",
        if report.diagnose.write_ops_gated {
            "yes"
        } else {
            "no"
        }
    ));
    output.push('\n');

    output.push_str("[paths]\n");
    output.push_str(&format!("  config: {}\n", paths::config_dir().display()));
    output.push_str(&format!("  data: {}\n", paths::data_dir().display()));
    output
}

fn format_doctor_check(check: &DoctorCheck) -> String {
    match (check.ok, check.latency_ms) {
        (true, Some(ms)) => format!("yes ({ms}ms)"),
        (true, None) => "yes".to_owned(),
        (false, _) => check.detail.clone(),
    }
}

fn format_doctor_ollama_models(models: &[DoctorOllamaModel]) -> String {
    if models.is_empty() {
        return "none (qwen3:4b not installed)".to_owned();
    }
    models
        .iter()
        .map(|model| format!("{} ({})", model.name, model.note))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_owned()
    } else {
        values.join(", ")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn format_models(models: &[OllamaModel]) -> String {
    let mut output = String::from("MODEL              SIZE     TOOL CALLS    NOTES\n");
    for model in models {
        let families = model.details.families.clone().unwrap_or_default();
        let support = supports_tools(&model.name, &families);
        let note = model_note(&model.name, support);
        output.push_str(&format!(
            "{:<18} {:<8} {:<13} {note}\n",
            model.name,
            format_size(model.size),
            if support { "yes" } else { "no" }
        ));
    }
    output
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    details: OllamaModelDetails,
}

#[derive(Debug, Default, Deserialize)]
struct OllamaModelDetails {
    #[serde(default)]
    families: Option<Vec<String>>,
}

fn format_size(bytes: u64) -> String {
    const GIB: f64 = 1_073_741_824.0;
    const MIB: f64 = 1_048_576.0;
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / GIB)
    } else {
        format!("{:.0} MB", bytes as f64 / MIB)
    }
}

// Tool-capable Ollama family allowlist reviewed on 2026-05-04. This will go stale.
fn supports_tools(model_name: &str, families: &[String]) -> bool {
    let name = model_name.to_ascii_lowercase();
    let family_text = families
        .iter()
        .map(|family| family.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    if name.contains("nomic-embed") || family_text.contains("bert") {
        return false;
    }
    if name.starts_with("gemma3") || family_text.contains("gemma3") {
        return false;
    }
    if is_cloud_model(&name) {
        return supported_cloud_family(&name, &family_text);
    }
    if name.starts_with("llama3.2") || family_text.contains("llama3.2") {
        return model_size_billions(&name).is_some_and(|size| size >= 3.0);
    }
    [
        "qwen3",
        "qwen2.5",
        "llama3.3",
        "mistral",
        "hermes4",
        "command-r",
    ]
    .iter()
    .any(|family| name.starts_with(family) || family_text.contains(family))
}

fn is_cloud_model(model_name: &str) -> bool {
    model_name
        .rsplit(':')
        .next()
        .is_some_and(|tag| tag == "cloud" || tag.ends_with("-cloud"))
}

fn supported_cloud_family(name: &str, family_text: &str) -> bool {
    [
        "qwen3",
        "qwen2.5",
        "llama3.3",
        "mistral",
        "hermes4",
        "command-r",
        "glm-5.1",
        "gemma4",
    ]
    .iter()
    .any(|family| name.starts_with(family) || family_text.contains(family))
}

fn model_note(model_name: &str, support: bool) -> &'static str {
    let name = model_name.to_ascii_lowercase();
    if support && name.starts_with("qwen3") {
        "recommended"
    } else if support && is_cloud_model(&name) {
        "cloud tools-capable"
    } else if name.starts_with("llama3.2")
        && model_size_billions(&name).is_some_and(|size| size < 3.0)
    {
        "too small for agent use"
    } else if name.contains("embed") {
        "embedding model"
    } else if name.starts_with("gemma3") {
        "no tool support in current Ollama gemma builds"
    } else if support {
        "tools-capable"
    } else {
        "unknown model family"
    }
}

fn model_size_billions(model_name: &str) -> Option<f32> {
    let marker = model_name.rsplit(':').next()?;
    let number = marker.strip_suffix('b')?;
    number.parse::<f32>().ok()
}

fn default_ollama_base_url() -> String {
    env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_owned())
}

fn build_provider(
    settings: &ProviderSettings,
    store: &SecretsStore,
) -> Result<(Box<dyn Provider>, String)> {
    match settings.choice {
        ProviderChoice::Groq => {
            let api_key = resolve_provider_key("GROQ_API_KEY", settings, store)?;
            let provider = OpenAiCompatProvider::groq(api_key)?;
            let model = settings
                .model
                .clone()
                .unwrap_or_else(|| provider.default_model().to_owned());
            Ok((Box::new(provider), model))
        }
        ProviderChoice::Anthropic => {
            let api_key = resolve_provider_key("ANTHROPIC_API_KEY", settings, store)?;
            let provider = AnthropicProvider::new(api_key)?;
            let model = settings
                .model
                .clone()
                .unwrap_or_else(|| AnthropicProvider::default_model().to_owned());
            Ok((Box::new(provider), model))
        }
        ProviderChoice::Ollama => {
            let provider = match settings.base_url.clone() {
                Some(url) => OllamaProvider::with_base_url(url)?,
                None => OllamaProvider::from_env()?,
            };
            let model = settings
                .model
                .clone()
                .unwrap_or_else(|| OllamaProvider::default_model().to_owned());
            Ok((Box::new(provider), model))
        }
        ProviderChoice::Gemini => {
            let env_name = settings.api_key_env.as_deref().unwrap_or("GOOGLE_API_KEY");
            let api_key = lookup_provider_key(env_name, settings, store)?
                .or(if env_name == "GOOGLE_API_KEY" {
                    lookup_provider_key("GEMINI_API_KEY", settings, store)?
                } else {
                    None
                })
                .ok_or_else(|| {
                    anyhow!("{env_name} is not set; run `helm secrets set {env_name}` to configure")
                })?;
            let provider = match settings.base_url.clone() {
                Some(url) => GeminiProvider::with_base_url(api_key, url)?,
                None => GeminiProvider::new(api_key)?,
            };
            let model = settings
                .model
                .clone()
                .unwrap_or_else(|| GeminiProvider::default_model().to_owned());
            Ok((Box::new(provider), model))
        }
        ProviderChoice::Openrouter => {
            let api_key = resolve_provider_key("OPENROUTER_API_KEY", settings, store)?;
            let provider = OpenAiCompatProvider::openrouter(api_key)?;
            let model = settings
                .model
                .clone()
                .unwrap_or_else(|| provider.default_model().to_owned());
            Ok((Box::new(provider), model))
        }
        ProviderChoice::NvidiaNim => {
            let api_key = resolve_provider_key("NVIDIA_API_KEY", settings, store)?;
            let provider = OpenAiCompatProvider::nvidia_nim(api_key)?;
            let model = settings
                .model
                .clone()
                .unwrap_or_else(|| provider.default_model().to_owned());
            Ok((Box::new(provider), model))
        }
        ProviderChoice::OpenaiCompat => {
            let api_key = match settings.api_key_env.as_deref() {
                Some(env_name) => Some(resolve_provider_key(env_name, settings, store)?),
                None => None,
            };
            let base_url = settings
                .base_url
                .clone()
                .ok_or_else(|| anyhow!("base_url is required for provider kind openai-compat"))?;
            let default_model = settings
                .model
                .clone()
                .unwrap_or_else(|| "gpt-4o-mini".to_owned());
            let mut builder = OpenAiCompatProvider::builder()
                .base_url(base_url)
                .default_model(default_model.clone())
                .label("openai-compat");
            if let Some(key) = api_key {
                builder = builder.api_key(key);
            }
            let provider = builder.build()?;
            Ok((Box::new(provider), default_model))
        }
        ProviderChoice::Auto => {
            let mut detected = settings.clone();
            apply_provider_defaults(&mut detected);
            build_provider(&detected, store)
        }
    }
}

fn resolve_provider_key(
    default_env: &str,
    settings: &ProviderSettings,
    store: &SecretsStore,
) -> Result<Secret> {
    let env_name = settings.api_key_env.as_deref().unwrap_or(default_env);
    lookup_provider_key(env_name, settings, store)?.ok_or_else(|| {
        anyhow!(
            "{env_name} is not set; run `helm secrets set {env_name}` or `helm init` to configure"
        )
    })
}

fn lookup_provider_key(
    env_name: &str,
    settings: &ProviderSettings,
    store: &SecretsStore,
) -> Result<Option<Secret>> {
    // settings.api_key acts as the CLI-level override (set via --api-key or TUI input)
    let cli_override = settings
        .api_key
        .as_ref()
        .map(|key| Secret::new(key.clone()));
    Ok(secrets::resolve_secret(
        env_name,
        cli_override.as_ref(),
        store,
    )?)
}

fn resolve_provider_choice(choice: ProviderChoice) -> ProviderChoice {
    choice
}

/// Parse a comma-separated fallback chain (e.g., "groq,anthropic,openrouter").
fn parse_fallback_chain(chain: &str) -> Vec<ProviderChoice> {
    chain
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .filter_map(|name| match name.as_str() {
            "groq" => Some(ProviderChoice::Groq),
            "anthropic" => Some(ProviderChoice::Anthropic),
            "ollama" => Some(ProviderChoice::Ollama),
            "gemini" => Some(ProviderChoice::Gemini),
            "openrouter" => Some(ProviderChoice::Openrouter),
            "nvidia-nim" | "nvidia" => Some(ProviderChoice::NvidiaNim),
            "openai-compat" | "openai" => Some(ProviderChoice::OpenaiCompat),
            "auto" => Some(ProviderChoice::Auto),
            _ => {
                eprintln!("warning: unknown provider in fallback chain: {}", name);
                None
            }
        })
        .collect()
}

impl ProviderSource {
    fn human(self) -> String {
        match self {
            Self::Cli => "from CLI".to_owned(),
            Self::HelmProviderEnv => "from $HELM_PROVIDER".to_owned(),
            Self::ConfigFile => "from config file".to_owned(),
            Self::EnvVar(name) => format!("from ${name}"),
            Self::Fallback => "fallback".to_owned(),
        }
    }
}

fn provider_choice_name(choice: ProviderChoice) -> &'static str {
    match choice {
        ProviderChoice::Auto => "auto",
        ProviderChoice::Groq => "groq",
        ProviderChoice::Anthropic => "anthropic",
        ProviderChoice::Ollama => "ollama",
        ProviderChoice::Gemini => "gemini",
        ProviderChoice::Openrouter => "openrouter",
        ProviderChoice::NvidiaNim => "nvidia-nim",
        ProviderChoice::OpenaiCompat => "openai-compat",
    }
}

pub(crate) fn default_model_name(choice: ProviderChoice) -> &'static str {
    match choice {
        ProviderChoice::Groq => "llama-3.3-70b-versatile",
        ProviderChoice::Anthropic => AnthropicProvider::default_model(),
        ProviderChoice::Ollama | ProviderChoice::Auto => "qwen3:4b",
        ProviderChoice::Gemini => GeminiProvider::default_model(),
        ProviderChoice::Openrouter => "meta-llama/llama-3.3-70b-instruct",
        ProviderChoice::NvidiaNim => "meta/llama-3.3-70b-instruct",
        ProviderChoice::OpenaiCompat => "gpt-4o-mini",
    }
}

pub(crate) fn default_api_key_env(choice: ProviderChoice) -> Option<&'static str> {
    match choice {
        ProviderChoice::Groq => Some("GROQ_API_KEY"),
        ProviderChoice::Anthropic => Some("ANTHROPIC_API_KEY"),
        ProviderChoice::Gemini => Some("GOOGLE_API_KEY"),
        ProviderChoice::Openrouter => Some("OPENROUTER_API_KEY"),
        ProviderChoice::NvidiaNim => Some("NVIDIA_API_KEY"),
        ProviderChoice::OpenaiCompat | ProviderChoice::Auto | ProviderChoice::Ollama => None,
    }
}

fn init_tracing(
    verbose: bool,
    _json: bool,
    log_path: Option<&Path>,
    telemetry: &telemetry::TelemetryConfig,
) -> Result<()> {
    let default_filter = if verbose { "helm=debug" } else { "helm=warn" };
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_filter))
        .map_err(|error| anyhow!("invalid tracing filter: {error}"))?;

    #[cfg(feature = "otel")]
    if let Some(tracer) = telemetry::build_otel_tracer(telemetry) {
        let otel = tracing_opentelemetry::layer().with_tracer(tracer);
        return match log_path {
            Some(path) => {
                ensure_parent_dir(path)?;
                tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt::layer().with_writer(LogFileMakeWriter {
                        path: path.to_path_buf(),
                    }))
                    .with(otel)
                    .try_init()
                    .map_err(|e| anyhow!("failed to initialize tracing: {e}"))
            }
            None => tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(std::io::stderr))
                .with(otel)
                .try_init()
                .map_err(|e| anyhow!("failed to initialize tracing: {e}")),
        };
    }

    #[cfg(not(feature = "otel"))]
    let _ = telemetry;

    match log_path {
        Some(path) => {
            ensure_parent_dir(path)?;
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(LogFileMakeWriter {
                    path: path.to_path_buf(),
                }))
                .try_init()
                .map_err(|error| anyhow!("failed to initialize tracing: {error}"))?;
        }
        None => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(std::io::stderr))
                .try_init()
                .map_err(|error| anyhow!("failed to initialize tracing: {error}"))?;
        }
    }
    Ok(())
}

struct LogFileMakeWriter {
    path: PathBuf,
}

impl<'a> MakeWriter<'a> for LogFileMakeWriter {
    type Writer = LogFileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok();
        LogFileWriter { file }
    }
}

struct LogFileWriter {
    file: Option<File>,
}

impl Write for LogFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.file {
            Some(file) => file.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.file {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

fn default_db_path() -> Result<PathBuf> {
    Ok(paths::default_db_path())
}

fn default_log_path() -> Result<PathBuf> {
    Ok(paths::default_log_path())
}

fn default_config_path() -> Result<PathBuf> {
    Ok(paths::default_config_path())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("database path must have a parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create database directory {}", parent.display()))?;
    Ok(())
}

fn classify_exit_code(error: &(dyn std::error::Error + 'static)) -> u8 {
    if let Some(HelmError::Provider(ProviderError::MissingConfig(_))) =
        error.downcast_ref::<HelmError>()
    {
        return 2;
    }
    if let Some(ProviderError::MissingConfig(_)) = error.downcast_ref::<ProviderError>() {
        return 2;
    }
    let text = error.to_string();
    if text.contains("_API_KEY")
        || text.contains("HELM_PROVIDER")
        || text.contains("database path")
        || text.contains("malformed config")
    {
        2
    } else {
        1
    }
}

async fn run_memory_command(args: MemoryArgs, _memory: &Arc<MemoryStore>) -> Result<()> {
    use helm_memory::EntityGraph;

    // Get or create graph at a standard location
    let graph_path = paths::default_graph_path();

    if let Some(parent) = graph_path.parent() {
        fs::create_dir_all(parent).context("failed to create graph directory")?;
    }

    let graph =
        EntityGraph::open(&graph_path).map_err(|e| anyhow!("failed to open graph: {}", e))?;

    match args.command {
        MemoryCommand::Graph { entity_type, name } => {
            let entities = graph
                .find_entities(entity_type.as_deref(), name.as_deref())
                .map_err(|e| anyhow!("graph error: {}", e))?;
            if entities.is_empty() {
                println!("No entities found.");
            } else {
                for entity in entities {
                    println!("{}: {} [{}]", entity.id, entity.name, entity.kind);
                }
            }
        }
        MemoryCommand::Export { output } => {
            let json = graph
                .export_json()
                .map_err(|e| anyhow!("export error: {}", e))?;
            fs::write(&output, json).context(format!("failed to write to {}", output))?;
            println!("Exported to: {}", output);
        }
        MemoryCommand::Import { input } => {
            let json = fs::read_to_string(&input).context(format!("failed to read {}", input))?;
            let (ents, rels) = graph
                .import_json(&json)
                .map_err(|e| anyhow!("import error: {}", e))?;
            println!("Imported {} entities and {} relations", ents, rels);
        }
        MemoryCommand::Gc {
            age_days,
            min_confidence,
        } => {
            let pruned = graph
                .prune_stale_relations(age_days, min_confidence)
                .map_err(|e| anyhow!("gc error: {}", e))?;
            println!("Pruned {} relations", pruned);
        }
    }
    Ok(())
}

async fn run_remote_command(args: RemoteArgs) -> Result<()> {
    use remote::{RemoteEntry, RemoteRegistry};
    let mut registry = RemoteRegistry::load()?;
    match args.command {
        RemoteCommand::Add(a) => {
            let entry = RemoteEntry {
                name: a.name.clone(),
                host: a.host,
                port: a.port,
                user: a.user,
                ssh_opts: a.ssh_opts,
            };
            registry.upsert(entry);
            registry.save()?;
            println!("added remote: {}", a.name);
        }
        RemoteCommand::List => {
            if registry.remotes.is_empty() {
                println!("no remotes registered. Use `helm remote add NAME --host ...`.");
            }
            for r in &registry.remotes {
                let user = r.user.as_deref().unwrap_or("(default user)");
                println!("{:<20} {}@{}:{}", r.name, user, r.host, r.port);
            }
        }
        RemoteCommand::Test { name } => {
            let entry = registry
                .get(&name)
                .ok_or_else(|| anyhow!("unknown remote: {name}"))?;
            match entry.ping().await {
                Ok(true) => println!("remote {name} reachable."),
                Ok(false) => println!("remote {name} unreachable (ssh exited non-zero)."),
                Err(error) => println!("remote {name} probe failed: {error}"),
            }
        }
        RemoteCommand::Remove { name } => {
            let removed = registry.remove(&name);
            registry.save()?;
            if removed {
                println!("removed remote: {name}");
            } else {
                println!("no remote named {name}");
            }
        }
    }
    Ok(())
}

async fn run_serve_command(
    args: ServeArgs,
    provider_settings: ProviderSettings,
    memory: Arc<MemoryStore>,
    max_iterations: Option<u32>,
    auto_approve: bool,
    read_only: bool,
    secrets: &SecretsStore,
) -> Result<()> {
    let token = match args.token {
        Some(t) => t,
        None => {
            let generated = serve::generate_token();
            eprintln!(
                "[helm serve] no --token supplied; generated one for this session:\n  {generated}"
            );
            generated
        }
    };
    serve::serve(serve::ServeConfig {
        bind: args.bind,
        token,
        provider_settings,
        memory,
        max_iterations,
        auto_approve,
        read_only,
        secrets: secrets.clone(),
    })
    .await
}

async fn run_attach_session(target: &str, token: &str) -> Result<()> {
    attach_tui::run_attach_tui(target.to_string(), token.to_string()).await
}

async fn run_profile_command(
    args: ProfileArgs,
    db_path: &Path,
    memory: &Arc<MemoryStore>,
) -> Result<()> {
    let profile_path = db_path
        .parent()
        .map(|p| p.join("profile.db"))
        .ok_or_else(|| anyhow!("invalid db path"))?;
    let prefs_toml = paths::user_profile_file();
    let profile = UserProfileStore::open_with_prefs(&profile_path, &prefs_toml)
        .map_err(|e| anyhow!("profile error: {}", e))?;

    match args.command {
        ProfileCommand::Show => {
            let prefs = profile.get().map_err(|e| anyhow!("profile error: {}", e))?;
            println!("User Preferences:");
            println!("  Preferred Model: {:?}", prefs.preferred_model);
            println!("  Verbosity: {:?}", prefs.verbosity);
            println!("  Timezone: {:?}", prefs.timezone);
            println!("  Corrections: {}", prefs.correction_count);
            println!("  Last Goal: {:?}", prefs.last_goal);
        }
        ProfileCommand::Set { key, value } => {
            profile
                .set_preference(&key, &value)
                .await
                .map_err(|e| anyhow!("profile error: {}", e))?;
            println!("Set {}: {}", key, value);
        }
        ProfileCommand::Get { key } => {
            let val = profile
                .get_preference(&key)
                .await
                .map_err(|e| anyhow!("profile error: {}", e))?;
            match val {
                Some(v) => println!("{}: {}", key, v),
                None => println!("{}: (not set)", key),
            }
        }
        ProfileCommand::Routes => {
            let stats = memory
                .routing_stats()
                .await
                .map_err(|e| anyhow!("routing stats: {}", e))?;
            if stats.is_empty() {
                println!("No routing outcomes recorded yet.");
                println!(
                    "Run agents with multiple providers (e.g. `helm run --fallback ...`) to populate routing data."
                );
            } else {
                println!(
                    "{:<32} {:>7} {:>7} {:>9} {:>10}",
                    "MODEL", "RUNS", "OK%", "AVG_MS", "COST_USD"
                );
                for s in stats {
                    let model_label: String = s.model.chars().take(32).collect();
                    println!(
                        "{:<32} {:>7} {:>6.1}% {:>9.0} {:>10.4}",
                        model_label,
                        s.total,
                        s.success_rate() * 100.0,
                        s.avg_latency_ms,
                        s.total_cost_usd,
                    );
                }
            }
        }
    }
    Ok(())
}

async fn run_collect_snapshot_command(args: SnapshotArgs) -> Result<()> {
    let profile: MonitorProfile = args.profile.parse().unwrap_or(MonitorProfile::Standard);
    let snapshot = collect_snapshot(profile).await;

    if args.diff {
        // Diff unavailable without persistence — collect and show
        eprintln!("note: --diff requires snapshot persistence (not yet wired to MemoryStore)");
        if args.json {
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        } else {
            render_snapshot(&snapshot);
        }
    } else if args.json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        render_snapshot(&snapshot);
    }

    Ok(())
}

fn render_snapshot(snapshot: &SystemSnapshot) {
    println!("Snapshot: {}", snapshot.id);
    println!(
        "  Host:       {} ({} {} {})",
        snapshot.host.hostname,
        snapshot.host.kernel_name,
        snapshot.host.kernel_release,
        snapshot.host.machine
    );
    if let Some(os) = &snapshot.host.os_pretty_name {
        println!("  OS:         {os}");
    }
    println!("  Uptime:     {}s", snapshot.host.uptime_seconds);
    println!("  Profile:    {:?}", snapshot.profile);
    println!("  Collected:  {}", snapshot.collected_at);

    println!("\nLoad:");
    println!(
        "  Load avg:     {:.2} {:.2} {:.2}",
        snapshot.domains.load.load_average.one,
        snapshot.domains.load.load_average.five,
        snapshot.domains.load.load_average.fifteen
    );
    println!(
        "  CPU cores:    {}",
        snapshot.domains.load.cpu_logical_count
    );
    println!(
        "  Memory:       {} / {} ({} available)",
        human_bytes(snapshot.domains.load.memory.used),
        human_bytes(snapshot.domains.load.memory.total),
        snapshot
            .domains
            .load
            .memory
            .available
            .map_or_else(|| "?".to_string(), human_bytes)
    );
    println!(
        "  Swap:         {} / {}",
        human_bytes(snapshot.domains.load.swap_used),
        human_bytes(snapshot.domains.load.swap_total)
    );

    println!("\nDisks:");
    for fs in &snapshot.domains.disks.filesystems {
        let pct = if fs.total_bytes > 0 {
            (fs.used_bytes as f64 / fs.total_bytes as f64 * 100.0) as u64
        } else {
            0
        };
        println!(
            "  {} {}: {}/{} ({}%)",
            fs.device,
            fs.mount_point,
            human_bytes(fs.used_bytes),
            human_bytes(fs.total_bytes),
            pct
        );
    }
    for inode in &snapshot.domains.disks.inodes {
        if inode.total > 0 {
            let pct = (inode.used as f64 / inode.total as f64 * 100.0) as u64;
            println!(
                "  inodes {}: {}/{} ({}%)",
                inode.mount_point, inode.used, inode.total, pct
            );
        }
    }
    if snapshot.domains.disks.smart_available {
        println!("  SMART: available");
    }
    for d in &snapshot.domains.disks.smart_devices {
        println!(
            "  {} health: {}",
            d.device,
            d.health.as_deref().unwrap_or("unknown")
        );
    }

    println!("\nServices:");
    let failed = snapshot.domains.services.failed_units.len();
    if failed > 0 {
        println!("  FAILED: {failed} units");
        for u in &snapshot.domains.services.failed_units {
            println!("    {} ({})", u.name, u.description);
        }
    }
    println!(
        "  {} loaded units, {} timers",
        snapshot.domains.services.units.len(),
        snapshot.domains.services.timers.len()
    );

    if let Some(rt) = &snapshot.domains.containers.runtime {
        println!(
            "\nContainers ({}): {}",
            rt,
            snapshot.domains.containers.containers.len()
        );
        for c in &snapshot.domains.containers.containers {
            println!("  {} ({}) - {}", c.name, c.image, c.status);
        }
    }

    println!(
        "\nPorts: {} listeners",
        snapshot.domains.ports.listeners.len()
    );
    for l in &snapshot.domains.ports.listeners {
        println!(
            "  {}:{} ({})",
            l.local_address,
            l.local_port,
            l.process_name.as_deref().unwrap_or("?")
        );
    }

    println!("\nNetwork:");
    println!("  Routes: {}", snapshot.domains.network.routes.len());
    println!(
        "  Interfaces: {}",
        snapshot.domains.network.interfaces.len()
    );
    println!("  Nameservers: {:?}", snapshot.domains.network.nameservers);

    println!("\nLogs:");
    println!(
        "  Journal errors (1h): {}",
        snapshot.domains.logs.journal_errors_last_hour
    );
    if let Some(rate) = snapshot.domains.logs.error_rate_per_minute {
        println!("  Error rate: {rate:.1}/min");
    }

    if !snapshot.domains.backups.tools_detected.is_empty() {
        println!(
            "\nBackups: {} tools detected",
            snapshot.domains.backups.tools_detected.len()
        );
        for t in &snapshot.domains.backups.tools_detected {
            println!("  {}", t.name);
        }
    }

    if let Some(pm) = &snapshot.domains.packages.package_manager {
        println!("\nPackages ({pm}):");
        if let Some(c) = snapshot.domains.packages.upgradable_count {
            println!("  Upgradable: {c}");
        }
        if let Some(c) = snapshot.domains.packages.security_count {
            println!("  Security: {c}");
        }
    }

    println!(
        "\nTimers: {} cron jobs",
        snapshot.domains.timers.cron_jobs.len()
    );

    if !snapshot.collector_errors.is_empty() {
        println!("\nCollector errors:");
        for e in &snapshot.collector_errors {
            println!("  {}: {}", e.domain, e.message);
        }
    }
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}G", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}M", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc};

    use helm_agent::{Budget, ReactAgent};
    use helm_core::ContentBlock;
    use helm_memory::{EpisodeOutcome, MemoryStore, StepRole};
    use helm_providers::{ChatResponse, MockProvider, StopReason, Usage};
    use helm_tools::{ToolContext, ToolRegistry};
    use tempfile::tempdir;

    use super::{
        DoctorCheck, DoctorEnvReport, DoctorMemoryReport, DoctorOllamaModel, DoctorOllamaReport,
        DoctorProviderReport, DoctorQuirksReport, DoctorReport, DoctorSecretsReport,
        DoctorToolReport, ProviderChoice, ProviderSource, classify_exit_code, format_audit_events,
        format_models, format_permissions, load_config, model_capability_warning_text,
        parse_capability_arg, parse_cli_from, parse_fallback_chain, parse_scope_arg, render_doctor,
        render_replay, render_run_stdout, resolve_provider_choice,
        resolve_provider_settings_with_env, supports_tools,
    };

    fn empty_env(_name: &str) -> Option<String> {
        None
    }

    #[test]
    fn resolve_choice_happy_path() {
        assert!(matches!(
            resolve_provider_choice(ProviderChoice::Ollama),
            ProviderChoice::Ollama
        ));
    }

    #[test]
    fn classify_runtime_error_path() {
        let error = anyhow::anyhow!("runtime failed");

        assert_eq!(classify_exit_code(error.as_ref()), 1);
    }

    #[test]
    fn classify_config_edge_case() {
        let error = anyhow::anyhow!("ANTHROPIC_API_KEY is required");

        assert_eq!(classify_exit_code(error.as_ref()), 2);
    }

    #[test]
    fn fallback_chain_parses_known_providers_and_skips_unknowns() {
        let parsed = parse_fallback_chain("groq, unknown, openrouter, nvidia");

        assert_eq!(
            parsed,
            vec![
                ProviderChoice::Groq,
                ProviderChoice::Openrouter,
                ProviderChoice::NvidiaNim
            ]
        );
    }

    #[test]
    fn missing_config_uses_defaults() {
        let dir = tempdir().unwrap();
        let config = load_config(&dir.path().join("missing.toml")).unwrap();
        let settings =
            resolve_provider_settings_with_env(config.as_ref(), None, None, None, None, empty_env)
                .unwrap();

        assert_eq!(settings.choice, ProviderChoice::Ollama);
        assert_eq!(settings.model, None);
        assert_eq!(settings.base_url, Some("http://localhost:11434".to_owned()));
        assert_eq!(settings.source, ProviderSource::Fallback);
    }

    #[test]
    fn malformed_config_reports_line_number() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[provider]\nkind = [").unwrap();

        let error = load_config(&path).unwrap_err();

        assert!(error.to_string().contains("line 2"));
    }

    #[test]
    fn valid_config_applies_provider_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[provider]\nkind = \"ollama\"\nbase_url = \"http://localhost:11434\"\nmodel = \"qwen3:4b\"\n",
        )
        .unwrap();

        let config = load_config(&path).unwrap();
        let settings =
            resolve_provider_settings_with_env(config.as_ref(), None, None, None, None, empty_env)
                .unwrap();

        assert_eq!(settings.choice, ProviderChoice::Ollama);
        assert_eq!(settings.base_url, Some("http://localhost:11434".to_owned()));
        assert_eq!(settings.model, Some("qwen3:4b".to_owned()));
        assert_eq!(settings.source, ProviderSource::ConfigFile);
    }

    #[test]
    fn cli_flags_override_config() {
        let config = super::FileConfig {
            provider: Some(super::FileProviderConfig {
                kind: Some(ProviderChoice::Ollama),
                base_url: Some("http://config:11434".to_owned()),
                model: Some("qwen3:4b".to_owned()),
                api_key_env: None,
            }),
            security: None,
            telemetry: None,
        };

        let settings = resolve_provider_settings_with_env(
            Some(&config),
            Some(ProviderChoice::Anthropic),
            Some("http://flag:11434".to_owned()),
            Some("claude".to_owned()),
            Some("flag-key".to_owned()),
            empty_env,
        )
        .unwrap();

        assert_eq!(settings.choice, ProviderChoice::Anthropic);
        assert_eq!(settings.base_url, Some("http://flag:11434".to_owned()));
        assert_eq!(settings.model, Some("claude".to_owned()));
        assert_eq!(settings.api_key, Some("flag-key".to_owned()));
        assert_eq!(settings.source, ProviderSource::Cli);
    }

    #[test]
    fn cli_provider_overrides_helm_provider_env() {
        let settings = resolve_provider_settings_with_env(
            None,
            Some(ProviderChoice::Ollama),
            None,
            None,
            None,
            |name| (name == "HELM_PROVIDER").then(|| "groq".to_owned()),
        )
        .unwrap();

        assert_eq!(settings.choice, ProviderChoice::Ollama);
        assert_eq!(settings.source, ProviderSource::Cli);
    }

    #[test]
    fn env_provider_overrides_config() {
        let config = super::FileConfig {
            provider: Some(super::FileProviderConfig {
                kind: Some(ProviderChoice::Ollama),
                base_url: None,
                model: None,
                api_key_env: None,
            }),
            security: None,
            telemetry: None,
        };
        let settings =
            resolve_provider_settings_with_env(Some(&config), None, None, None, None, |name| {
                (name == "HELM_PROVIDER").then(|| "groq".to_owned())
            })
            .unwrap();

        assert_eq!(settings.choice, ProviderChoice::Groq);
        assert_eq!(settings.source, ProviderSource::HelmProviderEnv);
    }

    #[test]
    fn helm_model_env_overrides_config_model() {
        let config = super::FileConfig {
            provider: Some(super::FileProviderConfig {
                kind: Some(ProviderChoice::Groq),
                base_url: None,
                model: Some("config-model".to_owned()),
                api_key_env: None,
            }),
            security: None,
            telemetry: None,
        };
        let settings =
            resolve_provider_settings_with_env(Some(&config), None, None, None, None, |name| {
                (name == "HELM_MODEL").then(|| "env-model".to_owned())
            })
            .unwrap();

        assert_eq!(settings.model, Some("env-model".to_owned()));
    }

    #[test]
    fn config_provider_overrides_auto_detect_env() {
        let config = super::FileConfig {
            provider: Some(super::FileProviderConfig {
                kind: Some(ProviderChoice::Ollama),
                base_url: None,
                model: None,
                api_key_env: None,
            }),
            security: None,
            telemetry: None,
        };
        let settings =
            resolve_provider_settings_with_env(Some(&config), None, None, None, None, |name| {
                (name == "GROQ_API_KEY").then(|| "set".to_owned())
            })
            .unwrap();

        assert_eq!(settings.choice, ProviderChoice::Ollama);
        assert_eq!(settings.source, ProviderSource::ConfigFile);
    }

    #[test]
    fn auto_detect_env_precedence() {
        let settings = resolve_provider_settings_with_env(None, None, None, None, None, |name| {
            matches!(name, "GROQ_API_KEY" | "ANTHROPIC_API_KEY").then(|| "set".to_owned())
        })
        .unwrap();

        assert_eq!(settings.choice, ProviderChoice::Groq);
        assert_eq!(settings.api_key_env, Some("GROQ_API_KEY".to_owned()));
        assert_eq!(settings.source, ProviderSource::EnvVar("GROQ_API_KEY"));
    }

    #[test]
    fn auto_detect_openai_sets_base_url() {
        let settings = resolve_provider_settings_with_env(None, None, None, None, None, |name| {
            (name == "OPENAI_API_KEY").then(|| "set".to_owned())
        })
        .unwrap();

        assert_eq!(settings.choice, ProviderChoice::OpenaiCompat);
        assert_eq!(
            settings.base_url,
            Some("https://api.openai.com/v1".to_owned())
        );
    }

    #[test]
    fn auto_detect_covers_each_env_var() {
        for (env_name, expected) in [
            ("GROQ_API_KEY", ProviderChoice::Groq),
            ("ANTHROPIC_API_KEY", ProviderChoice::Anthropic),
            ("OPENAI_API_KEY", ProviderChoice::OpenaiCompat),
            ("OPENROUTER_API_KEY", ProviderChoice::Openrouter),
            ("NVIDIA_API_KEY", ProviderChoice::NvidiaNim),
            ("GOOGLE_API_KEY", ProviderChoice::Gemini),
            ("GEMINI_API_KEY", ProviderChoice::Gemini),
        ] {
            let settings =
                resolve_provider_settings_with_env(None, None, None, None, None, |name| {
                    (name == env_name).then(|| "set".to_owned())
                })
                .unwrap();

            assert_eq!(settings.choice, expected, "env {env_name}");
        }
    }

    #[test]
    fn default_run_and_run_subcommand_parse_same_task() {
        let default = parse_cli_from(["helm", "do thing"]).unwrap();
        let explicit = parse_cli_from(["helm", "run", "do thing"]).unwrap();

        match (default.command, explicit.command) {
            (super::Command::Run(left), super::Command::Run(right)) => {
                assert_eq!(left.task, right.task);
            }
            _ => panic!("expected run commands"),
        }
    }

    #[test]
    fn no_args_opens_tui() {
        let parsed = parse_cli_from(["helm"]).unwrap();
        assert!(matches!(parsed.command, super::Command::Tui(_)));
    }

    #[test]
    fn permissions_and_audit_subcommands_parse_happy_path() {
        let grant = parse_cli_from([
            "helm",
            "permissions",
            "grant",
            "shell.shell",
            "--scope",
            "once",
        ])
        .unwrap();
        let audit = parse_cli_from(["helm", "audit", "show", "--episode", "ep1"]).unwrap();

        assert!(matches!(
            grant.command,
            super::Command::Permissions(super::PermissionsArgs {
                command: super::PermissionsCommand::Grant(_)
            })
        ));
        assert!(matches!(
            audit.command,
            super::Command::Audit(super::AuditArgs {
                command: super::AuditCommand::Show(_)
            })
        ));

        let verify = parse_cli_from(["helm", "audit", "verify", "--target", "prod-1"]).unwrap();
        assert!(matches!(
            verify.command,
            super::Command::Audit(super::AuditArgs {
                command: super::AuditCommand::Verify(_)
            })
        ));
    }

    #[test]
    fn secrets_subcommands_parse_happy_path() {
        for args in [
            vec!["helm", "secrets", "list"],
            vec!["helm", "secrets", "set", "GROQ_API_KEY"],
            vec!["helm", "secrets", "set", "GROQ_API_KEY", "--from-stdin"],
            vec![
                "helm",
                "secrets",
                "set",
                "GROQ_API_KEY",
                "--value",
                "gsk_test",
            ],
            vec!["helm", "secrets", "get", "GROQ_API_KEY"],
            vec!["helm", "secrets", "delete", "GROQ_API_KEY"],
            vec!["helm", "secrets", "path"],
            vec!["helm", "secrets", "import-env"],
        ] {
            let parsed = parse_cli_from(args).unwrap();
            assert!(matches!(parsed.command, super::Command::Secrets(_)));
        }
    }

    #[test]
    fn config_subcommands_parse_happy_path() {
        for args in [
            vec!["helm", "config", "get", "provider.kind"],
            vec!["helm", "config", "set", "provider.model", "qwen3:4b"],
            vec!["helm", "config", "edit"],
            vec!["helm", "config", "validate"],
            vec!["helm", "config", "path"],
        ] {
            let parsed = parse_cli_from(args).unwrap();
            assert!(matches!(parsed.command, super::Command::Config(_)));
        }
    }

    #[test]
    fn completion_subcommand_parses_all_shells() {
        for shell in ["bash", "zsh", "fish"] {
            let parsed = parse_cli_from(["helm", "completion", shell]).unwrap();
            assert!(matches!(parsed.command, super::Command::Completion(_)));
        }
    }

    #[test]
    fn mcp_subcommands_parse_happy_path() {
        for args in [
            vec!["helm", "mcp", "list"],
            vec!["helm", "mcp", "add", "gmail", "node", "server.js"],
            vec!["helm", "mcp", "remove", "gmail"],
            vec!["helm", "mcp", "test", "gmail"],
            vec![
                "helm",
                "mcp",
                "run",
                "gmail",
                "draft_reply",
                "--arguments",
                "{\"id\":\"1\"}",
            ],
        ] {
            let parsed = parse_cli_from(args).unwrap();
            assert!(matches!(parsed.command, super::Command::Mcp(_)));
        }
    }

    #[test]
    fn global_yes_and_read_only_flags_parse() {
        let parsed = parse_cli_from(["helm", "--yes", "--read-only", "run", "list files"]).unwrap();
        assert!(parsed.yes);
        assert!(parsed.read_only);
    }

    #[test]
    fn continue_flag_parses_with_bare_task() {
        let parsed = parse_cli_from(["helm", "--continue", "investigate nginx"]).unwrap();
        assert!(parsed.continue_last);
        assert!(matches!(parsed.command, super::Command::Run(_)));
    }

    #[test]
    fn resume_flag_parses_with_specific_session_id() {
        let parsed =
            parse_cli_from(["helm", "--resume", "sess-123", "run", "continue work"]).unwrap();
        assert_eq!(parsed.resume.as_deref(), Some("sess-123"));
        assert!(matches!(parsed.command, super::Command::Run(_)));
    }

    #[test]
    fn redo_command_parses() {
        let parsed =
            parse_cli_from(["helm", "redo", "--session-id", "sess-123", "--apply"]).unwrap();
        assert!(matches!(parsed.command, super::Command::Redo(_)));
    }

    #[test]
    fn diagnose_command_parses() {
        let parsed = parse_cli_from(["helm", "diagnose", "why is disk usage spiking"]).unwrap();
        match parsed.command {
            super::Command::Diagnose(args) => {
                assert_eq!(args.question, "why is disk usage spiking");
            }
            other => panic!("expected diagnose command, got {other:?}"),
        }
    }

    #[test]
    fn trust_report_command_parses() {
        let parsed = parse_cli_from(["helm", "trust-report", "--json"]).unwrap();
        match parsed.command {
            super::Command::TrustReport(args) => {
                assert!(args.json);
            }
            other => panic!("expected trust-report command, got {other:?}"),
        }
    }

    #[test]
    fn derive_session_name_prefers_goal_words() {
        assert_eq!(
            super::derive_session_name("Find why nginx leaks memory and patch it"),
            "find-why-nginx-leaks-memory-and"
        );
        assert!(super::derive_session_name("!!!").starts_with("run-"));
    }

    #[test]
    fn init_subcommand_writes_config_happy_path() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let db_path = dir.path().join("helm.db");

        super::write_helm_config(
            &config_path,
            &db_path,
            "groq",
            "llama-3.3-70b-versatile",
            None,
            Some("GROQ_API_KEY"),
        )
        .unwrap();
        let config = fs::read_to_string(&config_path).unwrap();

        assert!(config.contains("kind = \"groq\""));
        assert!(config.contains("api_key_env = \"GROQ_API_KEY\""));
        assert!(!config.contains("api_key ="));
        assert!(config.contains("[security]"));
        assert!(config.contains("[telemetry]"));
    }

    #[test]
    fn init_writes_ollama_config_no_key() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let db_path = dir.path().join("helm.db");

        super::write_helm_config(
            &config_path,
            &db_path,
            "ollama",
            "qwen3:4b",
            Some("http://localhost:11434"),
            None,
        )
        .unwrap();
        let config = fs::read_to_string(&config_path).unwrap();

        assert!(config.contains("kind = \"ollama\""));
        assert!(config.contains("base_url"));
        assert!(!config.contains("api_key"));
    }

    #[test]
    fn generates_agents_file_for_project_dir() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"demo\"\n").unwrap();

        let path = super::generate_agents_md_for_dir(dir.path()).unwrap();

        assert_eq!(path, Some(dir.path().join("AGENTS.md")));
        let body = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(body.contains("Top-level entries"));
        assert!(body.contains("Cargo.toml"));
    }

    #[test]
    fn permission_arg_parsing_rejects_unknown_error_path() {
        assert!(parse_capability_arg("shell.shell").is_ok());
        assert!(parse_scope_arg("15m").is_ok());
        assert!(parse_capability_arg("bad").is_err());
        assert!(parse_scope_arg("forever").is_err());
    }

    #[test]
    fn permission_and_audit_formatters_handle_empty_edge_case() {
        assert!(format_permissions(&[]).contains("CAPABILITY"));
        assert!(format_audit_events(&[]).contains("DECISION"));
    }

    #[test]
    fn unknown_subcommand_errors_clearly() {
        let error = parse_cli_from(["helm", "badcmd", "arg"]).unwrap_err();

        assert!(error.to_string().contains("unknown subcommand: badcmd"));
    }

    async fn run_with_mock(response: ChatResponse) -> helm_agent::RunResult {
        run_with_mocks(vec![response]).await
    }

    async fn run_with_mocks(responses: Vec<ChatResponse>) -> helm_agent::RunResult {
        let dir = tempdir().unwrap();
        let memory = Arc::new(
            MemoryStore::open(&dir.path().join("helm.db"))
                .await
                .unwrap(),
        );
        let agent = ReactAgent::with_tool_context(
            Box::new(MockProvider::new(responses)),
            ToolRegistry::default(),
            memory,
            Budget::default(),
            "mock",
            ToolContext::new(dir.path().to_path_buf()),
        );
        agent.run("task").await.unwrap()
    }

    fn response(content: Vec<ContentBlock>, stop_reason: StopReason) -> ChatResponse {
        ChatResponse {
            id: "msg".to_owned(),
            content,
            stop_reason,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        }
    }

    #[tokio::test]
    async fn stdout_prints_final_message_happy_path() {
        let result = run_with_mock(response(
            vec![ContentBlock::Text("done".to_owned())],
            StopReason::EndTurn,
        ))
        .await;

        assert_eq!(render_run_stdout(&result), "done\n");
    }

    #[tokio::test]
    async fn stdout_prints_last_assistant_message_error_path() {
        let result = run_with_mock(response(
            vec![ContentBlock::Text("partial text".to_owned())],
            StopReason::MaxTokens,
        ))
        .await;
        let mut result = result;
        result.final_message.clear();

        assert_eq!(
            render_run_stdout(&result),
            "[last assistant message]\npartial text\n"
        );
    }

    #[tokio::test]
    async fn stdout_prints_no_text_edge_case() {
        let result = run_with_mock(response(Vec::new(), StopReason::MaxTokens)).await;
        let mut result = result;
        result.final_message.clear();

        assert!(render_run_stdout(&result).contains("no assistant text was produced"));
    }

    #[tokio::test]
    async fn warning_text_is_available_for_tool_shaped_text() {
        // Provider emits a bare-JSON tool call as text; format recovery executes it
        // and then needs a second response to finish the run.
        let result = run_with_mocks(vec![
            response(
                vec![ContentBlock::Text(
                    r#"{"name":"shell","parameters":{"command":"echo","args":["hi"]}}"#.to_owned(),
                )],
                StopReason::EndTurn,
            ),
            response(
                vec![ContentBlock::Text("done".to_owned())],
                StopReason::EndTurn,
            ),
        ])
        .await;

        assert!(result.format_recovery_used);
        assert!(model_capability_warning_text().contains("qwen3:4b"));
    }

    #[tokio::test]
    async fn replay_prints_transcript_with_warning() {
        let dir = tempdir().unwrap();
        let store = MemoryStore::open(&dir.path().join("helm.db"))
            .await
            .unwrap();
        let id = store
            .start_episode("echo hello to /tmp/test.txt")
            .await
            .unwrap();
        store
            .log_step(
                &id,
                0,
                StepRole::User,
                &[ContentBlock::Text("echo hello to /tmp/test.txt".to_owned())],
                0,
                0,
            )
            .await
            .unwrap();
        store
            .log_step(
                &id,
                1,
                StepRole::Assistant,
                &[ContentBlock::ToolUse {
                    id: "toolu_1".to_owned(),
                    name: "shell".to_owned(),
                    input: serde_json::json!({"command":"echo"}),
                }],
                4,
                2,
            )
            .await
            .unwrap();
        store
            .log_step(
                &id,
                2,
                StepRole::Tool,
                &[ContentBlock::ToolResult {
                    tool_use_id: "toolu_1".to_owned(),
                    content: "exit 0\nhello world".to_owned(),
                    is_error: false,
                }],
                0,
                0,
            )
            .await
            .unwrap();
        store
            .set_model_capability_warning(&id, "model emitted tool-shaped text")
            .await
            .unwrap();
        store
            .finish_episode(&id, EpisodeOutcome::Partial, Some("partial"), Some("warn"))
            .await
            .unwrap();

        let transcript = render_replay(&store, &id).await.unwrap();

        assert!(transcript.contains("goal: echo hello"));
        assert!(transcript.contains("outcome: partial"));
        assert!(transcript.contains("warning: model_capability_warning"));
        assert!(transcript.contains("[step 2] tool (shell)"));
        assert!(transcript.contains("hello world"));
    }

    #[tokio::test]
    async fn replay_missing_episode_errors() {
        let dir = tempdir().unwrap();
        let store = MemoryStore::open(&dir.path().join("helm.db"))
            .await
            .unwrap();

        let error = render_replay(&store, "missing").await.unwrap_err();

        assert!(error.to_string().contains("episode not found"));
    }

    #[test]
    fn supports_tools_covers_known_cases() {
        assert!(!supports_tools("llama3.2:1b", &["llama3.2".to_owned()]));
        assert!(supports_tools("llama3.2:3b", &["llama3.2".to_owned()]));
        assert!(supports_tools("qwen3:4b", &["qwen3".to_owned()]));
        assert!(!supports_tools("nomic-embed-text", &["bert".to_owned()]));
        assert!(!supports_tools("gemma3:4b", &["gemma3".to_owned()]));
        assert!(supports_tools("glm-5.1:cloud", &[]));
        assert!(supports_tools("gemma4:31b-cloud", &[]));
        assert!(!supports_tools("unknown:7b", &[]));
    }

    #[test]
    fn format_models_reports_notes_edge_case() {
        let output = format_models(&[super::OllamaModel {
            name: "llama3.2:1b".to_owned(),
            size: 1_300_000_000,
            details: super::OllamaModelDetails {
                families: Some(vec!["llama3.2".to_owned()]),
            },
        }]);

        assert!(output.contains("too small for agent use"));
    }

    #[test]
    fn doctor_human_output_renders_each_section() {
        let report = DoctorReport {
            version: "0.1.2".to_owned(),
            provider: DoctorProviderReport {
                resolved: "groq".to_owned(),
                source: "from $GROQ_API_KEY".to_owned(),
                model: "llama-3.1-8b-instant".to_owned(),
                reachable: DoctorCheck {
                    ok: true,
                    detail: "yes".to_owned(),
                    latency_ms: Some(94),
                },
                tool_calls: DoctorCheck {
                    ok: true,
                    detail: "yes".to_owned(),
                    latency_ms: Some(21),
                },
            },
            memory: DoctorMemoryReport {
                database: "/tmp/helm.db".to_owned(),
                schema_version: 2,
                episodes: 14,
                success: 12,
                partial: 1,
                failure: 1,
            },
            tools: vec![
                DoctorToolReport {
                    name: "shell".to_owned(),
                    status: "ok".to_owned(),
                },
                DoctorToolReport {
                    name: "fs_read".to_owned(),
                    status: "ok".to_owned(),
                },
            ],
            ollama: DoctorOllamaReport {
                reachable: DoctorCheck {
                    ok: true,
                    detail: "yes".to_owned(),
                    latency_ms: None,
                },
                base_url: "http://localhost:11434".to_owned(),
                installed_models: vec![DoctorOllamaModel {
                    name: "qwen3:4b".to_owned(),
                    note: "recommended".to_owned(),
                    tools_capable: true,
                }],
                cloud_models: vec!["glm-5.1:cloud".to_owned()],
            },
            other_providers_detected: vec![DoctorEnvReport {
                name: "GROQ_API_KEY".to_owned(),
                set: true,
            }],
            quirks: DoctorQuirksReport {
                expected_format: "Native".to_owned(),
                force_temperature: Some(0.0),
                system_prompt_addendum: false,
                user_note: Some("Groq open-weight models require temperature=0.".to_owned()),
            },
            secrets: DoctorSecretsReport {
                store_path: "/home/user/.helm/secrets.toml".to_owned(),
                store_exists: true,
                store_permissions_ok: true,
                keys_stored: vec!["GROQ_API_KEY".to_owned()],
                env_overrides: Vec::new(),
            },
        };

        let output = render_doctor(&report);

        assert!(output.contains("[provider]"));
        assert!(output.contains("resolved: groq"));
        assert!(output.contains("[memory]"));
        assert!(output.contains("episodes: 14"));
        assert!(output.contains("[tools]"));
        assert!(output.contains("[ollama]"));
        assert!(output.contains("[other providers detected via env]"));
    }

    #[test]
    fn doctor_json_output_is_machine_readable() {
        let report = DoctorReport {
            version: "0.1.2".to_owned(),
            provider: DoctorProviderReport {
                resolved: "ollama".to_owned(),
                source: "fallback".to_owned(),
                model: "qwen3:4b".to_owned(),
                reachable: DoctorCheck {
                    ok: false,
                    detail: "offline".to_owned(),
                    latency_ms: None,
                },
                tool_calls: DoctorCheck {
                    ok: false,
                    detail: "not probed".to_owned(),
                    latency_ms: None,
                },
            },
            memory: DoctorMemoryReport {
                database: "/tmp/helm.db".to_owned(),
                schema_version: 2,
                episodes: 0,
                success: 0,
                partial: 0,
                failure: 0,
            },
            tools: Vec::new(),
            ollama: DoctorOllamaReport {
                reachable: DoctorCheck {
                    ok: false,
                    detail: "offline".to_owned(),
                    latency_ms: None,
                },
                base_url: "http://localhost:11434".to_owned(),
                installed_models: Vec::new(),
                cloud_models: Vec::new(),
            },
            other_providers_detected: Vec::new(),
            quirks: DoctorQuirksReport {
                expected_format: "BareJson".to_owned(),
                force_temperature: Some(0.0),
                system_prompt_addendum: true,
                user_note: Some("Ollama bare-JSON tool calls.".to_owned()),
            },
            secrets: DoctorSecretsReport {
                store_path: "/home/user/.helm/secrets.toml".to_owned(),
                store_exists: false,
                store_permissions_ok: false,
                keys_stored: Vec::new(),
                env_overrides: Vec::new(),
            },
        };

        let value = serde_json::to_value(&report).unwrap();

        assert_eq!(value["provider"]["resolved"], "ollama");
        assert_eq!(value["memory"]["schema_version"], 2);
    }

    #[test]
    fn models_command_reads_mocked_ollama_tags() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let maybe_server = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.block_on(async { mockito::Server::new_async().await })
        }));
        let Ok(mut server) = maybe_server else {
            eprintln!("skipping mockito-backed models test: mock server unavailable");
            return;
        };

        runtime.block_on(async {
            let mock = server
                .mock("GET", "/api/tags")
                .with_status(200)
                .with_body(
                    serde_json::json!({
                        "models": [{
                            "name": "qwen3:4b",
                            "size": 2400000000_u64,
                            "details": {"families": ["qwen3"]}
                        }]
                    })
                    .to_string(),
                )
                .create_async()
                .await;

            let output = super::render_models(&server.url()).await.unwrap();

            assert!(output.contains("qwen3:4b"));
            assert!(output.contains("recommended"));
            mock.assert_async().await;
        });
    }

    #[tokio::test]
    async fn doctor_probe_uses_mock_provider_for_reachable_and_tools() {
        let provider = MockProvider::new(vec![
            response(
                vec![ContentBlock::Text("ok".to_owned())],
                StopReason::EndTurn,
            ),
            response(
                vec![ContentBlock::ToolUse {
                    id: "noop_1".to_owned(),
                    name: "noop".to_owned(),
                    input: serde_json::json!({}),
                }],
                StopReason::ToolUse,
            ),
        ]);

        let (reachable, tool_calls) = super::probe_provider(&provider, "mock").await;

        assert!(reachable.ok);
        assert!(tool_calls.ok);
    }

    // ── v1.6 integration tests ──

    #[test]
    fn trust_report_struct_contains_all_v16_fields() {
        // Verify the TrustReport struct serializes all documented fields.
        let report = super::TrustReport {
            version: "1.6.0".into(),
            provider: super::TrustProviderSummary {
                kind: "groq".into(),
                is_local: false,
                base_url: None,
                model: Some("llama-3.3-70b-versatile".into()),
            },
            grants: super::TrustGrantsSummary {
                active: 2,
                total: 5,
            },
            audit: super::TrustAuditSummary {
                total_events: 10,
                chain_ok: true,
            },
            sandbox: super::TrustSandboxSummary {
                enabled: false,
                bwrap_found: true,
                bwrap_path: "/usr/bin/bwrap".into(),
                root_dir: None,
            },
            secrets: super::TrustSecretsSummary {
                store_exists: true,
                stored: 3,
                keys_missing: false,
            },
            permissions: super::TrustPermissionsSummary {
                restricted_by_default: vec!["fs.delete".into(), "shell.exec".into()],
                auto_granted: vec!["fs.read".into(), "fs.write".into()],
            },
            diagnose: super::TrustDiagnoseSummary {
                tools_available: 9,
                write_tools_blocked: 8,
                write_ops_gated: true,
            },
        };

        let rendered = super::render_trust_report(&report);
        assert!(rendered.contains("HELM v1.6.0"));
        assert!(rendered.contains("[provider]"));
        assert!(rendered.contains("boundary: remote API"));
        assert!(rendered.contains("[secrets]"));
        assert!(rendered.contains("store: exists"));
        assert!(rendered.contains("stored: 3"));
        assert!(rendered.contains("[permissions]"));
        assert!(rendered.contains("restricted: fs.delete, shell.exec"));
        assert!(rendered.contains("write sub-ops gated: yes"));

        // JSON roundtrip
        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("\"kind\": \"groq\""));
        assert!(json.contains("\"is_local\": false"));
        assert!(json.contains("\"store_exists\": true"));
        assert!(json.contains("\"write_ops_gated\": true"));
    }

    #[test]
    fn trust_report_local_provider_shows_local_boundary() {
        let report = super::TrustReport {
            version: "1.6.0".into(),
            provider: super::TrustProviderSummary {
                kind: "ollama".into(),
                is_local: true,
                base_url: Some("http://localhost:11434".into()),
                model: Some("qwen3:4b".into()),
            },
            grants: super::TrustGrantsSummary {
                active: 0,
                total: 0,
            },
            audit: super::TrustAuditSummary {
                total_events: 0,
                chain_ok: true,
            },
            sandbox: super::TrustSandboxSummary {
                enabled: false,
                bwrap_found: false,
                bwrap_path: String::new(),
                root_dir: None,
            },
            secrets: super::TrustSecretsSummary {
                store_exists: true,
                stored: 0,
                keys_missing: true,
            },
            permissions: super::TrustPermissionsSummary {
                restricted_by_default: vec![],
                auto_granted: vec![],
            },
            diagnose: super::TrustDiagnoseSummary {
                tools_available: 9,
                write_tools_blocked: 8,
                write_ops_gated: true,
            },
        };

        let rendered = super::render_trust_report(&report);
        assert!(rendered.contains("boundary: local"));
        assert!(rendered.contains("kind: ollama"));
        assert!(rendered.contains("base_url: http://localhost:11434"));
        assert!(rendered.contains("keys_missing: yes"));
    }

    #[test]
    fn diagnose_registry_excludes_write_tools() {
        let diagnose = ToolRegistry::with_diagnose_tools();
        let full = ToolRegistry::with_default_tools();

        let diagnose_names: Vec<_> = diagnose.names();
        let full_names: Vec<_> = full.names();

        // Diagnose must NOT include write tools.
        for write_tool in ["fs_write", "service", "package", "browser"] {
            assert!(
                !diagnose_names.contains(&write_tool.to_string()),
                "diagnose registry must not contain {write_tool}"
            );
        }

        // Diagnose must be a proper subset.
        assert!(diagnose_names.len() < full_names.len());

        // Diagnose-relevant read-only tools must be present.
        for read_tool in ["fs_read", "disk", "network", "logs", "search", "process"] {
            assert!(
                diagnose_names.contains(&read_tool.to_string()),
                "diagnose registry must contain {read_tool}"
            );
        }
    }

    #[test]
    fn dry_run_cli_flag_is_parsed() {
        let parsed = parse_cli_from([
            "helm",
            "run",
            "--dry-run",
            "--read-only",
            "check disk space",
        ])
        .unwrap();
        assert!(parsed.dry_run);
        assert!(parsed.read_only);
        match parsed.command {
            super::Command::Run(args) => {
                assert_eq!(args.task, "check disk space");
            }
            other => panic!("expected run command, got {other:?}"),
        }
    }

    #[test]
    fn evidence_flag_is_parsed() {
        let parsed = parse_cli_from(["helm", "run", "--evidence", "analyze logs"]).unwrap();
        assert!(parsed.evidence);
        match parsed.command {
            super::Command::Run(args) => {
                assert_eq!(args.task, "analyze logs");
            }
            other => panic!("expected run command, got {other:?}"),
        }
    }

    #[test]
    fn structured_evidence_contains_all_required_fields() {
        use helm_agent::{
            BlastRadius, Finding, ProposedAction, RollbackStatus, StructuredEvidence,
            ToolCallPreview, Uncertainty,
        };
        let ev = StructuredEvidence {
            inspected_sources: vec!["df -h".into(), "free -h".into()],
            findings: vec![Finding {
                label: "disk usage".into(),
                value: "85%".into(),
                source: "df -h".into(),
            }],
            assumptions: vec!["system is idle".into()],
            uncertainty: Uncertainty::Medium,
            proposed_actions: vec![ProposedAction {
                description: "check large files".into(),
                tool: "disk".into(),
                tool_input: r#"{"action":"largest_files","path":"/"}"#.into(),
            }],
            blast_radius: BlastRadius {
                paths: vec!["/var/log".into()],
                services: vec!["nginx".into()],
                hosts: vec![],
            },
            rollback: RollbackStatus {
                available: true,
                description: "rm -f /var/log/big.log".into(),
            },
            exact_tool_calls: vec![ToolCallPreview {
                tool: "disk".into(),
                tool_input: r#"{"action":"largest_files","path":"/"}"#.into(),
                summary: "find largest files on /".into(),
            }],
        };

        // Serialize roundtrip
        let json = serde_json::to_string_pretty(&ev).unwrap();
        assert!(json.contains("inspected_sources"));
        assert!(json.contains("df -h"));
        assert!(json.contains("findings"));
        assert!(json.contains("assumptions"));
        assert!(json.contains("uncertainty"));
        assert!(json.contains("proposed_actions"));
        assert!(json.contains("blast_radius"));
        assert!(json.contains("rollback"));
        assert!(json.contains("exact_tool_calls"));
        assert!(json.contains("largest_files"));

        // Deserialize roundtrip
        let ev2: StructuredEvidence = serde_json::from_str(&json).unwrap();
        assert_eq!(ev.inspected_sources, ev2.inspected_sources);
        assert_eq!(ev.findings.len(), ev2.findings.len());
        assert_eq!(ev.uncertainty, ev2.uncertainty);
    }
}
