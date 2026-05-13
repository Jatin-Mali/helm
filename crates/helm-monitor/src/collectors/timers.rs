//! Timers collector: cron jobs.
use crate::{
    collectors::Collector,
    snapshot::{CronJob, MonitorProfile, TimerSnapshot},
};
#[derive(Default)]
pub struct TimersCollector;
impl Collector for TimersCollector {
    type Output = TimerSnapshot;
    fn domain(&self) -> &'static str {
        "timers"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = TimerSnapshot::default();
        if matches!(profile, MonitorProfile::Standard | MonitorProfile::Deep) {
            for dir in &[
                "/etc/cron.d",
                "/etc/cron.daily",
                "/etc/cron.hourly",
                "/etc/cron.weekly",
                "/etc/cron.monthly",
            ] {
                if let Ok(e) = std::fs::read_dir(dir) {
                    for entry in e.flatten() {
                        let p = entry.path();
                        if p.is_file() {
                            out.cron_jobs.push(CronJob {
                                path: p.display().to_string(),
                                schedule: read_sched(&p),
                            });
                        }
                    }
                }
            }
            if let Ok(e) = std::fs::read_dir("/var/spool/cron/crontabs") {
                for entry in e.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        out.cron_jobs.push(CronJob {
                            path: p.display().to_string(),
                            schedule: None,
                        });
                    }
                }
            }
        }
        Ok(out)
    }
}
fn read_sched(p: &std::path::Path) -> Option<String> {
    let c = std::fs::read_to_string(p).ok()?;
    for l in c.lines() {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let ps: Vec<&str> = t.split_whitespace().collect();
        if ps.len() >= 6 && ps[0].chars().any(|c| c.is_ascii_digit() || c == '*') {
            return Some(ps[..5].join(" "));
        }
    }
    None
}
