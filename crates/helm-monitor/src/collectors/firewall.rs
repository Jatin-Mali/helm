//! Firewall collector: iptables/nftables, ufw, firewalld status.
use crate::{
    collectors::{Collector, bin_exists, run_timed},
    snapshot::{FirewallSnapshot, MonitorProfile},
};

pub struct FirewallCollector;
impl Collector for FirewallCollector {
    type Output = FirewallSnapshot;
    fn domain(&self) -> &'static str {
        "firewall"
    }
    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = FirewallSnapshot::default();

        // Detect iptables vs nftables
        if bin_exists("iptables") {
            out.firewall_tool = Some("iptables".into());
            // Count rules (bounded)
            if let Ok(o) = run_timed("iptables", &["-L", "-n"], profile).await {
                let s = String::from_utf8_lossy(&o.stdout);
                let line_count = s.lines().filter(|l| !l.is_empty()).count();
                out.iptables_rule_count = Some(line_count as u64);
                // Check for default ACCEPT on INPUT
                out.default_accept_input = Some(
                    s.lines()
                        .any(|l| l.contains("Chain INPUT") && l.contains("ACCEPT")),
                );
            }
        } else if bin_exists("nft") {
            out.firewall_tool = Some("nftables".into());
        }

        // ufw status
        if bin_exists("ufw") {
            if let Ok(o) = run_timed("ufw", &["status"], profile).await {
                let s = String::from_utf8_lossy(&o.stdout);
                out.ufw_active = Some(s.contains("active"));
            }
        }

        // firewalld status
        if bin_exists("firewall-cmd") {
            if let Ok(o) = run_timed("firewall-cmd", &["--state"], profile).await {
                let s = String::from_utf8_lossy(&o.stdout);
                out.firewalld_active = Some(s.trim() == "running");
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firewall_snapshot_default_is_empty() {
        let snap = FirewallSnapshot::default();
        assert!(snap.firewall_tool.is_none());
        assert!(snap.ufw_active.is_none());
    }
}
