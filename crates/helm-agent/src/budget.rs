//! ReAct run budget and consumption tracker.

use std::time::{Duration, Instant};

pub use helm_core::BudgetError;

/// Hard limits applied to a single HELM ReAct run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Maximum provider turns allowed before the run stops partially.
    pub max_iterations: u32,
    /// Maximum total provider input tokens allowed.
    pub max_input_tokens: u32,
    /// Maximum total provider output tokens allowed.
    pub max_output_tokens: u32,
    /// Maximum wall-clock runtime allowed.
    pub max_wall_time: Duration,
    /// Whether to auto-approve all capability requests (dangerously skip permissions).
    pub auto_approve: bool,
    /// Whether to permanently deny all write capabilities (plan mode).
    pub read_only: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_iterations: 20,
            max_input_tokens: 200_000,
            max_output_tokens: 50_000,
            max_wall_time: Duration::from_secs(5 * 60),
            auto_approve: false,
            read_only: false,
        }
    }
}

/// Mutable budget accounting for one active ReAct run.
#[derive(Debug, Clone)]
pub struct BudgetTracker {
    budget: Budget,
    started_at: Instant,
    iterations: u32,
    input_tokens: u32,
    output_tokens: u32,
}

impl BudgetTracker {
    /// Creates a tracker for `budget` starting at the current instant.
    pub fn new(budget: Budget) -> Self {
        Self {
            budget,
            started_at: Instant::now(),
            iterations: 0,
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    /// Returns an error if any budget limit has been reached.
    pub fn check(&self) -> Result<(), BudgetError> {
        if self.iterations >= self.budget.max_iterations {
            return Err(BudgetError::MaxIterations);
        }
        if self.input_tokens >= self.budget.max_input_tokens {
            return Err(BudgetError::MaxInputTokens);
        }
        if self.output_tokens >= self.budget.max_output_tokens {
            return Err(BudgetError::MaxOutputTokens);
        }
        if self.started_at.elapsed() >= self.budget.max_wall_time {
            return Err(BudgetError::WallTimeout);
        }
        Ok(())
    }

    /// Records one provider turn.
    pub fn record_iteration(&mut self) {
        self.iterations = self.iterations.saturating_add(1);
    }

    /// Records provider token usage for budget checks.
    pub fn record_tokens(&mut self, input_tokens: u32, output_tokens: u32) {
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(output_tokens);
    }

    /// Returns recorded provider iterations.
    pub fn iterations(&self) -> u32 {
        self.iterations
    }

    /// Returns recorded provider input tokens.
    pub fn input_tokens(&self) -> u32 {
        self.input_tokens
    }

    /// Returns recorded provider output tokens.
    pub fn output_tokens(&self) -> u32 {
        self.output_tokens
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Budget, BudgetError, BudgetTracker};

    #[test]
    fn default_budget_happy_path() {
        let budget = Budget::default();

        assert_eq!(budget.max_iterations, 20);
        assert_eq!(budget.max_wall_time, Duration::from_secs(300));
    }

    #[test]
    fn detects_iteration_limit_error_path() {
        let budget = Budget {
            max_iterations: 1,
            ..Budget::default()
        };
        let mut tracker = BudgetTracker::new(budget);
        tracker.record_iteration();

        assert_eq!(tracker.check().unwrap_err(), BudgetError::MaxIterations);
    }

    #[test]
    fn saturating_token_accounting_edge_case() {
        let mut tracker = BudgetTracker::new(Budget::default());
        tracker.record_tokens(u32::MAX, u32::MAX);
        tracker.record_tokens(1, 1);

        assert_eq!(tracker.input_tokens(), u32::MAX);
        assert_eq!(tracker.output_tokens(), u32::MAX);
    }

    #[test]
    fn wall_time_limit_can_expire() {
        let budget = Budget {
            max_wall_time: Duration::from_nanos(0),
            ..Budget::default()
        };
        let tracker = BudgetTracker::new(budget);

        assert_eq!(tracker.check().unwrap_err(), BudgetError::WallTimeout);
    }
}
