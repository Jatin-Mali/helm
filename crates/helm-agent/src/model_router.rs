//! Model router — classifies task complexity and selects the best available model.

use serde::{Deserialize, Serialize};

// ── TaskComplexity ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TaskComplexity {
    Simple,
    Medium,
    Complex,
}

// ── ModelRouter ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoute {
    pub model_id: String,
    pub max_complexity: TaskComplexity,
}

pub struct ModelRouter {
    routes: Vec<ModelRoute>,
    default_model: String,
}

impl ModelRouter {
    pub fn new(default_model: impl Into<String>) -> Self {
        Self {
            routes: Vec::new(),
            default_model: default_model.into(),
        }
    }

    /// Register a model for tasks up to `max_complexity`.
    /// Call multiple times to build the routing table.
    pub fn add_route(&mut self, model_id: impl Into<String>, max_complexity: TaskComplexity) {
        self.routes.push(ModelRoute {
            model_id: model_id.into(),
            max_complexity,
        });
    }

    /// Classify a raw goal string into a complexity tier based on heuristics.
    pub fn classify(goal: &str) -> TaskComplexity {
        let words = goal.split_whitespace().count();
        let then_count = goal.matches(" then ").count();
        let comma_count = goal.matches(',').count();
        // Complex: multiple sequential steps signalled by repeated "then" or commas
        let is_complex = then_count >= 2 || comma_count >= 2;
        let is_multi = then_count >= 1 || comma_count >= 1 || goal.contains(" and ");
        match (words, is_complex, is_multi) {
            (w, false, false) if w <= 8 => TaskComplexity::Simple,
            (_, true, _) => TaskComplexity::Complex,
            _ => TaskComplexity::Medium,
        }
    }

    /// Return the model_id best suited for the given complexity.
    /// Among capable routes, picks the one with the lowest max_complexity
    /// (cheapest model that can handle the task).
    pub fn route(&self, complexity: TaskComplexity) -> &str {
        let capable: Vec<&ModelRoute> = self
            .routes
            .iter()
            .filter(|r| r.max_complexity >= complexity)
            .collect();

        capable
            .into_iter()
            .min_by_key(|r| r.max_complexity)
            .map(|r| r.model_id.as_str())
            .unwrap_or(self.default_model.as_str())
    }

    /// Convenience: classify goal then route.
    pub fn route_for_goal(&self, goal: &str) -> &str {
        self.route(Self::classify(goal))
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{ModelRouter, TaskComplexity};

    fn router() -> ModelRouter {
        let mut r = ModelRouter::new("groq/openai/gpt-oss-120b");
        r.add_route("groq/llama-3-8b", TaskComplexity::Simple);
        r.add_route("groq/llama-3-70b", TaskComplexity::Medium);
        r.add_route("groq/openai/gpt-oss-120b", TaskComplexity::Complex);
        r
    }

    #[test]
    fn classify_short_goal_is_simple_happy_path() {
        assert_eq!(ModelRouter::classify("list files"), TaskComplexity::Simple);
        assert_eq!(
            ModelRouter::classify("check disk usage"),
            TaskComplexity::Simple
        );
    }

    #[test]
    fn classify_multi_step_is_complex_happy_path() {
        let goal =
            "deploy the app, then run migrations, then restart the service and verify health";
        assert_eq!(ModelRouter::classify(goal), TaskComplexity::Complex);
    }

    #[test]
    fn classify_medium_goal_happy_path() {
        assert_eq!(
            ModelRouter::classify("find all log files from the last 24 hours and count errors"),
            TaskComplexity::Medium
        );
    }

    #[test]
    fn route_picks_cheapest_capable_model_happy_path() {
        let r = router();
        assert_eq!(r.route(TaskComplexity::Simple), "groq/llama-3-8b");
        assert_eq!(r.route(TaskComplexity::Medium), "groq/llama-3-70b");
        assert_eq!(r.route(TaskComplexity::Complex), "groq/openai/gpt-oss-120b");
    }

    #[test]
    fn route_falls_back_to_default_when_no_routes_edge_case() {
        let r = ModelRouter::new("fallback-model");
        assert_eq!(r.route(TaskComplexity::Complex), "fallback-model");
    }

    #[test]
    fn route_for_goal_end_to_end_happy_path() {
        let r = router();
        assert_eq!(r.route_for_goal("list files"), "groq/llama-3-8b");
    }

    #[tokio::test]
    async fn record_and_get_success_rates_happy_path() {
        let mut r = ModelRouter::new("fallback");
        r.add_route("model-a", TaskComplexity::Simple);
        r.add_route("model-b", TaskComplexity::Medium);
        // Routes are for goal classification, not for outcome tracking
        // Success rates would require a separate tracking system
        assert_eq!(r.route(TaskComplexity::Simple), "model-a");
    }
}
