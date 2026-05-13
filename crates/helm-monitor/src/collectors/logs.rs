//! Logs collector: journalctl bounded windows.
use crate::{
    collectors::{Collector, run_timed},
    snapshot::{LogSnapshot, MonitorProfile},
};
#[derive(Default)]
pub struct LogsCollector;
impl Collector for LogsCollector {
    type Output = LogSnapshot;
    fn domain(&self) -> &'static str {
        "logs"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = LogSnapshot::default();
        if let Ok(o) = run_timed(
            "journalctl",
            &["-p", "err", "--since", "1 hour ago", "--no-pager"],
            profile,
        )
        .await
        {
            let s = String::from_utf8_lossy(&o.stdout);
            out.journal_errors_last_hour = s.lines().filter(|l| !l.is_empty()).count() as u64;
        }
        if let Ok(o) = run_timed(
            "journalctl",
            &[
                "-k",
                "-p",
                "err",
                "--since",
                "1 hour ago",
                "--no-pager",
                "-n",
                "20",
            ],
            profile,
        )
        .await
        {
            out.kernel_errors = String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .take(20)
                .map(|x| x.into())
                .collect();
        }
        if let Ok(o) = run_timed(
            "journalctl",
            &[
                "--no-pager",
                "-n",
                "50",
                "--since",
                "24 hours ago",
                "-u",
                "sshd",
                "-u",
                "sudo",
            ],
            profile,
        )
        .await
        {
            out.auth_failures = String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| {
                    l.to_lowercase().contains("failed")
                        || l.to_lowercase().contains("authentication failure")
                })
                .take(10)
                .map(|x| x.into())
                .collect();
        }
        if matches!(profile, MonitorProfile::Standard | MonitorProfile::Deep) {
            if let Ok(o) = run_timed(
                "journalctl",
                &["-p", "err", "--since", "5 minutes ago", "--no-pager"],
                profile,
            )
            .await
            {
                let c = String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| !l.is_empty())
                    .count() as f64;
                out.error_rate_per_minute = Some(c / 5.0);
            }
        }
        Ok(out)
    }
}
