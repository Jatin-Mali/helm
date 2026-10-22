//! Containers collector: Docker/Podman.
use crate::{
    collectors::{Collector, bin_exists, err, run_timed},
    snapshot::{ContainerInfo, ContainerRuntime, ContainerSnapshot, MonitorProfile},
};
#[derive(Default)]
pub struct ContainersCollector;
impl Collector for ContainersCollector {
    type Output = ContainerSnapshot;
    fn domain(&self) -> &'static str {
        "containers"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = ContainerSnapshot::default();
        let rt = if bin_exists("docker") {
            Some(ContainerRuntime::Docker)
        } else if bin_exists("podman") {
            Some(ContainerRuntime::Podman)
        } else {
            return Ok(out);
        };
        out.runtime = rt;
        let cmd = match rt {
            Some(ContainerRuntime::Docker) => "docker",
            _ => "podman",
        };
        match run_timed(
            cmd,
            &["ps", "-a", "--no-trunc", "--format", "{{json .}}"],
            profile,
        )
        .await
        {
            Ok(o) => out.containers = parse_list(&String::from_utf8_lossy(&o.stdout)),
            Err(e) => return Err(err("containers", e.message)),
        }
        if !out.containers.is_empty() {
            let mut args = vec!["inspect", "--format", "{{.Id}} {{.RestartCount}}"];
            args.extend(out.containers.iter().map(|c| c.id.as_str()));
            // A failed inspect is recorded by run_timed; counts stay unknown rather than zero.
            if let Ok(o) = run_timed(cmd, &args, profile).await {
                apply_restart_counts(&mut out.containers, &String::from_utf8_lossy(&o.stdout));
            }
        }
        Ok(out)
    }
}
fn parse_list(s: &str) -> Vec<ContainerInfo> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            #[derive(serde::Deserialize)]
            struct DockerPsRow {
                #[serde(rename = "ID")]
                id: String,
                #[serde(rename = "Names")]
                names: String,
                #[serde(rename = "Image")]
                image: String,
                #[serde(rename = "Status")]
                status: String,
                #[serde(rename = "Ports")]
                ports: Option<String>,
                #[serde(rename = "Mounts")]
                mounts: Option<String>,
            }

            let row = serde_json::from_str::<DockerPsRow>(l).ok()?;
            Some(ContainerInfo {
                id: row.id,
                name: row.names,
                image: row.image,
                status: row.status.clone(),
                ports: row
                    .ports
                    .as_deref()
                    .map(split_csv_field)
                    .unwrap_or_default(),
                mounts: row
                    .mounts
                    .as_deref()
                    .map(split_csv_field)
                    .unwrap_or_default(),
                restart_count: None,
                health: parse_health_status(&row.status),
            })
        })
        .collect()
}

fn split_csv_field(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// `docker ps` shows the last exit code in "Restarting (N)", not a restart count;
/// counts come from `inspect` output lines of "<full id> <count>".
fn apply_restart_counts(containers: &mut [ContainerInfo], inspect: &str) {
    for line in inspect.lines() {
        let Some((id, count)) = line.trim().split_once(' ') else {
            continue;
        };
        let Ok(count) = count.trim().parse::<u32>() else {
            continue;
        };
        if let Some(c) = containers
            .iter_mut()
            .find(|c| c.id == id || id.starts_with(c.id.as_str()))
        {
            c.restart_count = Some(count);
        }
    }
}

fn parse_health_status(status: &str) -> Option<String> {
    if status.contains("(healthy)") {
        Some("healthy".to_owned())
    } else if status.contains("(unhealthy)") {
        Some("unhealthy".to_owned())
    } else if status.contains("(health: starting)") {
        Some("starting".to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_does_not_confuse_exit_code_with_restart_count() {
        let json = r#"{"ID":"abc","Names":"web","Image":"nginx:latest","Status":"Restarting (7) 12 seconds ago (unhealthy)","Ports":"80/tcp, 443/tcp","Mounts":"/srv/www"}"#;
        let containers = parse_list(json);
        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].name, "web");
        assert_eq!(containers[0].restart_count, None);
        assert_eq!(containers[0].health.as_deref(), Some("unhealthy"));
        assert_eq!(containers[0].ports.len(), 2);
    }

    #[test]
    fn parse_list_handles_healthy_container() {
        let json = r#"{"ID":"def","Names":"api","Image":"my-api:v1","Status":"Up 5 minutes (healthy)","Ports":"","Mounts":""}"#;
        let containers = parse_list(json);
        assert_eq!(containers[0].health.as_deref(), Some("healthy"));
        assert_eq!(containers[0].restart_count, None);
    }

    #[test]
    fn inspect_counts_apply_by_id_and_malformed_lines_stay_unknown() {
        let json = r#"{"ID":"abc123","Names":"a","Image":"i","Status":"Up"}
{"ID":"def456","Names":"b","Image":"i","Status":"Up"}"#;
        let mut containers = parse_list(json);
        apply_restart_counts(&mut containers, "abc123ffff 5\ndef456 notanumber\ngarbage\n");
        assert_eq!(containers[0].restart_count, Some(5));
        assert_eq!(containers[1].restart_count, None);
    }
}
