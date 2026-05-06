use std::{fs, sync::Arc, time::Duration};

use helm_agent::{Budget, ReactAgent};
use helm_core::ContentBlock;
use helm_memory::{EpisodeOutcome, MemoryStore};
use helm_providers::{ChatResponse, MockProvider, StopReason, Usage};
use helm_tools::{ToolContext, ToolRegistry};
use serde_json::json;
use tempfile::tempdir;

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

async fn run_file_roundtrip(case_index: usize) {
    let dir = tempdir().unwrap();
    let output = dir.path().join(format!("roundtrip-{case_index}.txt"));
    let memory = Arc::new(
        MemoryStore::open(&dir.path().join("helm.db"))
            .await
            .unwrap(),
    );
    let provider = MockProvider::new(vec![
        response(
            vec![ContentBlock::ToolUse {
                id: "write_1".to_owned(),
                name: "fs_write".to_owned(),
                input: json!({
                    "path": output,
                    "content": format!("HELLO-{case_index}"),
                    "mode": "create_only"
                }),
            }],
            StopReason::ToolUse,
        ),
        response(
            vec![ContentBlock::ToolUse {
                id: "read_1".to_owned(),
                name: "fs_read".to_owned(),
                input: json!({"path": output}),
            }],
            StopReason::ToolUse,
        ),
        response(
            vec![ContentBlock::Text(format!("verified HELLO-{case_index}"))],
            StopReason::EndTurn,
        ),
    ]);
    let agent = ReactAgent::with_tool_context(
        Box::new(provider),
        ToolRegistry::default(),
        Arc::clone(&memory),
        Budget {
            max_iterations: 6,
            max_wall_time: Duration::from_secs(30),
            ..Budget::default()
        },
        "mock",
        ToolContext::new(dir.path().to_path_buf()),
    );

    let result = agent.run("create a file and read it back").await.unwrap();
    let episode = memory
        .get_episode(&result.episode_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        episode.outcome,
        Some(EpisodeOutcome::Success.as_str().to_owned())
    );
    assert_eq!(
        fs::read_to_string(output).unwrap(),
        format!("HELLO-{case_index}")
    );
}

#[tokio::test]
async fn deterministic_25_run_reliability_suite() {
    for case_index in 0..25 {
        run_file_roundtrip(case_index).await;
    }
}

#[tokio::test]
#[ignore = "release gate: run before v1 RC"]
async fn deterministic_100_run_release_suite() {
    for case_index in 0..100 {
        run_file_roundtrip(case_index).await;
    }
}
