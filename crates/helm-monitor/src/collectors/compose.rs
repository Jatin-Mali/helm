//! Docker Compose project collector.
//! Groups containers by com.docker.compose.project label and provides project-level aggregates.

use crate::{
    collectors::{Collector, bin_exists, err, run_timed},
    snapshot::MonitorProfile,
};
use serde::{Deserialize, Serialize};

/// Docker Compose snapshot: project-level aggregates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ComposeSnapshot {
    pub projects: Vec<ComposeProject>,
    pub total_container_count: usize,
    pub available: bool,
}

/// Docker Compose project with container aggregates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeProject {
    pub name: String,
    pub status: String, // "healthy", "degraded", "down"
    pub container_count: u32,
    pub running_count: u32,
    pub unhealthy_count: u32,
}

/// Collector for Docker Compose projects.
#[derive(Default)]
pub struct ComposeCollector;

impl Collector for ComposeCollector {
    type Output = ComposeSnapshot;

    fn domain(&self) -> &'static str {
        "compose"
    }

    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        // Check if docker is available
        if !bin_exists("docker") {
            return Ok(ComposeSnapshot {
                projects: Vec::new(),
                total_container_count: 0,
                available: false,
            });
        }

        // Run docker ps to get all containers with labels
        match run_timed(
            "docker",
            &[
                "ps",
                "-a",
                "--filter",
                "label=com.docker.compose.project",
                "--format",
                "{{json .}}",
            ],
            profile,
        )
        .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let projects = parse_compose_projects(&stdout);
                let total_container_count =
                    projects.iter().map(|p| p.container_count as usize).sum();
                Ok(ComposeSnapshot {
                    projects,
                    total_container_count,
                    available: true,
                })
            }
            Err(e) => Err(err("compose", e.message)),
        }
    }
}

/// Parse docker ps JSON output and group containers by compose project label.
fn parse_compose_projects(output: &str) -> Vec<ComposeProject> {
    use std::collections::HashMap;

    #[derive(Debug, Deserialize)]
    struct DockerContainer {
        #[serde(rename = "ID")]
        _id: String,
        #[serde(rename = "Names")]
        _names: String,
        #[serde(rename = "State")]
        state: String,
        #[serde(rename = "Status")]
        status: String,
        #[serde(rename = "Labels")]
        labels: Option<String>,
    }

    let mut projects: HashMap<String, Vec<DockerContainer>> = HashMap::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match serde_json::from_str::<DockerContainer>(line) {
            Ok(container) => {
                // Extract project name from labels
                if let Some(labels_str) = &container.labels {
                    // Parse labels: "key1=value1,key2=value2"
                    let project_name = labels_str.split(',').find_map(|kv| {
                        let parts: Vec<&str> = kv.splitn(2, '=').collect();
                        if parts.len() == 2 && parts[0] == "com.docker.compose.project" {
                            Some(parts[1].to_string())
                        } else {
                            None
                        }
                    });

                    if let Some(proj_name) = project_name {
                        projects.entry(proj_name).or_default().push(container);
                    }
                }
            }
            Err(_) => {
                // Skip malformed JSON lines
                continue;
            }
        }
    }

    // Aggregate containers by project
    let mut result: Vec<ComposeProject> = projects
        .into_iter()
        .map(|(name, containers)| {
            let total = containers.len() as u32;
            let running = containers
                .iter()
                .filter(|c| c.state.to_lowercase() == "running")
                .count() as u32;
            let unhealthy = containers
                .iter()
                .filter(|c| {
                    c.status.to_lowercase().contains("unhealthy")
                        || c.status.to_lowercase().contains("exited")
                })
                .count() as u32;

            let status = if total == 0 {
                "down".to_string()
            } else if running == total && unhealthy == 0 {
                "healthy".to_string()
            } else if running > 0 {
                "degraded".to_string()
            } else {
                "down".to_string()
            };

            ComposeProject {
                name,
                status,
                container_count: total,
                running_count: running,
                unhealthy_count: unhealthy,
            }
        })
        .collect();

    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_compose_empty() {
        let result = parse_compose_projects("");
        assert_eq!(result, Vec::new());
    }

    #[test]
    fn parse_compose_single_project() {
        let json = r#"{"ID":"abc123","Names":"test_app_1","State":"running","Status":"Up 2 hours","Labels":"com.docker.compose.project=myapp"}"#;
        let result = parse_compose_projects(json);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "myapp");
        assert_eq!(result[0].container_count, 1);
        assert_eq!(result[0].running_count, 1);
        assert_eq!(result[0].unhealthy_count, 0);
        assert_eq!(result[0].status, "healthy");
    }

    #[test]
    fn parse_compose_status_calculation() {
        let json = r#"{"ID":"abc1","Names":"test_1","State":"running","Status":"Up 2 hours","Labels":"com.docker.compose.project=myapp"}
{"ID":"abc2","Names":"test_2","State":"exited","Status":"Exited (1) 10 seconds ago","Labels":"com.docker.compose.project=myapp"}"#;
        let result = parse_compose_projects(json);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "myapp");
        assert_eq!(result[0].container_count, 2);
        assert_eq!(result[0].running_count, 1);
        assert_eq!(result[0].unhealthy_count, 1);
        assert_eq!(result[0].status, "degraded");
    }

    #[test]
    fn parse_compose_multiple_projects() {
        let json = r#"{"ID":"abc1","Names":"app1_web_1","State":"running","Status":"Up 2 hours","Labels":"com.docker.compose.project=app1"}
{"ID":"abc2","Names":"app2_web_1","State":"running","Status":"Up 1 hour","Labels":"com.docker.compose.project=app2"}
{"ID":"abc3","Names":"app1_db_1","State":"running","Status":"Up 2 hours","Labels":"com.docker.compose.project=app1"}"#;
        let result = parse_compose_projects(json);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "app1");
        assert_eq!(result[0].container_count, 2);
        assert_eq!(result[1].name, "app2");
        assert_eq!(result[1].container_count, 1);
    }

    #[test]
    fn parse_compose_no_label() {
        let json = r#"{"ID":"abc1","Names":"orphan_container","State":"running","Status":"Up 2 hours","Labels":null}"#;
        let result = parse_compose_projects(json);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn parse_compose_all_stopped() {
        let json = r#"{"ID":"abc1","Names":"test_1","State":"exited","Status":"Exited (0) 1 hour ago","Labels":"com.docker.compose.project=myapp"}
{"ID":"abc2","Names":"test_2","State":"exited","Status":"Exited (0) 2 hours ago","Labels":"com.docker.compose.project=myapp"}"#;
        let result = parse_compose_projects(json);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, "down");
        assert_eq!(result[0].running_count, 0);
    }
}
