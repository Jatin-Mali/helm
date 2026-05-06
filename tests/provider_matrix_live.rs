use std::{env, fs, sync::Arc, time::Duration};

use helm_agent::{Budget, ReactAgent};
use helm_memory::{EpisodeOutcome, MemoryStore};
use helm_providers::{GeminiProvider, OllamaProvider, OpenAiCompatProvider, Provider};
use helm_tools::{ToolContext, ToolRegistry};
use tempfile::tempdir;

async fn run_provider_case(
    label: &str,
    provider: Box<dyn Provider>,
    model: &str,
) -> Result<(), String> {
    let dir = tempdir().unwrap();
    let output_path = dir.path().join(format!("{label}-provider.txt"));
    let memory = Arc::new(
        MemoryStore::open(&dir.path().join("helm.db"))
            .await
            .unwrap(),
    );
    let agent = ReactAgent::with_tool_context(
        provider,
        ToolRegistry::default(),
        Arc::clone(&memory),
        Budget {
            max_iterations: 8,
            max_input_tokens: 12_000,
            max_output_tokens: 1_024,
            max_wall_time: Duration::from_secs(120),
        },
        model,
        ToolContext::new(dir.path().to_path_buf()),
    );
    let task = format!(
        "write the text 'provider matrix {label}' to {}, then read it back and summarize",
        output_path.display()
    );

    let result = agent
        .run(&task)
        .await
        .map_err(|error| format!("{label}: {error}"))?;
    let episode = memory
        .get_episode(&result.episode_id)
        .await
        .map_err(|error| format!("{label}: {error}"))?
        .ok_or_else(|| format!("{label}: episode not found"))?;
    let content = fs::read_to_string(&output_path).map_err(|error| format!("{label}: {error}"))?;

    assert_eq!(
        episode.outcome,
        Some(EpisodeOutcome::Success.as_str().to_owned()),
        "{label} episode failed: {:?}",
        episode.error
    );
    assert_eq!(content, format!("provider matrix {label}"));
    Ok(())
}

#[tokio::test]
#[ignore = "requires real provider keys and/or local Ollama model"]
async fn live_provider_matrix_runs_available_providers() {
    let strict = env::var_os("HELM_LIVE_STRICT").is_some();
    let mut attempted = 0_u32;
    let mut passed = 0_u32;
    let mut failures = Vec::new();

    if env::var_os("GROQ_API_KEY").is_some() {
        attempted += 1;
        record_live_result(
            strict,
            &mut passed,
            &mut failures,
            "groq",
            run_provider_case(
                "groq",
                Box::new(OpenAiCompatProvider::groq_from_env().unwrap()),
                &env::var("HELM_GROQ_MODEL").unwrap_or_else(|_| "openai/gpt-oss-20b".to_owned()),
            )
            .await,
        )
    }

    if env::var_os("OPENROUTER_API_KEY").is_some() {
        attempted += 1;
        record_live_result(
            strict,
            &mut passed,
            &mut failures,
            "openrouter",
            run_provider_case(
                "openrouter",
                Box::new(OpenAiCompatProvider::openrouter_from_env().unwrap()),
                &env::var("HELM_OPENROUTER_MODEL")
                    .unwrap_or_else(|_| "meta-llama/llama-3.3-70b-instruct".to_owned()),
            )
            .await,
        )
    }

    if env::var_os("NVIDIA_API_KEY").is_some() {
        attempted += 1;
        record_live_result(
            strict,
            &mut passed,
            &mut failures,
            "nvidia",
            run_provider_case(
                "nvidia",
                Box::new(OpenAiCompatProvider::nvidia_nim_from_env().unwrap()),
                &env::var("HELM_NVIDIA_MODEL")
                    .unwrap_or_else(|_| "meta/llama-3.3-70b-instruct".to_owned()),
            )
            .await,
        )
    }

    if env::var_os("GOOGLE_API_KEY").is_some() || env::var_os("GEMINI_API_KEY").is_some() {
        attempted += 1;
        record_live_result(
            strict,
            &mut passed,
            &mut failures,
            "gemini",
            run_provider_case(
                "gemini",
                Box::new(GeminiProvider::from_env().unwrap()),
                &env::var("HELM_GEMINI_MODEL")
                    .unwrap_or_else(|_| GeminiProvider::default_model().to_owned()),
            )
            .await,
        )
    }

    if ollama_qwen_available().await {
        attempted += 1;
        record_live_result(
            strict,
            &mut passed,
            &mut failures,
            "ollama",
            run_provider_case(
                "ollama",
                Box::new(OllamaProvider::from_env().unwrap()),
                &env::var("HELM_OLLAMA_MODEL")
                    .unwrap_or_else(|_| OllamaProvider::default_model().to_owned()),
            )
            .await,
        )
    }

    if attempted == 0 {
        eprintln!("skipping provider matrix: no provider keys and no qwen3:4b Ollama model");
    }
    if strict && !failures.is_empty() {
        panic!("provider matrix failures:\n{}", failures.join("\n"));
    }
    eprintln!("provider matrix: {passed}/{attempted} provider(s) passed");
}

async fn ollama_qwen_available() -> bool {
    let base = env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_owned());
    let Ok(response) = reqwest::get(format!("{}/api/tags", base.trim_end_matches('/'))).await
    else {
        return false;
    };
    let Ok(text) = response.text().await else {
        return false;
    };
    text.contains("\"qwen3:4b\"") || text.contains("\"qwen3")
}

fn record_live_result(
    strict: bool,
    passed: &mut u32,
    failures: &mut Vec<String>,
    label: &str,
    result: Result<(), String>,
) {
    match result {
        Ok(()) => {
            *passed = passed.saturating_add(1);
            eprintln!("{label}: ok");
        }
        Err(error) => {
            let message = format!("{label}: {error}");
            if strict {
                failures.push(message);
            } else {
                eprintln!("{message}");
            }
        }
    }
}
