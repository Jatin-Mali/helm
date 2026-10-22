//! Immutable operation policy shared by command and UI mutation paths.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecutionPolicy {
    pub read_only: bool,
    pub dry_run: bool,
    pub sandbox_requested: bool,
}

impl ExecutionPolicy {
    /// `false` means render a preview; it never authorizes a process launch.
    pub fn check_mutation(self) -> Result<bool, String> {
        if self.sandbox_requested {
            return Err("sandboxed operations are unavailable; refusing host execution".into());
        }
        if self.dry_run {
            return Ok(false);
        }
        if self.read_only {
            return Err("read-only mode forbids this mutation".into());
        }
        Ok(true)
    }
}
