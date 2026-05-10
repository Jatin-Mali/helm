//! Cooperative cancellation token for agent runs.
//!
//! Create a token, call `.child()` to share it with the agent, and call
//! `.cancel()` from a signal handler or TUI Ctrl+C handler to stop the loop.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// A cloneable cancellation handle backed by a shared atomic flag.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Signal cancellation.  Affects all clones that share this token.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Returns true if `cancel()` has been called on any clone.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Create a clone that shares the same cancellation state.
    pub fn child(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::CancellationToken;

    #[test]
    fn new_token_is_not_cancelled_happy_path() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
    }

    #[test]
    fn cancel_sets_flag_happy_path() {
        let t = CancellationToken::new();
        t.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn child_shares_cancellation_state_happy_path() {
        let parent = CancellationToken::new();
        let child = parent.child();
        parent.cancel();
        assert!(child.is_cancelled());
    }

    #[test]
    fn child_cancel_propagates_to_parent_happy_path() {
        let parent = CancellationToken::new();
        let child = parent.child();
        child.cancel();
        assert!(parent.is_cancelled());
    }

    #[test]
    fn default_token_is_not_cancelled_edge_case() {
        let t = CancellationToken::default();
        assert!(!t.is_cancelled());
    }
}
