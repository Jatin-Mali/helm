//! Command-line entry point for HELM.

mod secrets;
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
    AuditEventRecord, CapabilityGrantRecord, EpisodeRecord, MemoryStore, StepRecord,
    UserProfileStore,
};
use helm_providers::{
    AnthropicProvider, ChatRequest, ChatResponse, GeminiProvider, OllamaProvider,
    OpenAiCompatProvider, Provider, StopReason, ToolSchema, quirks_for,
};
use helm_tools::{Tool, ToolContext, ToolRegistry};
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
    /// Auto-approve all tool permission requests (dangerous!)
    #[arg(
        long = "yes",
        alias = "yolo",
        alias = "dangerously-skip-permissions",
        global = true
    )]
    yes: bool,
    /// Plan mode: read-only analysis, no writes or executions
    #[arg(long = "read-only", alias = "plan", global = true)]
    read_only: bool,
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
    Tui,
    /// Manage sessions (list/delete/export/resume)
    Sessions(SessionsArgs),
    /// Export episode to file
    Export(ExportArgs),
    /// Manage file snapshots and undo/redo
    Undo(UndoArgs),
    /// Show cost and usage statistics
    Stats(StatsArgs),
    /// Manage knowledge graph and memory
    Memory(MemoryArgs),
    /// Manage user profile and preferences
    Profile(ProfileArgs),
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(value_name = "TASK")]
    task: String,
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
}

#[derive(Debug, Args)]
struct StatsArgs {
    #[arg(long)]
    since: Option<String>,
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
    Verify,
    Show(AuditShowArgs),
}

#[derive(Debug, Args)]
struct AuditShowArgs {
    #[arg(long, value_name = "EPISODE_ID")]
    episode: Option<String>,
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
    #[arg(long, help = "Overwrite an existing ~/.helm/config.toml")]
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

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, ValueEnum)]
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

async fn run() -> Result<()> {
    let cli = parse_cli_from(env::args_os())?;
    let tui_log_path = if matches!(cli.command, Command::Tui) {
        Some(default_log_path()?)
    } else {
        None
    };
    init_tracing(cli.verbose, tui_log_path.as_deref())?;
    let config_path = default_config_path()?;
    let config = load_config(&config_path)?;
    let _telemetry_enabled = config
        .as_ref()
        .and_then(|config| config.telemetry.as_ref())
        .and_then(|telemetry| telemetry.enabled)
        .unwrap_or(false);
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
            let agent = ReactAgent::new(provider, ToolRegistry::default(), memory, budget, model)?
                .with_cancel_token(cancel.child());
            let signal_cancel = cancel.child();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                signal_cancel.cancel();
            });
            let result = agent.run_with_events(&args.task, &CliProgressSink).await?;
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
            AuditCommand::Verify => {
                let verification = memory.verify_audit_chain().await?;
                if verification.ok {
                    println!("audit ok: checked {} event(s)", verification.checked);
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
                let report = render_audit_events(&memory, args.episode.as_deref()).await?;
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
            SessionsCommand::Resume { id } => {
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
                let session = store
                    .get_session(&id)
                    .await?
                    .ok_or_else(|| anyhow!("session not found: {}", id))?;
                println!(
                    "resuming session: {}\ngoal: {}\nepisode: {}",
                    session.name, session.goal, session.episode_id
                );
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
            let sessions_dir = dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                .join("helm");
            let db_path = cli
                .db_path
                .clone()
                .unwrap_or_else(|| sessions_dir.join("helm.db"));
            let store =
                helm_memory::SessionStore::open(&db_path, sessions_dir.join("snapshots")).await?;
            if let Some(sid) = args.session_id {
                let snapshots = store.list_snapshots(&sid).await?;
                if snapshots.is_empty() {
                    println!("no snapshots for session {}", sid);
                } else {
                    let target = snapshots.get((args.n as usize - 1).min(snapshots.len() - 1));
                    if let Some(snap) = target {
                        let content = store.restore_snapshot(&snap.id).await?;
                        println!(
                            "snapshot {} (step {}):\n{}",
                            snap.id, snap.step_index, content
                        );
                    }
                }
            } else {
                println!("use --session-id to specify a session for undo");
            }
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
                let manager = helm_memory::SkillsManager::new();
                let skills = manager.list()?;
                for skill in skills {
                    println!("{}: {} (v{})", skill.id, skill.name, skill.version);
                }
            }
            SkillsCommand::Show(args) => {
                let manager = helm_memory::SkillsManager::new();
                let skill = manager.show(&args.id)?;
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
        Command::Tui => {
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
            })
            .await?;
        }
        Command::Memory(args) => {
            run_memory_command(args, &memory).await?;
        }
        Command::Profile(args) => {
            run_profile_command(args, &db_path).await?;
        }
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
                | "--ollama-url",
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

async fn render_replay(memory: &MemoryStore, episode_id: &str) -> Result<String> {
    let episode = memory
        .get_episode(episode_id)
        .await?
        .ok_or_else(|| anyhow!("episode not found: {episode_id}"))?;
    let steps = memory.get_steps(episode_id).await?;
    Ok(format_transcript(&episode, &steps))
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

async fn render_audit_events(memory: &MemoryStore, episode: Option<&str>) -> Result<String> {
    let events = memory.audit_events(episode).await?;
    Ok(format_audit_events(&events))
}

fn set_toml_path(root: &mut toml::Value, parts: &[&str], value: toml::Value) -> Result<()> {
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
    let mut output =
        String::from("ID  DECISION CAPABILITY      TAINT              TOOL       EPISODE\n");
    for event in events {
        output.push_str(&format!(
            "{:<3} {:<8} {:<15} {:<18} {:<10} {}\n",
            event.id,
            event.decision,
            event.capability,
            event.taint,
            event.tool_name,
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

fn run_tools_doctor() -> Vec<DoctorToolReport> {
    let mut tools = ToolRegistry::default()
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

fn init_tracing(verbose: bool, log_path: Option<&Path>) -> Result<()> {
    let default_filter = if verbose { "helm=debug" } else { "helm=warn" };
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_filter))
        .map_err(|error| anyhow!("invalid tracing filter: {error}"))?;
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
    let home = env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".helm").join("helm.db"))
}

fn default_log_path() -> Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".helm").join("helm.log"))
}

fn default_config_path() -> Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".helm").join("config.toml"))
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
    let graph_path = dirs::data_local_dir()
        .ok_or_else(|| anyhow!("could not determine data directory"))?
        .join("helm")
        .join("graph.db");

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

async fn run_profile_command(args: ProfileArgs, db_path: &Path) -> Result<()> {
    let profile_path = db_path.parent().map(|p| p.join("profile.db")).ok_or_else(|| anyhow!("invalid db path"))?;
    let profile = UserProfileStore::open(&profile_path).map_err(|e| anyhow!("profile error: {}", e))?;

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
            println!("Model routing success rates not yet implemented");
        }
    }
    Ok(())
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
        parse_capability_arg, parse_cli_from, parse_scope_arg, render_doctor, render_replay,
        render_run_stdout, resolve_provider_choice, resolve_provider_settings_with_env,
        supports_tools,
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
        assert!(matches!(parsed.command, super::Command::Tui));
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

    #[tokio::test]
    async fn models_command_reads_mocked_ollama_tags() {
        let mut server = mockito::Server::new_async().await;
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
}
