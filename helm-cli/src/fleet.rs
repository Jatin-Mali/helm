//! `monitor --fleet`: real SSH observations with host identity and freshness.
//! An unreachable host is reported unknown (with its last answer labeled stale), never local data.

use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::remote::{FleetHostReport, RemoteEntry, RemoteRegistry};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LastKnown {
    pub name: String,
    pub host: String,
    pub identity: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub critical: usize,
    pub warning: usize,
    pub info: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FleetState {
    pub hosts: BTreeMap<String, LastKnown>,
}

impl FleetState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
    }
}

fn summarize(entry: &RemoteEntry, report: &FleetHostReport) -> LastKnown {
    use helm_monitor::Severity;
    let count = |severity: Severity| {
        report
            .findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    };
    LastKnown {
        name: entry.name.clone(),
        host: entry.host.clone(),
        identity: report
            .snapshot
            .as_ref()
            .and_then(|s| s.domains.collection.host_id.clone()),
        observed_at: report.observed_at,
        critical: count(Severity::Critical),
        warning: count(Severity::Warning),
        info: count(Severity::Info),
    }
}

/// Folds collection results into state. Returns one JSON row and one text block per remote.
pub fn apply_results(
    registry: &RemoteRegistry,
    results: Vec<(uuid::Uuid, Result<FleetHostReport>)>,
    state: &mut FleetState,
) -> Vec<(serde_json::Value, String)> {
    let mut seen_identity: BTreeMap<String, String> = BTreeMap::new();
    let mut rows = Vec::new();
    for entry in &registry.remotes {
        // Keyed by name: legacy registry entries get a fresh host_id on every load.
        let key = entry.name.clone();
        let result = results.iter().find(|(id, _)| *id == entry.host_id);
        let row = match result {
            Some((_, Ok(report))) => {
                let mut fresh = summarize(entry, report);
                let mut warning = None;
                if let Some(id) = fresh.identity.clone()
                    && let Some(other) = seen_identity.insert(id, entry.name.clone())
                {
                    warning = Some(format!("same machine identity as '{other}'"));
                }
                if warning.is_some() {
                    // Identical identities mean two registrations reach one machine; do not
                    // present them as independent observations.
                    fresh.identity = None;
                }
                state.hosts.insert(key, fresh.clone());
                let mut text = format!(
                    ">>> {} ({})  identity={}\n  observed {}\n  Findings: {} critical, {} warning, {} info",
                    entry.name,
                    entry.host,
                    fresh
                        .identity
                        .as_deref()
                        .map_or("unknown".into(), |i| i[..i.len().min(12)].to_string()),
                    fresh.observed_at.to_rfc3339(),
                    fresh.critical,
                    fresh.warning,
                    fresh.info
                );
                if let Some(w) = &warning {
                    text.push_str(&format!("\n  WARNING: {w}"));
                }
                (
                    serde_json::json!({"host": entry.name, "status": "observed", "warning": warning, "last_known": fresh}),
                    text,
                )
            }
            other => {
                let error = match other {
                    Some((_, Err(e))) => e.to_string(),
                    _ => "no collection result".to_owned(),
                };
                let stale = state.hosts.get(&key).cloned();
                let mut text = format!(
                    ">>> {} ({})\n  UNKNOWN — unavailable: {}",
                    entry.name,
                    entry.host,
                    error.lines().next().unwrap_or_default()
                );
                if let Some(s) = &stale {
                    text.push_str(&format!(
                        "\n  last known (STALE, observed {}): {} critical, {} warning, {} info",
                        s.observed_at.to_rfc3339(),
                        s.critical,
                        s.warning,
                        s.info
                    ));
                }
                (
                    serde_json::json!({"host": entry.name, "status": "unavailable", "error": error, "stale_last_known": stale}),
                    text,
                )
            }
        };
        rows.push(row);
    }
    rows
}

pub async fn run(json: bool) -> Result<()> {
    let registry = RemoteRegistry::load()?;
    if registry.remotes.is_empty() {
        if json {
            println!("[]");
        }
        eprintln!("No remote hosts configured. Use `helm remote add <name> <host>` first.");
        return Ok(());
    }
    let path = crate::paths::data_file("fleet-state.json");
    let mut state = FleetState::load(&path);
    let results = registry.parallel_collect_reports().await;
    let rows = apply_results(&registry, results, &mut state);
    state
        .save(&path)
        .context("failed to persist fleet observations")?;
    if json {
        let rows: Vec<_> = rows.into_iter().map(|(json, _)| json).collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!("Fleet Monitor — {} host(s)", registry.remotes.len());
        for (_, text) in rows {
            println!("\n{text}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{Credential, RemoteTransport};

    fn entry(name: &str) -> RemoteEntry {
        RemoteEntry {
            host_id: uuid::Uuid::new_v4(),
            name: name.into(),
            host: format!("{name}.invalid"),
            port: 22,
            user: None,
            ssh_opts: None,
            credential: Credential::SshAgent,
            transport: RemoteTransport::Ssh,
            agent: None,
        }
    }

    fn report(entry: &RemoteEntry) -> FleetHostReport {
        FleetHostReport {
            host_id: entry.host_id,
            host_name: entry.name.clone(),
            findings: Vec::new(),
            snapshot: None,
            observed_at: Utc::now(),
        }
    }

    #[test]
    fn unreachable_host_is_unknown_with_labeled_stale_data_not_local_results() {
        let (a, b) = (entry("a"), entry("b"));
        let registry = RemoteRegistry {
            remotes: vec![a.clone(), b.clone()],
        };
        let mut state = FleetState::default();
        apply_results(
            &registry,
            vec![(a.host_id, Ok(report(&a))), (b.host_id, Ok(report(&b)))],
            &mut state,
        );
        let rows = apply_results(
            &registry,
            vec![
                (a.host_id, Ok(report(&a))),
                (b.host_id, Err(anyhow::anyhow!("ssh timed out"))),
            ],
            &mut state,
        );
        assert_eq!(rows[0].0["status"], "observed");
        assert_eq!(rows[1].0["status"], "unavailable");
        assert!(rows[1].1.contains("UNKNOWN") && rows[1].1.contains("STALE"));
        assert!(rows[1].0["stale_last_known"]["observed_at"].is_string());
    }

    #[test]
    fn never_observed_unreachable_host_has_no_invented_data() {
        let a = entry("a");
        let registry = RemoteRegistry {
            remotes: vec![a.clone()],
        };
        let rows = apply_results(&registry, Vec::new(), &mut FleetState::default());
        assert_eq!(rows[0].0["status"], "unavailable");
        assert!(rows[0].0["stale_last_known"].is_null());
    }

    #[test]
    fn state_round_trips_through_atomic_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fleet-state.json");
        let a = entry("a");
        let mut state = FleetState::default();
        state
            .hosts
            .insert(a.name.clone(), summarize(&a, &report(&a)));
        state.save(&path).unwrap();
        assert_eq!(FleetState::load(&path).hosts.len(), 1);
    }
}
