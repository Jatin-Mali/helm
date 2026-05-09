//! Workspace-level test host for HELM integration tests.

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_test_host_loads() {
        assert_eq!(2 + 2, 4);
    }
}
