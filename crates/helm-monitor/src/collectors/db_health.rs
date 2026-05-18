//! Database health collector: probes PostgreSQL and MySQL/MariaDB.
//! Uses respective CLI tools for non-invasive health checks.

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

        // Probe PostgreSQL instances
        if snapshot.psql_available {
            if let Some(instance) = probe_postgresql(profile).await {
                snapshot.instances.push(instance);
            }
        }

        // Probe MySQL instances
        if snapshot.mysql_available {
            if let Some(instance) = probe_mysql(profile).await {
                snapshot.instances.push(instance);
            }
        }

        Ok(snapshot)
    }
}

async fn probe_postgresql(profile: MonitorProfile) -> Option<DbInstance> {
    let result = run_timed(
        "psql",
        &[
            "-U",
            "postgres",
            "-c",
            "SELECT version(), count(*) FROM pg_stat_activity;",
            "-t",
            "-A",
        ],
        profile,
    )
    .await;

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut version = None;
            let mut connections = None;
            for line in stdout.lines() {
                if let Some(ver) = line.split('|').next() {
                    version = Some(ver.trim().to_string());
                }
                if let Some(conn_str) = line.split('|').nth(1) {
                    connections = conn_str.trim().parse().ok();
                }
            }
            Some(DbInstance {
                db_type: "postgresql".into(),
                connection: "local socket".into(),
                reachable: true,
                version,
                replication_lag_seconds: None,
                database_count: None,
                active_connections: connections,
                error: None,
            })
        }
        Err(e) => Some(DbInstance {
            db_type: "postgresql".into(),
            connection: "local socket".into(),
            reachable: false,
            version: None,
            replication_lag_seconds: None,
            database_count: None,
            active_connections: None,
            error: Some(e.message),
        }),
    }
}

async fn probe_mysql(profile: MonitorProfile) -> Option<DbInstance> {
    let result = run_timed(
        "mysql",
        &[
            "-e",
            "SELECT VERSION(), COUNT(*) FROM information_schema.PROCESSLIST;",
            "-N",
        ],
        profile,
    )
    .await;

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut version = None;
            let mut connections = None;
            for line in stdout.lines() {
                let parts: Vec<&str> = line.trim().split('\t').collect();
                if let Some(ver) = parts.first() {
                    version = Some(ver.trim().to_string());
                }
                if let Some(conn) = parts.get(1) {
                    connections = conn.trim().parse().ok();
                }
            }
            Some(DbInstance {
                db_type: "mysql".into(),
                connection: "local socket".into(),
                reachable: true,
                version,
                replication_lag_seconds: None,
                database_count: None,
                active_connections: connections,
                error: None,
            })
        }
        Err(e) => Some(DbInstance {
            db_type: "mysql".into(),
            connection: "local socket".into(),
            reachable: false,
            version: None,
            replication_lag_seconds: None,
            database_count: None,
            active_connections: None,
            error: Some(e.message),
        }),
    }
}
