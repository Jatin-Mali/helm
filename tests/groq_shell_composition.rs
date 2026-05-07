use std::{env, fs, path::PathBuf, sync::Arc, time::Duration};

use helm_agent::{Budget, ReactAgent};
use helm_memory::MemoryStore;
use helm_providers::OpenAiCompatProvider;
use helm_tools::{ToolContext, ToolRegistry};
use tempfile::tempdir;
use uuid::Uuid;

struct Cleanup {
    path: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[tokio::test]
#[ignore]
async fn groq_expands_shell_composition_in_output_file() {
    if env::var_os("GROQ_API_KEY").is_none() {
        eprintln!("skipping Groq shell composition test: GROQ_API_KEY is unset");
        return;
    }
    tokio::time::sleep(Duration::from_secs(10)).await;

    let dir = tempdir().unwrap();
    let output_path = PathBuf::from(format!("/tmp/helm-hello-{}.txt", Uuid::new_v4()));
    let _cleanup = Cleanup {
        path: output_path.clone(),
    };
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
            max_iterations: 12,
            max_output_tokens: 1_024,
            ..Budget::default()
        },
        "openai/gpt-oss-20b",
        ToolContext::new(dir.path().to_path_buf()),
    );
    let task = format!(
        "create {} with the current date and uname -a, then read it back",
        output_path.display()
    );

    let _result = match agent.run(&task).await {
        Ok(result) => result,
        Err(error) if !live_strict() && is_live_provider_environment_error(&error.to_string()) => {
            eprintln!("skipping Groq shell composition live test: {error}");
            return;
        }
        Err(error) => panic!("Groq shell composition live test failed: {error}"),
    };
    let content = fs::read_to_string(&output_path).unwrap();

    assert!(!content.contains("$(date)"));
    assert!(!content.contains("$(uname"));
    assert!(contains_four_digit_year(&content));
    assert!(content.contains("Linux"));
}

fn contains_four_digit_year(text: &str) -> bool {
    text.as_bytes()
        .windows(4)
        .any(|window| window.iter().all(u8::is_ascii_digit))
}

fn is_live_provider_environment_error(error: &str) -> bool {
    error.contains("HTTP 401")
        || error.contains("invalid_api_key")
        || error.contains("Invalid API Key")
        || error.contains("HTTP 429")
        || error.to_ascii_lowercase().contains("rate limit")
}

fn live_strict() -> bool {
    std::env::var_os("HELM_LIVE_STRICT").is_some()
}
