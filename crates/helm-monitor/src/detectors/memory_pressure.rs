//! Memory pressure detection.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct MemoryPressureDetector;

impl Detector for MemoryPressureDetector {
    fn id(&self) -> &'static str {
        "memory-pressure"
    }
    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Load
    }
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        let mem = &snapshot.domains.load.memory;
        if mem.total == 0 {
            return vec![];
        }
        let used_pct = (mem.used as f64 / mem.total as f64) * 100.0;

        let mut findings = Vec::new();
        if used_pct >= 95.0 {
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    &format!("Memory usage is critical ({:.0}% used)", used_pct),
                    Severity::Critical,
                    Confidence::High,
                    MonitorDomain::Load,
                )
                .with_evidence(
                    "load.memory.used",
                    &format!("{} / {}", human_bytes(mem.used), human_bytes(mem.total)),
                    "Nearly all memory is consumed",
                )
                .with_impact("System may OOM-kill processes or swap heavily")
                .with_read_only_check("ps aux --sort=-%mem | head -15"),
            );
        } else if used_pct >= 85.0 {
            findings.push(
                Finding::new(
                    &snapshot.id,
                    self.id(),
                    "system",
                    &format!("Memory usage is high ({:.0}% used)", used_pct),
                    Severity::Warning,
                    Confidence::High,
                    MonitorDomain::Load,
                )
                .with_evidence(
                    "load.memory.used",
                    &format!("{} / {}", human_bytes(mem.used), human_bytes(mem.total)),
                    "Memory usage is approaching capacity",
                )
                .with_impact(
                    "Available memory is low; new allocations may fail or trigger swapping",
                )
                .with_read_only_check("free -h && ps aux --sort=-%mem | head -10"),
            );
        }

        if let Some(psi) = &snapshot.domains.load.memory_pressure {
            if let Some(avg60) = psi.avg60 {
                if avg60 > 10.0 {
                    findings.push(
                        Finding::new(
                            &snapshot.id,
                            self.id(),
                            "system",
                            &format!("Memory pressure stall is high (avg60={avg60:.1})"),
                            Severity::Warning,
                            Confidence::High,
                            MonitorDomain::Load,
                        )
                        .with_evidence(
                            "load.memory_pressure.avg60",
                            &format!("{avg60:.1}"),
                            "PSI memory pressure indicates processes waiting for memory",
                        )
                        .with_impact("Applications are stalling due to memory contention"),
                    );
                }
            }
        }

        findings
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
