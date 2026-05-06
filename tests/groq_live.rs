use std::{env, fs, sync::Arc};

use helm_agent::{Budget, ReactAgent};
use helm_memory::{EpisodeOutcome, MemoryStore};
use helm_providers::OpenAiCompatProvider;
use helm_tools::{ToolContext, ToolRegistry};
use tempfile::tempdir;

#[tokio::test]
#[ignore]
async fn groq_live_react_loop() {
    if env::var_os("GROQ_API_KEY").is_none() {
        eprintln!("skipping Groq live test: GROQ_API_KEY is unset");
        return;
    }

    let dir = tempdir().unwrap();
    let output_path = dir.path().join("groq_live.txt");
    let memory = Arc::new(
        MemoryStore::open(&dir.path().join("helm.db"))
            .await
            .unwrap(),
    );
    let provider = OpenAiCompatProvider::groq_from_env().unwrap();
    let agent = ReactAgent::with_tool_context(
        Box::new(provider),
        ToolRegistry::default(),
        Arc::clone(&memory),
        Budget {
            max_iterations: 10,
            max_output_tokens: 1_024,
            ..Budget::default()
        },
        "openai/gpt-oss-20b",
        ToolContext::new(dir.path().to_path_buf()),
    );
    let task = format!(
        "write the text 'hello from helm' to {}, then read it back and tell me what it says",
        output_path.display()
    );

    let result = agent.run(&task).await.unwrap();
    let episode = memory
        .get_episode(&result.episode_id)
        .await
        .unwrap()
        .unwrap();
    let file_content = fs::read_to_string(&output_path).unwrap();

    assert_eq!(
        episode.outcome,
        Some(EpisodeOutcome::Success.as_str().to_owned())
    );
    assert_eq!(file_content, "hello from helm");
    assert!(
        result
            .final_message
            .to_lowercase()
            .contains("hello from helm")
    );
    assert!(episode.model_capability_warning.is_none());
}
