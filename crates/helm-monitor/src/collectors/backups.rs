//! Backups collector: detect backup tools.
use crate::{
    collectors::{Collector, bin_exists},
    snapshot::{BackupSnapshot, BackupTool, MonitorProfile},
};
#[derive(Default)]
pub struct BackupsCollector;
impl Collector for BackupsCollector {
    type Output = BackupSnapshot;
    fn domain(&self) -> &'static str {
        "backups"
    }
    async fn collect(
        self,
        _p: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = BackupSnapshot::default();
        for (n, ps) in &[
            ("restic", &["/usr/bin/restic", "/usr/local/bin/restic"][..]),
            ("borg", &["/usr/bin/borg", "/usr/local/bin/borg"]),
            (
                "borgmatic",
                &["/etc/borgmatic", "/etc/borgmatic/config.yaml"],
            ),
            ("rsync", &["/usr/bin/rsync"]),
            ("tar", &["/usr/bin/tar"]),
        ] {
            if ps.iter().any(|p| std::path::Path::new(p).exists()) {
                let bp = ps
                    .iter()
                    .find(|p| p.contains("bin") || bin_exists(n))
                    .map(|s| s.to_string());
                let cp = ps
                    .iter()
                    .find(|p| p.contains("etc") || p.contains("config"))
                    .map(|s| s.to_string());
                let rp = if *n == "restic" {
                    let h = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
                    let c = format!("{h}/.cache/restic");
                    if std::path::Path::new(&c).exists() {
                        Some(c)
                    } else {
                        None
                    }
                } else if *n == "borg" {
                    let h = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
                    let c = format!("{h}/.cache/borg");
                    if std::path::Path::new(&c).exists() {
                        Some(c)
                    } else {
                        None
                    }
                } else {
                    None
                };
                out.tools_detected.push(BackupTool {
                    name: n.to_string(),
                    binary_path: bp,
                    config_path: cp,
                    repo_path: rp,
                });
            }
        }
        Ok(out)
    }
}
