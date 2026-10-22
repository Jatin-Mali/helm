//! Database tooling availability. Instances are never probed implicitly; configured
//! databases are checked through the service registry.

use crate::{
    collectors::{Collector, run_timed},
    snapshot::MonitorProfile,
};

/// Result of probing a database instance.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DbInstance {
    /// Database type: "postgresql" or "mysql"
    pub db_type: String,
    /// Instance identifier (socket path or host:port)
    pub connection: String,
    /// Whether the instance is reachable
    pub reachable: bool,
    /// Version string if available
    pub version: Option<String>,
    /// Whether replication is lagging (if applicable)
    pub replication_lag_seconds: Option<f64>,
    /// Database count
    pub database_count: Option<u32>,
    /// Active connections
    pub active_connections: Option<u32>,
    /// Error message if probe failed
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct DbHealthSnapshot {
    pub instances: Vec<DbInstance>,
    pub psql_available: bool,
    pub mysql_available: bool,
}

#[derive(Default)]
pub struct DbHealthCollector;

impl Collector for DbHealthCollector {
    type Output = DbHealthSnapshot;

    fn domain(&self) -> &'static str {
        "db-health"
    }

    #[allow(clippy::field_reassign_with_default)]
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut snapshot = DbHealthSnapshot::default();

        // Check tool availability
        snapshot.psql_available = run_timed("which", &["psql"], profile)
            .await
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);
        snapshot.mysql_available = run_timed("which", &["mysql"], profile)
            .await
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);

        // No implicit probing: guessing credentials/sockets (e.g. `psql -U postgres`) is
        // forbidden. Explicit targets are registered services (TCP health, no credentials).
        Ok(snapshot)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn collector_never_probes_databases_implicitly() {
        let snapshot = DbHealthCollector
            .collect(MonitorProfile::Quick)
            .await
            .expect("collect");
        assert!(snapshot.instances.is_empty());
    }
}
