//! Ports collector: ss -tulpn.
use crate::{
    collectors::{Collector, err, run_timed},
    snapshot::{ListenerEntry, MonitorProfile, PortSnapshot},
};
#[derive(Default)]
pub struct PortsCollector;
impl Collector for PortsCollector {
    type Output = PortSnapshot;
    fn domain(&self) -> &'static str {
        "ports"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        match run_timed("ss", &["-tulpn"], profile).await {
            Ok(o) => Ok(PortSnapshot {
                listeners: parse_ss(&String::from_utf8_lossy(&o.stdout)),
            }),
            Err(e) => Err(err("ports", e.message)),
        }
    }
}
fn parse_ss(s: &str) -> Vec<ListenerEntry> {
    s.lines()
        .skip(1)
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 5 {
                None
            } else {
                let (a, po) = parse_addr(p[4]);
                let (pr, pi) = parse_proc(p.get(6).unwrap_or(&""));
                Some(ListenerEntry {
                    protocol: p[0].to_lowercase(),
                    local_address: a,
                    local_port: po,
                    process_name: pr,
                    pid: pi,
                })
            }
        })
        .collect()
}
fn parse_addr(local: &str) -> (String, u16) {
    if let Some(pos) = local.rfind(':') {
        let a = &local[..pos];
        let port = local[pos + 1..].parse().unwrap_or(0);
        (a.trim_start_matches('[').trim_end_matches(']').into(), port)
    } else {
        (local.into(), 0)
    }
}
fn parse_proc(info: &str) -> (Option<String>, Option<u32>) {
    let inner = info
        .strip_prefix("users:((")
        .and_then(|s| s.strip_suffix(')'));
    if let Some(inner) = inner {
        let parts: Vec<&str> = inner.split(',').collect();
        let name = parts.first().map(|s| s.trim_matches('"').into());
        let pid = parts
            .iter()
            .find_map(|p| p.trim().strip_prefix("pid=")?.parse::<u32>().ok());
        (name, pid)
    } else {
        (None, None)
    }
}
