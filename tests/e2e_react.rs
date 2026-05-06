use std::{fs, sync::Arc};

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
            input_tokens: 10,
            output_tokens: 5,
        },
    }
}

#[tokio::test]
async fn e2e_react_reads_and_writes_file_with_mock_provider() {
    let dir = tempdir().unwrap();
    let notes = dir.path().join("notes.txt");
    let upper = dir.path().join("notes_upper.txt");
    fs::write(&notes, "hello world").unwrap();
    let memory = Arc::new(
        MemoryStore::open(&dir.path().join("helm.db"))
            .await
            .unwrap(),
    );
    let provider = MockProvider::new(vec![
        response(
            vec![ContentBlock::ToolUse {
                id: "toolu_read".to_owned(),
                name: "fs_read".to_owned(),
                input: json!({"path": notes}),
            }],
            StopReason::ToolUse,
        ),
        response(
            vec![ContentBlock::ToolUse {
                id: "toolu_write".to_owned(),
                name: "fs_write".to_owned(),
                input: json!({"path": upper, "content": "HELLO WORLD"}),
            }],
            StopReason::ToolUse,
        ),
        response(
            vec![ContentBlock::Text(
                "Done. Wrote uppercased copy to notes_upper.txt.".to_owned(),
            )],
            StopReason::EndTurn,
        ),
    ]);
    let agent = ReactAgent::with_tool_context(
        Box::new(provider),
        ToolRegistry::default(),
        Arc::clone(&memory),
        Budget::default(),
        "mock",
        ToolContext::new(dir.path().to_path_buf()),
    );

    let result = agent.run("uppercase notes").await.unwrap();
    let episode = memory
        .episode_by_id(&result.episode_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fs::read_to_string(&upper).unwrap(), "HELLO WORLD");
    assert_eq!(result.iterations, 3);
    assert_eq!(
        episode.outcome,
        Some(EpisodeOutcome::Success.as_str().to_owned())
    );
    assert_eq!(memory.episode_count().await.unwrap(), 1);
    assert!(memory.step_count(&result.episode_id).await.unwrap() >= 3);
}
