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
            &[
                "ps",
                "-a",
                "--no-trunc",
                "--format",
                "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}\t{{.Mounts}}",
            ],
            profile,
        )
        .await
        {
            Ok(o) => out.containers = parse_list(&String::from_utf8_lossy(&o.stdout)),
            Err(e) => return Err(err("containers", e.message)),
        }
        Ok(out)
    }
}
fn parse_list(s: &str) -> Vec<ContainerInfo> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let p: Vec<&str> = l.split('\t').collect();
            if p.len() < 4 {
                None
            } else {
                Some(ContainerInfo {
                    id: p[0].into(),
                    name: p[1].into(),
                    image: p[2].into(),
                    status: p[3].into(),
                    ports: p
                        .get(4)
                        .map(|s| s.split(',').map(|x| x.trim().into()).collect())
                        .unwrap_or_default(),
                    mounts: p
                        .get(5)
                        .map(|s| s.split(',').map(|x| x.trim().into()).collect())
                        .unwrap_or_default(),
                    restart_count: None,
                    health: None,
                })
            }
        })
        .collect()
}
