//! Monitor reporter: run snapshot, apply detectors, produce report. Per TRD §7.
#![allow(clippy::needless_borrows_for_generic_args)]

use crate::{
    collect_snapshot,
    detectors::DetectorRegistry,
    findings::{Finding, MonitorDomain, Severity},
    snapshot::{MonitorProfile, SystemSnapshot},
};

/// Result of a single monitor run.
#[derive(Debug, Clone)]
pub struct MonitorReport {
    pub snapshot: SystemSnapshot,
    pub findings: Vec<Finding>,
    pub domains_checked: Vec<MonitorDomain>,
    pub previous_snapshot_id: Option<String>,
}

/// Orchestrate a full monitor cycle.
pub struct MonitorReporter {
    pub registry: DetectorRegistry,
}

impl MonitorReporter {
    pub fn new() -> Self {
        Self {
            registry: DetectorRegistry::default_registry(),
        }
    }

    pub async fn run(
        &self,
        profile: MonitorProfile,
        domain_filter: Option<&[MonitorDomain]>,
        previous_snapshot: Option<SystemSnapshot>,
    ) -> MonitorReport {
        let snapshot = collect_snapshot(profile).await;
        let previous_snapshot_id = previous_snapshot.as_ref().map(|p| p.id.clone());
        let findings = self
            .registry
            .detect(&snapshot, domain_filter, previous_snapshot.as_ref());
        let domains_checked = if let Some(doms) = domain_filter {
            doms.to_vec()
        } else {
            findings.iter().map(|f| f.category).collect()
        };
        MonitorReport {
            snapshot,
            findings,
            domains_checked,
            previous_snapshot_id,
        }
    }
}

impl Default for MonitorReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl MonitorReport {
    pub fn severity_counts(&self) -> (usize, usize, usize) {
        let mut info = 0;
        let mut warning = 0;
        let mut critical = 0;
        for f in &self.findings {
            match f.severity {
                Severity::Info => info += 1,
                Severity::Warning => warning += 1,
                Severity::Critical => critical += 1,
            }
        }
        (info, warning, critical)
    }

    pub fn render_text(&self) -> String {
        let (info, warning, critical) = self.severity_counts();
        let mut out = String::new();
        out.push_str(&format!(
            "HELM Monitor Report\n==================\nSnapshot: {}\nHost: {}\nTime: {}\n",
            self.snapshot.id, self.snapshot.host.hostname, self.snapshot.collected_at
        ));
        if let Some(ref prev_id) = self.previous_snapshot_id {
            out.push_str(&format!("Baseline: {prev_id}\n"));
        }
        out.push('\n');
        out.push_str(&format!(
            "Findings: {} critical, {} warning, {} info\n\n",
            critical, warning, info
        ));

        if self.findings.is_empty() {
            out.push_str("No issues detected.\n");
            return out;
        }

        for severity in &[Severity::Critical, Severity::Warning, Severity::Info] {
            let group: Vec<&Finding> = self
                .findings
                .iter()
                .filter(|f| f.severity == *severity)
                .collect();
            if group.is_empty() {
                continue;
            }
            let label = match severity {
                Severity::Critical => "CRITICAL",
                Severity::Warning => "WARNING",
                Severity::Info => "INFO",
            };
            out.push_str(&format!("── {label} ──\n\n"));
            for f in &group {
                out.push_str(&format!("  {}\n", f.title));
                out.push_str(&format!("    Resource:  {}\n", f.affected_resource));
                out.push_str(&format!("    Impact:    {}\n", f.impact));
                if !f.evidence.is_empty() {
                    out.push_str("    Evidence:\n");
                    for e in &f.evidence {
                        out.push_str(&format!("      - {}: {} ({})\n", e.source, e.value, e.note));
                    }
                }
                if !f.assumptions.is_empty() {
                    out.push_str(&format!("    Assumptions: {}\n", f.assumptions.join(", ")));
                }
                if !f.missing_data.is_empty() {
                    out.push_str(&format!("    Missing: {}\n", f.missing_data.join(", ")));
                }
                if !f.read_only_checks.is_empty() {
                    out.push_str("    Checks:\n");
                    for c in &f.read_only_checks {
                        out.push_str(&format!("      $ {c}\n"));
                    }
                }
                out.push('\n');
            }
        }
        out
    }

    pub fn render_json(&self) -> String {
        serde_json::to_string_pretty(&self.findings).unwrap_or_default()
    }

    pub fn render_markdown(&self) -> String {
        let (info, warning, critical) = self.severity_counts();
        let mut out = String::new();
        out.push_str(&format!(
            "# HELM Monitor Report\n\n**Snapshot:** `{}`  \n**Host:** `{}`  \n**Time:** {}  \n\n",
            self.snapshot.id, self.snapshot.host.hostname, self.snapshot.collected_at
        ));
        out.push_str(&format!(
            "| Severity | Count |\n|----------|-------|\n| Critical | {critical} |\n| Warning  | {warning} |\n| Info     | {info} |\n\n"
        ));

        if self.findings.is_empty() {
            out.push_str("*No issues detected.*\n");
            return out;
        }

        for severity in &[Severity::Critical, Severity::Warning, Severity::Info] {
            let group: Vec<&Finding> = self
                .findings
                .iter()
                .filter(|f| f.severity == *severity)
                .collect();
            if group.is_empty() {
                continue;
            }
            let heading = match severity {
                Severity::Critical => "Critical",
                Severity::Warning => "Warnings",
                Severity::Info => "Info",
            };
            out.push_str(&format!("## {heading}\n\n"));
            for f in &group {
                out.push_str(&format!("### {}\n\n", f.title));
                out.push_str(&format!("- **Resource:** {}\n", f.affected_resource));
                out.push_str(&format!("- **Impact:** {}\n", f.impact));
                if !f.evidence.is_empty() {
                    out.push_str("- **Evidence:**\n");
                    for e in &f.evidence {
                        out.push_str(&format!(
                            "  - `{}` = `{}` — {}\n",
                            e.source, e.value, e.note
                        ));
                    }
                }
                if !f.read_only_checks.is_empty() {
                    out.push_str("- **Checks:**\n");
                    for c in &f.read_only_checks {
                        out.push_str(&format!("  - `{c}`\n"));
                    }
                }
                out.push('\n');
            }
        }
        out
    }
}
