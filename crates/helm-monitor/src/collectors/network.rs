//! Network collector: routes, interfaces, DNS.
use crate::{
    collectors::{Collector, read_proc, run_timed},
    snapshot::{InterfaceEntry, MonitorProfile, NetworkSnapshot, RouteEntry},
};
#[derive(Default)]
pub struct NetworkCollector;
impl Collector for NetworkCollector {
    type Output = NetworkSnapshot;
    fn domain(&self) -> &'static str {
        "network"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = NetworkSnapshot::default();
        if let Ok(o) = run_timed("ip", &["route"], profile).await {
            out.routes = parse_routes(&String::from_utf8_lossy(&o.stdout));
        }
        if let Ok(o) = run_timed("ip", &["-br", "addr"], profile).await {
            out.interfaces = parse_addr(&String::from_utf8_lossy(&o.stdout));
        }
        if let Ok(c) = read_proc("/etc/resolv.conf", profile.per_collector_timeout()).await {
            for l in c.lines() {
                if let Some(ns) = l.trim().strip_prefix("nameserver") {
                    let a = ns.trim();
                    if !a.is_empty() {
                        out.nameservers.push(a.into());
                    }
                }
            }
        }
        Ok(out)
    }
}
fn parse_routes(s: &str) -> Vec<RouteEntry> {
    s.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            let mut e = RouteEntry {
                destination: p.first().unwrap_or(&"").to_string(),
                gateway: None,
                interface: None,
            };
            for i in 0..p.len() {
                if p[i] == "via" {
                    e.gateway = p.get(i + 1).map(|s| s.to_string());
                }
                if p[i] == "dev" {
                    e.interface = p.get(i + 1).map(|s| s.to_string());
                }
            }
            e
        })
        .collect()
}
fn parse_addr(s: &str) -> Vec<InterfaceEntry> {
    s.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            InterfaceEntry {
                name: p.first().unwrap_or(&"").to_string(),
                state: if l.contains("UP") { "UP" } else { "DOWN" }.into(),
                addresses: p
                    .iter()
                    .filter(|s| s.contains('/') || s.contains(':'))
                    .map(|s| s.to_string())
                    .collect(),
            }
        })
        .collect()
}
