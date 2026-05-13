//! ReAct run budget and consumption tracker.

use std::time::{Duration, Instant};

pub use helm_core::BudgetError;

/// Tracks cost in USD with warning threshold.
#[derive(Debug, Clone, Copy)]
pub struct CostBudget {
    /// Maximum cost in USD for this run.
    pub limit_usd: f64,
    /// Warning threshold (0.0-1.0, e.g., 0.8 = warn at 80% of limit).
    pub warn_threshold: f64,
    /// Amount spent so far in USD.
    pub spent_usd: f64,
}

impl CostBudget {
    /// Creates a new cost budget with default 80% warning threshold.
    pub fn new(limit_usd: f64) -> Self {
        Self {
            limit_usd,
            warn_threshold: 0.8,
            spent_usd: 0.0,
        }
    }

    /// Records a cost transaction.
    pub fn record_cost(&mut self, cost_usd: f64) {
        self.spent_usd = (self.spent_usd + cost_usd).min(f64::MAX);
    }

    /// Remaining budget.
    pub fn remaining(&self) -> f64 {
        (self.limit_usd - self.spent_usd).max(0.0)
    }

    /// Current spending as a fraction of the limit (0.0-1.0+).
    pub fn utilization(&self) -> f64 {
        self.spent_usd / self.limit_usd.max(0.01)
    }

    /// Status of the budget.
    pub fn status(&self) -> BudgetStatus {
        if self.spent_usd >= self.limit_usd {
            BudgetStatus::Exceeded
        } else if self.utilization() >= self.warn_threshold {
            BudgetStatus::Warning {
                pct: (self.utilization() * 100.0).round() as u32,
            }
        } else {
            BudgetStatus::Ok
        }
    }
}

/// Status of a cost budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    /// Normal operation.
    Ok,
    /// Warning threshold reached (e.g., 80% of limit).
    Warning { pct: u32 },
    /// Budget limit exceeded.
    Exceeded,
}

/// Hard limits applied to a single HELM ReAct run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// Maximum provider turns allowed before the run stops partially.
    pub max_iterations: u32,
    /// Maximum total provider input tokens allowed.
    pub max_input_tokens: u32,
    /// Maximum total provider output tokens allowed.
    pub max_output_tokens: u32,
    /// Maximum wall-clock runtime allowed.
    pub max_wall_time: Duration,
    /// Maximum total cost allowed in USD.
    pub max_cost_usd: Option<f64>,
    /// Whether to auto-approve all capability requests (dangerously skip permissions).
    pub auto_approve: bool,
    /// Whether to permanently deny all write capabilities (plan mode).
    pub read_only: bool,
    /// When true, tool calls return synthetic success without executing.
    pub dry_run: bool,
    /// When true, emit an evidence snapshot before each permission check.
    pub require_evidence: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_iterations: 20,
            max_input_tokens: 200_000,
            max_output_tokens: 50_000,
            max_wall_time: Duration::from_secs(5 * 60),
            max_cost_usd: None,
            auto_approve: false,
            read_only: false,
            dry_run: false,
            require_evidence: false,
        }
    }
}

/// Mutable budget accounting for one active HELM ReAct run.
#[derive(Debug, Clone)]
pub struct BudgetTracker {
    budget: Budget,
    started_at: Instant,
    iterations: u32,
    input_tokens: u32,
    output_tokens: u32,
    cost_usd: f64,
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
            cost_usd: 0.0,
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
        if let Some(max_cost) = self.budget.max_cost_usd {
            if self.cost_usd >= max_cost {
                return Err(BudgetError::CostLimitExceeded);
            }
        }
        Ok(())
    }

    /// Checks cost status and returns warning if near limit.
    pub fn cost_status(&self) -> BudgetStatus {
        if let Some(max_cost) = self.budget.max_cost_usd {
            if max_cost > 0.0 {
                let pct = (self.cost_usd / max_cost * 100.0).round() as u32;
                if self.cost_usd >= max_cost {
                    BudgetStatus::Exceeded
                } else if pct >= 80 {
                    BudgetStatus::Warning { pct }
                } else {
                    BudgetStatus::Ok
                }
            } else {
                BudgetStatus::Ok
            }
        } else {
            BudgetStatus::Ok
        }
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

    /// Records cost in USD based on token usage and pricing model.
    pub fn record_cost(
        &mut self,
        input_tokens: u32,
        output_tokens: u32,
        input_rate: f64,
        output_rate: f64,
    ) {
        let input_cost = (input_tokens as f64) * input_rate / 1_000_000.0;
        let output_cost = (output_tokens as f64) * output_rate / 1_000_000.0;
        self.cost_usd += input_cost + output_cost;
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

    /// Returns recorded cost in USD.
    pub fn cost_usd(&self) -> f64 {
        self.cost_usd
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Budget, BudgetError, BudgetStatus, BudgetTracker, CostBudget};

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

    // ── Cost budget tests ──────────────────────────────────────────────────

    #[test]
    fn cost_budget_ok_status() {
        let budget = CostBudget::new(10.0);
        assert_eq!(budget.status(), BudgetStatus::Ok);
    }

    #[test]
    fn cost_budget_warn_at_threshold() {
        let mut budget = CostBudget::new(10.0);
        budget.record_cost(8.5);
        match budget.status() {
            BudgetStatus::Warning { pct } => assert!(pct >= 85),
            _ => panic!("Expected warning status"),
        }
    }

    #[test]
    fn cost_budget_exceeded_stops() {
        let mut budget = CostBudget::new(10.0);
        budget.record_cost(11.0);
        assert_eq!(budget.status(), BudgetStatus::Exceeded);
    }

    #[test]
    fn cost_budget_remaining_calculation() {
        let mut budget = CostBudget::new(10.0);
        budget.record_cost(3.0);
        assert!((budget.remaining() - 7.0).abs() < 0.01);
    }

    #[test]
    fn cost_budget_utilization_ratio() {
        let mut budget = CostBudget::new(10.0);
        budget.record_cost(2.5);
        assert!((budget.utilization() - 0.25).abs() < 0.01);
    }
}
