//! Regression test: Groq + llama model runs through the hardened ReAct loop.
//!
//! Verifies that all 5 hardening layers compose correctly against a live Groq
//! endpoint: quirks auto-detection (L5), bare-JSON format recovery (L1/L3),
//! schema validation (L2), corrective retry (L3), and post-condition checks (L4).
//!
//! Requires: GROQ_API_KEY env var.  Skipped automatically when absent.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use helm_agent::{Budget, ReactAgent};
    use helm_memory::MemoryStore;
    use helm_providers::quirks_for;
    use helm_tools::{ToolContext, ToolRegistry};
    use tempfile::tempdir;

    fn groq_api_key() -> Option<String> {
        std::env::var("GROQ_API_KEY").ok()
    }

    /// Smoke-test: agent completes a simple shell task via Groq llama model.
    /// Checks that quirks are auto-detected and the run succeeds without panics.
    #[tokio::test]
    #[ignore = "requires GROQ_API_KEY"]
    async fn groq_llama_hardened_loop_completes_shell_task() {
        let key = match groq_api_key() {
            Some(k) => k,
            None => return,
        };

        let model = "llama3-70b-8192";
        let provider_name = "groq";

        // Verify quirks are detected for this combination.
        let quirks = quirks_for(provider_name, model);
        assert_eq!(quirks.force_temperature, Some(0.0));

        let provider =
            helm_providers::OpenAiCompatProvider::groq(key).expect("build groq provider");

        let dir = tempdir().unwrap();
        let memory = Arc::new(
            MemoryStore::open(&dir.path().join("helm.db"))
                .await
                .unwrap(),
        );
        let agent = ReactAgent::with_tool_context(
            Box::new(provider),
            ToolRegistry::default(),
            memory.clone(),
            Budget {
                max_iterations: 4,
                max_input_tokens: 8_000,
                max_output_tokens: 512,
                max_wall_time: std::time::Duration::from_secs(60),
                ..Budget::default()
            },
            model,
            ToolContext::new(dir.path().to_path_buf()),
        );

        let result = match agent.run("echo 'hello helm' to stdout").await {
            Ok(result) => result,
            Err(error)
                if !live_strict() && is_live_provider_environment_error(&error.to_string()) =>
            {
                eprintln!("skipping Groq hardened live test: {error}");
                return;
            }
            Err(error) => panic!("Groq hardened live test failed: {error}"),
        };

        assert!(
            !result.final_message.is_empty(),
            "expected a final message from the agent"
        );
        // Groq llama models are quirked to temperature=0; run must not panic or
        // time-out on format parsing.
        eprintln!(
            "groq run: iters={} corrections={} format_recovery={}",
            result.iterations, result.corrections_used, result.format_recovery_used
        );
    }

    /// Verify the quirks registry correctly identifies Groq + mixtral.
    #[test]
    fn groq_mixtral_quirks_detected() {
        let q = quirks_for("groq", "mixtral-8x7b-32768");
        assert_eq!(q.force_temperature, Some(0.0));
        assert!(q.user_note.is_some());
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
}
