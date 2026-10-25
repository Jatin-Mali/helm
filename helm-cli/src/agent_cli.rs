//! `helm run <task>`: the general agent over the shared runtime, with persisted sessions.
use crate::agent_session::{self, LogEntry, SessionRecord};
use crate::{ProviderSettings, SecretsStore, build_provider};
use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use helm_agent::{
    AgentConfig, AgentEvent, AgentOutcome, AgentRun, AgentRuntime, HostTools, Pricing, ToolCall,
};
use helm_monitor::{ExecutionPolicy, process_runner::CancellationToken};
use std::{
    io::{BufRead, IsTerminal, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Set while an agent run owns Ctrl-C so the process-wide handler lets it cancel cleanly once.
pub(crate) static OWNS_INTERRUPT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    /// What the agent should do. With --resume, an optional follow-up instruction.
    pub task: Option<String>,
    /// Directory the agent may read and (with approval) write. Defaults to the current directory.
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
    /// Continue a saved session. Unfinished tool calls are not replayed; they must be re-approved.
    #[arg(long, value_name = "ID")]
    pub resume: Option<String>,
    /// List saved sessions and exit.
    #[arg(long)]
    pub sessions: bool,
    #[arg(long, default_value_t = 20)]
    pub max_steps: u32,
    /// Wall-clock limit for the whole run.
    #[arg(long, default_value_t = 600)]
    pub timeout_secs: u64,
    /// Hard spend limit; requires --input-price and --output-price (USD per million tokens).
    #[arg(long)]
    pub budget_usd: Option<f64>,
    #[arg(long)]
    pub input_price: Option<f64>,
    #[arg(long)]
    pub output_price: Option<f64>,
    /// Emit one JSON object per event and a final session record on stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(Default)]
pub(crate) struct Observed {
    pub log: Vec<LogEntry>,
    pub approval: Option<(ToolCall, String)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Output {
    Human,
    Json,
    /// Record only; the caller (the TUI) renders events itself.
    Quiet,
}

fn brief(text: &str, limit: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > limit {
        format!("{}…", line.chars().take(limit).collect::<String>())
    } else {
        line.to_owned()
    }
}

pub(crate) fn observe(event: &AgentEvent, observed: &Mutex<Observed>, output: Output) {
    let json = output == Output::Json;
    let human = output == Output::Human;
    if json {
        if let Ok(line) = serde_json::to_string(event) {
            println!("{line}");
        }
    }
    let Ok(mut observed) = observed.lock() else {
        return;
    };
    match event {
        AgentEvent::Request { step } if human => eprintln!("· step {step}"),
        AgentEvent::Text { content } if human => eprintln!("{}", brief(content, 200)),
        AgentEvent::ToolCall { call } => {
            if human {
                eprintln!("→ {} {}", call.name, brief(&call.input.to_string(), 120));
            }
            observed.log.push(LogEntry::ToolCall {
                call_id: call.id.clone(),
                name: call.name.clone(),
                input: agent_session::redact_value(&call.input),
            });
        }
        AgentEvent::ToolResult { id, output } => {
            if human {
                let state = if output.is_error { "error" } else { "ok" };
                eprintln!("← {state}: {}", brief(&output.content, 120));
            }
            observed.log.push(LogEntry::ToolResult {
                call_id: id.clone(),
                content: helm_core::redact_secrets(&output.content),
                is_error: output.is_error,
            });
        }
        AgentEvent::ApprovalRequired { call, description } => {
            observed.approval = Some((call.clone(), description.clone()));
        }
        _ => {}
    }
}

fn ask(description: &str) -> bool {
    eprintln!("\nApproval required:\n{description}");
    eprint!("Approve this exact action once? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

fn config_from(args: &RunArgs, model: &str) -> Result<AgentConfig> {
    let pricing = match (args.input_price, args.output_price) {
        (Some(input_per_million), Some(output_per_million)) => Some(Pricing {
            input_per_million,
            output_per_million,
        }),
        (None, None) => None,
        _ => bail!("--input-price and --output-price must be given together"),
    };
    if args.budget_usd.is_some() && pricing.is_none() {
        bail!("--budget-usd needs --input-price and --output-price; HELM will not guess costs");
    }
    Ok(AgentConfig {
        model: model.to_owned(),
        max_steps: args.max_steps,
        wall_time: Duration::from_secs(args.timeout_secs),
        budget_usd: args.budget_usd,
        pricing,
        ..Default::default()
    })
}

pub(crate) struct Setup {
    pub runtime: Arc<AgentRuntime>,
    pub tools: Arc<HostTools>,
    pub model: String,
}

/// Builds the runtime shared by `helm run` and the TUI: provider, host tools over `workspace`,
/// the durable journal, and protected HELM state.
pub(crate) fn setup(
    settings: &ProviderSettings,
    secrets: &SecretsStore,
    db_path: &std::path::Path,
    policy: ExecutionPolicy,
    workspace: &str,
    config: impl FnOnce(&str) -> Result<AgentConfig>,
) -> Result<Setup> {
    let (provider, model) = build_provider(settings, secrets)?;
    let journal = Arc::new(crate::execution_journal::DurableJournal::new(
        db_path,
        secrets.clone(),
    )?);
    let tools = Arc::new(
        HostTools::new(workspace, policy, Some(journal))
            .map_err(|e| anyhow!("workspace unavailable: {e}"))?,
    );
    for path in [
        db_path.to_path_buf(),
        crate::paths::default_secrets_path(),
        crate::paths::default_config_path(),
        agent_session::sessions_dir(),
    ] {
        tools.protect_path(path).map_err(|e| anyhow!(e))?;
    }
    let runtime = Arc::new(AgentRuntime::new(
        Arc::from(provider),
        tools.clone(),
        config(&model)?,
    ));
    Ok(Setup {
        runtime,
        tools,
        model,
    })
}

pub(crate) async fn run_command(
    args: RunArgs,
    settings: &ProviderSettings,
    secrets: &SecretsStore,
    db_path: &std::path::Path,
    policy: ExecutionPolicy,
) -> Result<()> {
    if args.sessions {
        for s in agent_session::list() {
            println!(
                "{}  {:<18} {} step(s)  {}",
                s.id,
                format!("{:?}", s.outcome).to_lowercase(),
                s.steps,
                s.goal.lines().next().unwrap_or("")
            );
        }
        return Ok(());
    }
    let mut record = match &args.resume {
        Some(id) => agent_session::load(id)?,
        None => {
            let goal = args.task.as_deref().filter(|t| !t.trim().is_empty());
            let goal = goal.ok_or_else(|| anyhow!("provide a task, or --resume <session>"))?;
            let workspace = args.workspace.clone().unwrap_or(std::env::current_dir()?);
            let workspace = workspace
                .canonicalize()
                .with_context(|| format!("workspace {}", workspace.display()))?;
            SessionRecord::new(goal, &workspace.to_string_lossy(), "", "")
        }
    };
    let Setup {
        runtime,
        tools,
        model,
    } = setup(
        settings,
        secrets,
        db_path,
        policy,
        &record.workspace,
        |model| config_from(&args, model),
    )?;
    record.provider = format!("{:?}", settings.choice).to_lowercase();
    record.model = model;
    record.begin_run();

    let (history, goal) = if args.resume.is_some() {
        let note = args
            .task
            .clone()
            .unwrap_or_else(|| "Continue the task.".into());
        (
            record.resumable_history(),
            format!(
                "Session resumed in a new process. Earlier tool calls that had not finished were NOT executed and nothing is pending approval. Propose any still-needed action again. {note}"
            ),
        )
    } else {
        (Vec::new(), record.goal.clone())
    };

    let cancel = CancellationToken::default();
    OWNS_INTERRUPT.store(true, Ordering::SeqCst);
    let interrupt = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        })
    };
    let observed = Arc::new(Mutex::new(Observed::default()));
    let emit = {
        let observed = observed.clone();
        let output = if args.json {
            Output::Json
        } else {
            Output::Human
        };
        move |event: AgentEvent| observe(&event, &observed, output)
    };
    let interactive = std::io::stdin().is_terminal();

    let mut run = runtime.run(&goal, history, cancel.clone(), &emit).await;
    while run.outcome == AgentOutcome::WaitingForApproval {
        let pending = observed.lock().ok().and_then(|mut o| o.approval.take());
        let Some((call, description)) = pending else {
            break;
        };
        let approved = interactive && !args.json && ask(&description);
        let decision = if !interactive || args.json {
            "pending"
        } else if approved {
            "approved"
        } else {
            "denied"
        };
        if let Ok(mut o) = observed.lock() {
            o.log.push(LogEntry::Approval {
                call_id: call.id.clone(),
                name: call.name.clone(),
                description: helm_core::redact_secrets(&description),
                decision: decision.into(),
            });
        }
        if decision == "pending" {
            break;
        }
        if let Err(error) = tools.decide(&call, approved) {
            bail!("approval could not be recorded: {error}");
        }
        run = runtime.resume(run, cancel.clone(), &emit).await;
    }
    interrupt.abort();
    OWNS_INTERRUPT.store(false, Ordering::SeqCst);

    conclude(&mut record, &run, &observed);
    agent_session::save(&mut record)?;
    if args.json {
        println!("{}", serde_json::to_string(&record)?);
    } else {
        println!("{}", record.summary);
        println!(
            "\nsession {} · {:?} · {} step(s) · {} evidence item(s)",
            record.id,
            record.outcome,
            record.steps,
            record.evidence.len()
        );
    }
    match record.outcome {
        AgentOutcome::Completed => Ok(()),
        AgentOutcome::WaitingForApproval => bail!(
            "approval pending; nothing was executed. Resume interactively: helm run --resume {}",
            record.id
        ),
        other => bail!("agent run {other:?}; session {}", record.id),
    }
}

pub(crate) fn conclude(record: &mut SessionRecord, run: &AgentRun, observed: &Mutex<Observed>) {
    record.outcome = run.outcome;
    record.summary = run.summary.clone();
    for item in &run.evidence {
        if !record.evidence.contains(item) {
            record.evidence.push(item.clone());
        }
    }
    record.set_run_totals(
        run.steps,
        u64::from(run.usage.input_tokens),
        u64::from(run.usage.output_tokens),
        run.cost_usd,
    );
    record.messages = run.messages.clone();
    if let Ok(mut observed) = observed.lock() {
        record.log.append(&mut observed.log);
        // A pending approval that never got a decision is recorded as such, once.
        if let Some((call, description)) = observed.approval.take() {
            record.log.push(LogEntry::Approval {
                call_id: call.id,
                name: call.name,
                description: helm_core::redact_secrets(&description),
                decision: "pending".into(),
            });
        }
    }
}
