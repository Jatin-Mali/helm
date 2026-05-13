//! Load collector: CPU, memory, swap, PSI from /proc.
use crate::{
    collectors::{Collector, read_proc, timeout_err},
    snapshot::{LoadAverage, LoadSnapshot, MemoryInfo, MonitorProfile, PressureStall},
};

#[derive(Default)]
pub struct LoadCollector;

impl Collector for LoadCollector {
    type Output = LoadSnapshot;
    fn domain(&self) -> &'static str {
        "load"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let t = profile.per_collector_timeout();
        let mut out = LoadSnapshot::default();
        match read_proc("/proc/loadavg", t).await {
            Ok(c) => {
                let p: Vec<&str> = c.split_whitespace().collect();
                if p.len() >= 3 {
                    out.load_average = LoadAverage {
                        one: p[0].parse().unwrap_or(0.0),
                        five: p[1].parse().unwrap_or(0.0),
                        fifteen: p[2].parse().unwrap_or(0.0),
                    };
                }
            }
            Err(e) => return Err(timeout_err("load", format!("/proc/loadavg: {e}"))),
        }
        if let Ok(c) = read_proc("/proc/stat", t).await {
            out.cpu_logical_count = c
                .lines()
                .filter(|l| {
                    l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(|b| b.is_ascii_digit())
                })
                .count() as u32;
        }
        if let Ok(c) = read_proc("/proc/meminfo", t).await {
            let (mut tot, mut avail) = (0u64, 0u64);
            for l in c.lines() {
                if l.starts_with("MemTotal:") {
                    tot = pk(l);
                } else if l.starts_with("MemAvailable:") {
                    avail = pk(l);
                }
            }
            out.memory = MemoryInfo {
                total: tot * 1024,
                used: 0,
                available: if avail > 0 { Some(avail * 1024) } else { None },
            };
            if let Ok(o) = tokio::process::Command::new("free")
                .arg("-b")
                .output()
                .await
            {
                if o.status.success() {
                    let f = String::from_utf8_lossy(&o.stdout);
                    if let Some(ml) = f.lines().nth(1) {
                        let cols: Vec<&str> = ml.split_whitespace().collect();
                        if cols.len() >= 3 {
                            out.memory.total = cols[1].parse().unwrap_or(out.memory.total);
                            out.memory.used = cols[2].parse().unwrap_or(0);
                        }
                    }
                }
            }
        }
        if let Ok(c) = read_proc("/proc/meminfo", t).await {
            for l in c.lines() {
                if l.starts_with("SwapTotal:") {
                    out.swap_total = pk(l) * 1024;
                } else if l.starts_with("SwapFree:") {
                    let f = pk(l) * 1024;
                    out.swap_used = out.swap_total.saturating_sub(f);
                }
            }
        }
        if let Ok(c) = read_proc("/proc/pressure/cpu", t).await {
            out.cpu_pressure = psi(&c);
        }
        if let Ok(c) = read_proc("/proc/pressure/memory", t).await {
            out.memory_pressure = psi(&c);
        }
        if let Ok(c) = read_proc("/proc/pressure/io", t).await {
            out.io_pressure = psi(&c);
        }
        Ok(out)
    }
}

fn pk(l: &str) -> u64 {
    l.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
fn psi(c: &str) -> Option<PressureStall> {
    let mut ps = PressureStall {
        avg10: None,
        avg60: None,
        avg300: None,
    };
    for l in c.lines() {
        if l.starts_with("some") {
            for p in l.split_whitespace() {
                if let Some(v) = p.strip_prefix("avg10=") {
                    ps.avg10 = v.parse().ok();
                } else if let Some(v) = p.strip_prefix("avg60=") {
                    ps.avg60 = v.parse().ok();
                } else if let Some(v) = p.strip_prefix("avg300=") {
                    ps.avg300 = v.parse().ok();
                }
            }
            break;
        }
    }
    Some(ps)
}
