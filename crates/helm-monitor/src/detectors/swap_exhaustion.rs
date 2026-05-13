//! Swap exhaustion detection (>90% used).

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct SwapExhaustionDetector;

impl Detector for SwapExhaustionDetector {
    fn id(&self) -> &'static str {
        "swap-exhaustion"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Load
    }
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        if snapshot.domains.load.swap_total == 0 {
            return vec![];
        }
        let pct = (snapshot.domains.load.swap_used as f64
            / snapshot.domains.load.swap_total as f64)
            * 100.0;
        if pct >= 90.0 {
            vec![Finding::new(
                &snapshot.id,
                self.id(),
                "system",
                &format!("Swap usage is critical ({:.0}% used)", pct),
                Severity::Warning,
                Confidence::High,
                MonitorDomain::Load,
            )
            .with_evidence(
                "load.swap_used",
                &format!(
                    "{} / {}",
                    human_bytes(snapshot.domains.load.swap_used),
                    human_bytes(snapshot.domains.load.swap_total)
                ),
                "Swap space nearly exhausted",
            )
            .with_impact(
                "Swap exhaustion may trigger OOM killer. System memory is under severe pressure.",
            )
            .with_read_only_check("free -h")]
        } else {
            vec![]
        }
    }
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}G", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}M", bytes as f64 / 1_048_576.0)
    } else {
        format!("{bytes}B")
    }
}
