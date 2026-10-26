//! Services collector: systemd units, failed, timers.
use crate::{
    collectors::{Collector, run_timed},
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
        let o = run_timed(
            "systemctl",
            &["list-units", "--all", "--no-legend", "--no-pager"],
            profile,
        )
        .await?;
        out.units = parse_units(&String::from_utf8_lossy(&o.stdout));
        let o = run_timed(
            "systemctl",
            &["--failed", "--no-legend", "--no-pager"],
            profile,
        )
        .await?;
        out.failed_units = parse_failed(&String::from_utf8_lossy(&o.stdout));
        let o = run_timed(
            "systemctl",
            &["list-timers", "--all", "--no-legend", "--no-pager"],
            profile,
        )
        .await?;
        out.timers = parse_timers(&String::from_utf8_lossy(&o.stdout));
        Ok(out)
    }
}
/// Splits `systemctl --no-legend` rows into `n` leading fields and the free-text remainder.
/// Newer systemd prefixes failed/inactive rows with a status bullet (`●`, `○`, `×`, `↻`, `*`);
/// without stripping it the bullet becomes the unit name and distinct units collapse together.
fn split_row(line: &str, n: usize) -> Option<(Vec<&str>, &str)> {
    let mut rest = line.trim_start();
    if let Some(stripped) = rest.strip_prefix(['●', '○', '×', '↻', '*']) {
        rest = stripped.trim_start();
    }
    let mut fields = Vec::with_capacity(n);
    for _ in 0..n {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        fields.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    Some((fields, rest.trim()))
}
fn parse_units(s: &str) -> Vec<SystemdUnit> {
    s.lines()
        .filter_map(|l| {
            let (p, description) = split_row(l, 4)?;
            Some(SystemdUnit {
                name: p[0].into(),
                load: p[1].into(),
                active: p[2].into(),
                sub: p[3].into(),
                description: description.into(),
            })
        })
        .collect()
}
fn parse_failed(s: &str) -> Vec<FailedUnit> {
    s.lines()
        .filter_map(|l| {
            let (p, description) = split_row(l, 4)?;
            Some(FailedUnit {
                name: p[0].into(),
                description: description.into(),
                loaded: p[1].into(),
                active: p[2].into(),
                sub: p[3].into(),
            })
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
                    name: p[5].into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_bullets_do_not_become_unit_names() {
        let rows = "● snapd.service loaded failed failed  Snap Daemon\n\
                    ● logrotate.service loaded failed failed Rotate log files\n\
                    nginx.service loaded active running A web server\n";
        let failed = parse_failed(rows);
        let names: Vec<_> = failed.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(
            names,
            ["snapd.service", "logrotate.service", "nginx.service"]
        );
        assert_eq!(failed[0].description, "Snap Daemon");
        assert_eq!(failed[0].sub, "failed");
        let units = parse_units(rows);
        assert_eq!(units[2].name, "nginx.service");
        assert_eq!(units[2].description, "A web server");
    }

    #[test]
    fn short_or_empty_rows_are_skipped() {
        assert!(parse_failed("").is_empty());
        assert!(parse_failed("● only.service loaded").is_empty());
        assert!(parse_units("   \n").is_empty());
    }
}
