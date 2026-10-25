use async_trait::async_trait;
use helm_agent::*;
use helm_core::{ContentBlock, Message, ProviderError};
use helm_monitor::process_runner::CancellationToken;
use helm_providers::{ChatRequest, ChatResponse, Provider, StopReason, ToolSchema, Usage};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Script {
    turns: Mutex<VecDeque<Result<ChatResponse, ProviderError>>>,
    requests: Mutex<Vec<ChatRequest>>,
}
#[async_trait]
impl Provider for Script {
    fn name(&self) -> &'static str {
        "script"
    }
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        self.turns
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected provider request")
    }
}
struct Tools {
    approved: AtomicBool,
    effects: AtomicUsize,
}
#[async_trait]
impl ToolExecutor for Tools {
    fn schemas(&self) -> Vec<ToolSchema> {
        vec!["read","write"].into_iter().map(|name|ToolSchema{name:name.into(),description:"test host".into(),input_schema:json!({"type":"object","required":["value"],"properties":{"value":{"type":"string"}},"additionalProperties":false})}).collect()
    }
    async fn execute(&self, call: &ToolCall, _: &CancellationToken) -> ToolExecution {
        if call.name == "write" && !self.approved.load(Ordering::SeqCst) {
            return ToolExecution::ApprovalRequired {
                description: "exact write".into(),
            };
        }
        self.effects.fetch_add(1, Ordering::SeqCst);
        ToolExecution::Completed(ToolOutput {
            content: "observed".into(),
            is_error: false,
            evidence: vec![format!("checked:{}", call.id)],
        })
    }
}
fn response(
    blocks: Vec<ContentBlock>,
    stop_reason: StopReason,
) -> Result<ChatResponse, ProviderError> {
    Ok(ChatResponse {
        id: "fixture".into(),
        content: blocks,
        stop_reason,
        usage: Usage {
            input_tokens: 5,
            output_tokens: 5,
        },
    })
}
fn call(id: &str, name: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.into(),
        name: name.into(),
        input: json!({"value":"fixture"}),
    }
}
fn final_turn() -> Result<ChatResponse, ProviderError> {
    response(
        vec![ContentBlock::Text("Verified fixture".into())],
        StopReason::EndTurn,
    )
}
fn setup(
    turns: Vec<Result<ChatResponse, ProviderError>>,
    config: AgentConfig,
) -> (AgentRuntime, Arc<Script>, Arc<Tools>) {
    let provider = Arc::new(Script {
        turns: Mutex::new(turns.into()),
        requests: Mutex::new(Vec::new()),
    });
    let tools = Arc::new(Tools {
        approved: AtomicBool::new(false),
        effects: AtomicUsize::new(0),
    });
    (
        AgentRuntime::new(provider.clone(), tools.clone(), config),
        provider,
        tools,
    )
}
#[tokio::test]
async fn multiple_tool_turns_preserve_results_and_evidence() {
    let (runtime, provider, tools) = setup(
        vec![
            response(vec![call("a", "read")], StopReason::ToolUse),
            response(vec![call("b", "read")], StopReason::ToolUse),
            final_turn(),
        ],
        AgentConfig::default(),
    );
    let events = Mutex::new(Vec::new());
    let run = runtime
        .run("inspect fixture", vec![], Default::default(), |e| {
            events.lock().unwrap().push(e)
        })
        .await;
    assert_eq!(run.outcome, AgentOutcome::Completed);
    assert_eq!(tools.effects.load(Ordering::SeqCst), 2);
    assert_eq!(run.evidence, vec!["checked:a", "checked:b"]);
    assert_eq!(run.usage.input_tokens, 15);
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e,AgentEvent::ToolResult{id,..} if id=="b"))
    );
    assert!(
        provider.requests.lock().unwrap()[2]
            .messages
            .iter()
            .any(|m| m
                .content
                .iter()
                .any(|b| matches!(b,ContentBlock::ToolResult{tool_use_id,..} if tool_use_id=="a")))
    );
}
#[tokio::test]
async fn pending_approval_resumes_without_replaying_completed_effects() {
    let (runtime, provider, tools) = setup(
        vec![
            response(
                vec![call("a", "read"), call("b", "write")],
                StopReason::ToolUse,
            ),
            final_turn(),
        ],
        AgentConfig::default(),
    );
    let waiting = runtime
        .run("edit", vec![], Default::default(), |_| {})
        .await;
    assert_eq!(waiting.outcome, AgentOutcome::WaitingForApproval);
    assert_eq!(tools.effects.load(Ordering::SeqCst), 1);
    assert_eq!(waiting.pending[0].id, "b");
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
    tools.approved.store(true, Ordering::SeqCst);
    let done = runtime.resume(waiting, Default::default(), |_| {}).await;
    assert_eq!(done.outcome, AgentOutcome::Completed);
    assert_eq!(tools.effects.load(Ordering::SeqCst), 2);
    let requests = provider.requests.lock().unwrap();
    let results = &requests[1].messages.last().unwrap().content;
    assert_eq!(results.len(), 2);
}
#[tokio::test]
async fn malformed_arguments_and_unknown_tools_never_reach_host() {
    let bad = ContentBlock::ToolUse {
        id: "bad".into(),
        name: "read".into(),
        input: json!({"value":42}),
    };
    let (runtime, provider, tools) = setup(
        vec![
            response(
                vec![bad, call("unknown", "not_installed")],
                StopReason::ToolUse,
            ),
            final_turn(),
        ],
        AgentConfig::default(),
    );
    let run = runtime
        .run("check", vec![], Default::default(), |_| {})
        .await;
    assert_eq!(tools.effects.load(Ordering::SeqCst), 0);
    assert_eq!(run.outcome, AgentOutcome::Failed);
    assert!(
        provider.requests.lock().unwrap()[1]
            .messages
            .last()
            .unwrap()
            .content
            .iter()
            .all(|block| matches!(block, ContentBlock::ToolResult { is_error: true, .. }))
    );
}
#[tokio::test]
async fn model_prose_never_executes_a_tool() {
    let (runtime, _, tools) = setup(
        vec![response(
            vec![ContentBlock::Text(
                "<tool>write</tool> {\"name\":\"write\"}".into(),
            )],
            StopReason::EndTurn,
        )],
        AgentConfig::default(),
    );
    let run = runtime
        .run("explain", vec![], Default::default(), |_| {})
        .await;
    assert_eq!(tools.effects.load(Ordering::SeqCst), 0);
    assert!(run.summary.contains("No tools executed"));
}
#[tokio::test]
async fn unknown_pricing_blocks_hard_budget_before_provider() {
    let config = AgentConfig {
        budget_usd: Some(1.0),
        ..Default::default()
    };
    let (runtime, provider, _) = setup(vec![], config);
    let run = runtime
        .run("check", vec![], Default::default(), |_| {})
        .await;
    assert_eq!(run.outcome, AgentOutcome::Failed);
    assert!(run.summary.contains("pricing"));
    assert!(provider.requests.lock().unwrap().is_empty());
}
#[tokio::test]
async fn insufficient_reserved_budget_blocks_request() {
    let config = AgentConfig {
        budget_usd: Some(0.000001),
        pricing: Some(Pricing {
            input_per_million: 1.0,
            output_per_million: 1.0,
        }),
        ..Default::default()
    };
    let (runtime, provider, _) = setup(vec![], config);
    let run = runtime
        .run("check", vec![], Default::default(), |_| {})
        .await;
    assert_eq!(run.outcome, AgentOutcome::Failed);
    assert!(run.summary.contains("budget"));
    assert!(provider.requests.lock().unwrap().is_empty());
}
#[tokio::test]
async fn step_exhaustion_stops_without_an_extra_request() {
    let config = AgentConfig {
        max_steps: 1,
        ..Default::default()
    };
    let (runtime, provider, _) = setup(
        vec![response(vec![call("a", "read")], StopReason::ToolUse)],
        config,
    );
    let run = runtime
        .run("check", vec![], Default::default(), |_| {})
        .await;
    assert_eq!(run.outcome, AgentOutcome::Failed);
    assert!(run.summary.contains("step"));
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}
struct Hanging;
#[async_trait]
impl Provider for Hanging {
    fn name(&self) -> &'static str {
        "hanging"
    }
    async fn chat(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        std::future::pending().await
    }
}
#[tokio::test]
async fn cancellation_and_wall_deadline_stop_hanging_provider() {
    let tools = Arc::new(Tools {
        approved: AtomicBool::new(false),
        effects: AtomicUsize::new(0),
    });
    let runtime = AgentRuntime::new(
        Arc::new(Hanging),
        tools.clone(),
        AgentConfig {
            wall_time: Duration::from_secs(30),
            request_timeout: Duration::from_secs(10),
            ..Default::default()
        },
    );
    let cancel = CancellationToken::default();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        trigger.cancel();
    });
    let run = tokio::time::timeout(
        Duration::from_secs(1),
        runtime.run("check", vec![], cancel, |_| {}),
    )
    .await
    .unwrap();
    assert_eq!(run.outcome, AgentOutcome::Cancelled, "{}", run.summary);
    let runtime = AgentRuntime::new(
        Arc::new(Hanging),
        tools,
        AgentConfig {
            wall_time: Duration::from_millis(100),
            request_timeout: Duration::from_secs(10),
            ..Default::default()
        },
    );
    let run = tokio::time::timeout(
        Duration::from_secs(1),
        runtime.run("check", vec![], Default::default(), |_| {}),
    )
    .await
    .unwrap();
    assert_eq!(run.outcome, AgentOutcome::Failed);
    assert!(run.summary.contains("deadline"));
}
#[tokio::test]
async fn provider_errors_retry_only_to_configured_bound() {
    let config = AgentConfig {
        max_retries: 1,
        ..Default::default()
    };
    let (runtime, provider, _) = setup(
        vec![Err(ProviderError::Timeout), Err(ProviderError::Timeout)],
        config,
    );
    let run = runtime
        .run("check", vec![], Default::default(), |_| {})
        .await;
    assert_eq!(run.outcome, AgentOutcome::Failed);
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn repeated_tool_ids_and_ambiguous_stop_reason_fail_before_execution() {
    for (blocks, reason) in [
        (
            vec![call("same", "read"), call("same", "read")],
            StopReason::ToolUse,
        ),
        (vec![call("a", "read")], StopReason::EndTurn),
    ] {
        let (runtime, _, tools) = setup(vec![response(blocks, reason)], AgentConfig::default());
        let run = runtime
            .run("check", vec![], Default::default(), |_| {})
            .await;
        assert_eq!(run.outcome, AgentOutcome::Failed);
        assert_eq!(tools.effects.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn oversized_context_fails_before_provider() {
    let (runtime, provider, _) = setup(
        vec![],
        AgentConfig {
            context_bytes: 16,
            ..Default::default()
        },
    );
    let run = runtime
        .run("long goal", vec![], Default::default(), |_| {})
        .await;
    assert_eq!(run.outcome, AgentOutcome::Failed);
    assert!(run.summary.contains("context"));
    assert!(provider.requests.lock().unwrap().is_empty());
}
#[test]
fn empty_history_fixture_is_provider_neutral() {
    let _: Vec<Message> = vec![];
}
