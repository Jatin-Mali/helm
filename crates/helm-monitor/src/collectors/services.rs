//! Services collector: systemd units, failed, timers.
use crate::{
    collectors::{Collector, err, run_timed},
    snapshot::{FailedUnit, MonitorProfile, ServiceSnapshot, SystemdTimer, SystemdUnit},
};
#[derive(Default)]
pub struct ServicesCollector;
impl Collector for ServicesCollector {
    type Output = ServiceSnapshot;
    fn domain(&self) -> &'static str {
        "services"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = ServiceSnapshot::default();
        match run_timed(
            "systemctl",
            &["list-units", "--all", "--no-legend", "--no-pager"],
            profile,
        )
        .await
        {
            Ok(o) => out.units = parse_units(&String::from_utf8_lossy(&o.stdout)),
            Err(e) => return Err(err("services", e.message)),
        }
        if let Ok(o) = run_timed(
            "systemctl",
            &["--failed", "--no-legend", "--no-pager"],
            profile,
        )
        .await
        {
            out.failed_units = parse_failed(&String::from_utf8_lossy(&o.stdout));
        }
        if let Ok(o) = run_timed(
            "systemctl",
            &["list-timers", "--all", "--no-legend", "--no-pager"],
            profile,
        )
        .await
        {
            out.timers = parse_timers(&String::from_utf8_lossy(&o.stdout));
        }
        Ok(out)
    }
}
fn parse_units(s: &str) -> Vec<SystemdUnit> {
    s.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 4 {
                None
            } else {
                let d = p[..4].iter().map(|x| x.len()).sum::<usize>() + 4;
                Some(SystemdUnit {
                    name: p[0].trim_end_matches(".service").into(),
                    load: p[1].into(),
                    active: p[2].into(),
                    sub: p[3].into(),
                    description: l.get(d..).unwrap_or("").trim().into(),
                })
            }
        })
        .collect()
}
fn parse_failed(s: &str) -> Vec<FailedUnit> {
    s.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 4 {
                None
            } else {
                let d = p[..4].iter().map(|x| x.len()).sum::<usize>() + 4;
                Some(FailedUnit {
                    name: p[0].into(),
                    description: l.get(d..).unwrap_or("").trim().into(),
                    loaded: p[1].into(),
                    active: p[2].into(),
                    sub: p[3].into(),
                })
            }
        })
        .collect()
}
fn parse_timers(s: &str) -> Vec<SystemdTimer> {
    s.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 6 {
                None
            } else {
                Some(SystemdTimer {
                    name: p[5].trim_end_matches(".timer").into(),
                    next_trigger: format!("{} {}", p[0], p.get(1).unwrap_or(&"")),
                    last_trigger: format!("{} {}", p[2], p.get(3).unwrap_or(&"")),
                    passed: p.get(4).unwrap_or(&"").to_string(),
                    unit: p[5].into(),
                    activates: p.get(6).unwrap_or(&"").to_string(),
                })
            }
        })
        .collect()
}
