//! Packages collector: apt/dnf/pacman update status.
use crate::{
    collectors::{Collector, bin_exists, run_timed},
    snapshot::{MonitorProfile, PackageSnapshot},
};
#[derive(Default)]
pub struct PackagesCollector;
impl Collector for PackagesCollector {
    type Output = PackageSnapshot;
    fn domain(&self) -> &'static str {
        "packages"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = PackageSnapshot::default();
        if bin_exists("apt") || bin_exists("apt-get") {
            out.package_manager = Some("apt".into());
            if let Ok(o) =
                run_timed("sh", &["-c", "apt list --upgradable 2>/dev/null"], profile).await
            {
                let s = String::from_utf8_lossy(&o.stdout);
                let c = s.lines().filter(|l| l.contains('/')).count();
                if c > 0 {
                    out.upgradable_count = Some(c as u64);
                    out.security_count = Some(
                        s.lines()
                            .filter(|l| l.contains("-security") || l.contains("-updates"))
                            .count() as u64,
                    );
                }
            }
        } else if bin_exists("dnf") {
            out.package_manager = Some("dnf".into());
            if let Ok(o) = run_timed("dnf", &["check-update", "-q"], profile).await {
                let c = String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| !l.is_empty() && !l.contains("Last metadata"))
                    .count();
                if c > 0 {
                    out.upgradable_count = Some(c as u64);
                }
            }
            if let Ok(o) =
                run_timed("dnf", &["updateinfo", "list", "--security", "-q"], profile).await
            {
                let c = String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| l.contains("sec") || l.contains("Important"))
                    .count();
                if c > 0 {
                    out.security_count = Some(c as u64);
                }
            }
        } else if bin_exists("pacman") {
            out.package_manager = Some("pacman".into());
            if let Ok(o) = run_timed("checkupdates", &[], profile).await {
                let c = String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| !l.is_empty())
                    .count();
                if c > 0 {
                    out.upgradable_count = Some(c as u64);
                }
            }
        }
        Ok(out)
    }
}
