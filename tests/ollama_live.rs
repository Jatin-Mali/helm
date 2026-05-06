use std::{env, fs, sync::Arc};

use helm_agent::{Budget, ReactAgent};
use helm_memory::{EpisodeOutcome, MemoryStore};
use helm_providers::OllamaProvider;
use helm_tools::{ToolContext, ToolRegistry};
use serde_json::Value;
use tempfile::tempdir;

#[tokio::test]
#[ignore]
async fn ollama_qwen3_live_react_loop() {
    let base_url = env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let tags = match fetch_tags(&base_url).await {
        Ok(tags) => tags,
        Err(error) => {
            eprintln!("skipping Ollama live test: {error}");
            return;
        }
    };
    if !has_model(&tags, "qwen3:4b") {
        eprintln!("skipping Ollama live test: run `ollama pull qwen3:4b` first");
        return;
    }

    let dir = tempdir().unwrap();
    let output_path = dir.path().join("ollama_live.txt");
    let memory = Arc::new(
        MemoryStore::open(&dir.path().join("helm.db"))
            .await
            .unwrap(),
    );
    let provider = OllamaProvider::with_base_url(base_url).unwrap();
    let agent = ReactAgent::with_tool_context(
        Box::new(provider),
        ToolRegistry::default(),
        Arc::clone(&memory),
        Budget {
            max_iterations: 10,
            ..Budget::default()
        },
        "qwen3:4b",
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

async fn fetch_tags(base_url: &str) -> Result<Value, String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|error| format!("cannot reach {base_url}: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("cannot read /api/tags response: {error}"))?;
    if !status.is_success() {
        return Err(format!("Ollama returned HTTP {}: {body}", status.as_u16()));
    }
    serde_json::from_str(&body).map_err(|error| format!("invalid /api/tags JSON: {error}"))
}

fn has_model(tags: &Value, model_name: &str) -> bool {
    tags.get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models.iter().any(|model| {
                model
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name == model_name)
            })
        })
        .unwrap_or(false)
}
