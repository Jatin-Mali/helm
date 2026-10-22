//! Monitor reporter: run snapshot, apply detectors, produce report. Per TRD §7.
#![allow(clippy::needless_borrows_for_generic_args)]

use crate::{
    collect_snapshot,
    correlation::{CorrelationEngine, CorrelationGroup},
    custom_rules::CustomRulesEngine,
    custom_service::ServiceRegistry,
    detectors::DetectorRegistry,
    findings::{Finding, MonitorDomain, Severity, self_resolved_findings, sort_findings},
    pipeline::pipeline_failure_findings,
    snapshot::{MonitorProfile, ObservationStatus, SnapshotDomains, SystemSnapshot},
};

/// Result of a single monitor run.
#[derive(Debug, Clone)]
pub struct MonitorReport {
    pub snapshot: SystemSnapshot,
    pub findings: Vec<Finding>,
    pub correlation_groups: Vec<CorrelationGroup>,
    pub domains_checked: Vec<MonitorDomain>,
    pub previous_snapshot_id: Option<String>,
}

/// Orchestrate a full monitor cycle.
pub struct MonitorReporter {
    pub registry: DetectorRegistry,
    pub correlation_engine: CorrelationEngine,
    previous_findings: Vec<Finding>,
    previous_host_id: Option<String>,
}

impl MonitorReporter {
    pub fn new() -> Self {
        Self {
            registry: DetectorRegistry::default_registry(),
            correlation_engine: CorrelationEngine::default(),
            previous_findings: Vec::new(),
            previous_host_id: None,
        }
    }

    pub async fn run(
        &self,
        profile: MonitorProfile,
        domain_filter: Option<&[MonitorDomain]>,
        previous_snapshot: Option<SystemSnapshot>,
    ) -> MonitorReport {
        let mut engine = self.correlation_engine.clone();
        self.run_with_engine(&mut engine, profile, domain_filter, previous_snapshot, None)
            .await
    }

    pub async fn run_stateful(
        &mut self,
        profile: MonitorProfile,
        domain_filter: Option<&[MonitorDomain]>,
        previous_snapshot: Option<SystemSnapshot>,
    ) -> MonitorReport {
        let mut engine = self.correlation_engine.clone();
        let mut report = self
            .run_with_engine(
                &mut engine,
                profile,
                domain_filter,
                previous_snapshot,
                Some(&self.previous_findings),
            )
            .await;
        self.correlation_engine = engine;
        self.remember_report(&mut report);
        report
    }

    fn remember_report(&mut self, report: &mut MonitorReport) {
        if self.previous_host_id != report.snapshot.domains.collection.host_id {
            // A reporter may switch hosts; persisted incidents belong to their original host.
            report
                .findings
                .retain(|finding| finding.snapshot_id == report.snapshot.id);
        }
        self.previous_host_id = report.snapshot.domains.collection.host_id.clone();
        self.previous_findings = report
            .findings
            .iter()
            .filter(|finding| !finding.lifecycle.is_resolved())
            .cloned()
            .collect();
    }

    async fn run_with_engine(
        &self,
        engine: &mut CorrelationEngine,
        profile: MonitorProfile,
        domain_filter: Option<&[MonitorDomain]>,
        previous_snapshot: Option<SystemSnapshot>,
        previous_findings: Option<&[Finding]>,
    ) -> MonitorReport {
        let snapshot = collect_snapshot(profile).await;
        let mut active_findings =
            self.registry
                .detect(&snapshot, domain_filter, previous_snapshot.as_ref());
        active_findings.extend(
            CustomRulesEngine::load_default()
                .evaluate(&snapshot)
                .into_iter()
                .map(|finding| finding.normalize_for_emission("custom-rule", snapshot.collected_at))
                .filter(|finding| finding.validate_for_emission().is_ok())
                .filter(|finding| {
                    domain_filter.is_none_or(|domains| domains.contains(&finding.category))
                }),
        );
        if let Ok(mut service_registry) = ServiceRegistry::load_default() {
            active_findings.extend(
                service_registry
                    .check_health()
                    .await
                    .into_iter()
                    .map(|finding| {
                        finding.normalize_for_emission("custom-service", snapshot.collected_at)
                    })
                    .filter(|finding| finding.validate_for_emission().is_ok())
                    .filter(|finding| {
                        domain_filter.is_none_or(|domains| domains.contains(&finding.category))
                    }),
            );
        }
        active_findings.extend(
            pipeline_failure_findings(
                &snapshot.domains.pipelines,
                &snapshot.id,
                snapshot.collected_at,
            )
            .into_iter()
            .filter(|finding| finding.validate_for_emission().is_ok())
            .filter(|finding| {
                domain_filter.is_none_or(|domains| domains.contains(&finding.category))
            }),
        );
        self.finish_report(
            engine,
            snapshot,
            active_findings,
            previous_snapshot.as_ref(),
            previous_findings,
            domain_filter,
        )
    }

    fn finish_report(
        &self,
        engine: &mut CorrelationEngine,
        snapshot: SystemSnapshot,
        mut active_findings: Vec<Finding>,
        previous_snapshot: Option<&SystemSnapshot>,
        previous_findings: Option<&[Finding]>,
        domain_filter: Option<&[MonitorDomain]>,
    ) -> MonitorReport {
        let previous_snapshot_id = previous_snapshot.map(|snapshot| snapshot.id.clone());
        sort_findings(&mut active_findings);
        let correlation = engine.correlate(&snapshot, &active_findings);
        let mut findings = correlation.findings;
        if let Some(previous_findings) = previous_findings {
            let can_resolve = |finding: &Finding| {
                let comparable = previous_snapshot.is_some_and(|previous| {
                    snapshot.domains.collection.host_id.is_some()
                        && snapshot.domains.collection.host_id
                            == previous.domains.collection.host_id
                        && snapshot.host.hostname == previous.host.hostname
                        && snapshot.profile == previous.profile
                        && snapshot.collected_at > previous.collected_at
                });
                let registered = self.registry.detector_ids();
                if !comparable
                    || !registered.contains(&finding.detector_id.as_str())
                    || matches!(
                        finding.detector_id.as_str(),
                        "config-drift" | "desktop-notify"
                    )
                    || domain_filter.is_some_and(|domains| !domains.contains(&finding.category))
                {
                    return false;
                }
                let mut required = vec![finding.category.as_str()];
                if matches!(
                    finding.detector_id.as_str(),
                    "db-health" | "web-server" | "monitoring-health"
                ) {
                    required.extend(["services", "ports"]);
                }
                if finding.detector_id == "missing-backup-schedule" {
                    required.extend(["services", "timers"]);
                }
                required.into_iter().all(|domain| {
                    snapshot.observation_status(domain) == ObservationStatus::Successful
                        && previous_snapshot.is_some_and(|previous| {
                            previous.observation_status(domain) == ObservationStatus::Successful
                        })
                })
            };
            let proven: Vec<_> = previous_findings
                .iter()
                .filter(|finding| can_resolve(finding))
                .map(|finding| {
                    let mut finding = finding.clone();
                    finding
                        .missing_data
                        .retain(|message| !message.starts_with("Recovery unverified:"));
                    finding
                })
                .collect();
            findings.extend(self_resolved_findings(
                &proven,
                &findings,
                &snapshot.id,
                snapshot.collected_at,
            ));
            let current: std::collections::HashSet<_> =
                findings.iter().map(Finding::fingerprint).collect();
            for finding in previous_findings.iter().filter(|finding| {
                !finding.lifecycle.is_resolved()
                    && !can_resolve(finding)
                    && !current.contains(&finding.fingerprint())
            }) {
                let mut last_known = finding.clone();
                last_known.clear_correlation();
                last_known
                    .missing_data
                    .retain(|message| !message.starts_with("Recovery unverified:"));
                last_known.missing_data.push(format!("Recovery unverified: current coverage does not prove recovery; last observed at {} (snapshot {}).", finding.detected_at, finding.snapshot_id));
                findings.push(last_known);
            }
        }
        sort_findings(&mut findings);
        let domains_checked = if let Some(doms) = domain_filter {
            doms.to_vec()
        } else {
            SnapshotDomains::domain_names()
                .into_iter()
                .filter_map(MonitorDomain::from_domain_str)
                .collect()
        };
        MonitorReport {
            snapshot,
            findings,
            correlation_groups: correlation.groups,
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
    pub fn coverage(&self) -> std::collections::BTreeMap<String, ObservationStatus> {
        SnapshotDomains::domain_names()
            .into_iter()
            .map(|domain| {
                let status = if MonitorDomain::from_domain_str(domain)
                    .is_some_and(|domain| !self.domains_checked.contains(&domain))
                {
                    ObservationStatus::Filtered
                } else {
                    self.snapshot.observation_status(domain)
                };
                (domain.into(), status)
            })
            .collect()
    }

    fn coverage_text(&self) -> String {
        let coverage = self.coverage();
        let checked = coverage
            .values()
            .filter(|status| **status == ObservationStatus::Successful)
            .count();
        let mut text = format!(
            "Coverage: {checked}/{} domains successfully observed\n",
            coverage.len()
        );
        for (domain, status) in coverage {
            text.push_str(&format!("  {domain}: {}\n", status.as_str()));
        }
        text
    }

    pub fn severity_counts(&self) -> (usize, usize, usize) {
        let mut info = 0;
        let mut warning = 0;
        let mut critical = 0;
        for f in self
            .findings
            .iter()
            .filter(|finding| !finding.lifecycle.is_resolved())
        {
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
            out.push_str(&self.coverage_text());
            out.push_str("No issues detected in available evidence; incomplete coverage leaves health unknown.\n");
            return out;
        }

        out.push_str(&self.coverage_text());
        if !self.correlation_groups.is_empty() {
            out.push_str("Correlations:\n");
            for group in &self.correlation_groups {
                if let Some(primary) = self
                    .findings
                    .iter()
                    .find(|finding| finding.id == group.primary_finding_id)
                {
                    out.push_str(&format!(
                        "  - {} (×{})\n",
                        primary.title,
                        group.finding_ids.len()
                    ));
                }
            }
            out.push('\n');
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
                if f.lifecycle.is_resolved() {
                    out.push_str("    Status: resolved\n");
                }
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
        helm_core::redact_secrets(
            &serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1, "snapshot_id": self.snapshot.id,
                "host": self.snapshot.host.hostname, "collected_at": self.snapshot.collected_at,
                "coverage": self.coverage(), "collector_errors": self.snapshot.collector_errors,
                "findings": self.findings,
            }))
            .expect("monitor report contains serializable data"),
        )
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
            out.push_str(&self.coverage_text());
            out.push_str("*No issues detected in available evidence; incomplete coverage leaves health unknown.*\n");
            return out;
        }

        out.push_str(&self.coverage_text());
        if !self.correlation_groups.is_empty() {
            out.push_str("## Correlations\n\n");
            for group in &self.correlation_groups {
                if let Some(primary) = self
                    .findings
                    .iter()
                    .find(|finding| finding.id == group.primary_finding_id)
                {
                    out.push_str(&format!(
                        "- **{}** groups {} related finding(s)\n",
                        primary.title,
                        group.finding_ids.len().saturating_sub(1)
                    ));
                }
            }
            out.push('\n');
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
                if f.lifecycle.is_resolved() {
                    out.push_str("- **Status:** resolved\n");
                }
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

#[cfg(test)]
mod coverage_tests {
    use super::*;
    use crate::{Confidence, FindingLifecycle, snapshot::ObservationStatus};

    fn snapshot(id: &str) -> SystemSnapshot {
        let mut snapshot = SystemSnapshot::new(
            id.into(),
            Default::default(),
            MonitorProfile::Quick,
            Default::default(),
        );
        snapshot.host.hostname = "fixture".into();
        snapshot.domains.collection.host_id = Some("same-host-and-boot".into());
        snapshot
            .domains
            .collection
            .domains
            .insert("services".into(), ObservationStatus::Successful);
        snapshot
    }

    fn previous_finding(snapshot: &SystemSnapshot) -> Finding {
        Finding::new(
            &snapshot.id,
            "failed-services",
            "fixture.service",
            "Service failed",
            Severity::Warning,
            Confidence::High,
            MonitorDomain::Services,
        )
    }

    #[test]
    fn unavailable_observation_cannot_resolve_or_forget_incident() {
        let reporter = MonitorReporter::new();
        let previous = snapshot("previous");
        let active = vec![previous_finding(&previous)];
        for status in [
            ObservationStatus::Failed,
            ObservationStatus::Unsupported,
            ObservationStatus::Filtered,
            ObservationStatus::Stale,
            ObservationStatus::Unknown,
        ] {
            let mut current = snapshot("current");
            current
                .domains
                .collection
                .domains
                .insert("services".into(), status);
            let report = reporter.finish_report(
                &mut CorrelationEngine::default(),
                current,
                vec![],
                Some(&previous),
                Some(&active),
                None,
            );
            assert_eq!(report.findings.len(), 1, "lost incident with {status:?}");
            assert!(
                !report.findings[0].lifecycle.is_resolved(),
                "false resolution with {status:?}"
            );
            assert!(!report.findings[0].missing_data.is_empty());
        }
    }

    #[test]
    fn fresh_comparable_success_can_resolve_incident() {
        let reporter = MonitorReporter::new();
        let mut previous = snapshot("previous");
        previous.collected_at -= chrono::Duration::seconds(10);
        let active = vec![previous_finding(&previous)];
        let report = reporter.finish_report(
            &mut CorrelationEngine::default(),
            snapshot("current"),
            vec![],
            Some(&previous),
            Some(&active),
            None,
        );
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].lifecycle, FindingLifecycle::SelfResolved);
        assert_eq!(report.severity_counts(), (0, 0, 0));
        assert!(report.render_text().contains("Status: resolved"));
    }

    #[test]
    fn switching_hosts_cannot_migrate_and_later_resolve_old_incidents() {
        let mut reporter = MonitorReporter::new();
        let first = snapshot("host-a");
        let active = vec![previous_finding(&first)];
        let mut report = reporter.finish_report(
            &mut CorrelationEngine::default(),
            first.clone(),
            active.clone(),
            None,
            None,
            None,
        );
        reporter.remember_report(&mut report);
        assert_eq!(reporter.previous_findings.len(), 1);
        let mut second = snapshot("host-b");
        second.domains.collection.host_id = Some("other-host".into());
        let mut report = reporter.finish_report(
            &mut CorrelationEngine::default(),
            second.clone(),
            vec![],
            Some(&first),
            Some(&active),
            None,
        );
        reporter.remember_report(&mut report);
        assert!(report.findings.is_empty());
        assert!(reporter.previous_findings.is_empty());
        let next = reporter.finish_report(
            &mut CorrelationEngine::default(),
            second.clone(),
            vec![],
            Some(&second),
            Some(&reporter.previous_findings),
            None,
        );
        assert!(next.findings.is_empty());
    }

    #[test]
    fn incident_survives_collection_gap_and_later_resolves_without_stale_warning() {
        let reporter = MonitorReporter::new();
        let mut baseline = snapshot("baseline");
        baseline.collected_at -= chrono::Duration::seconds(30);
        let mut active = vec![previous_finding(&baseline)];
        for (id, seconds, status) in [
            ("failed", 20, ObservationStatus::Failed),
            ("restored-first", 10, ObservationStatus::Successful),
            ("restored-second", 0, ObservationStatus::Successful),
        ] {
            let mut current = snapshot(id);
            current.collected_at -= chrono::Duration::seconds(seconds);
            current
                .domains
                .collection
                .domains
                .insert("services".into(), status);
            let report = reporter.finish_report(
                &mut CorrelationEngine::default(),
                current.clone(),
                vec![],
                Some(&baseline),
                Some(&active),
                None,
            );
            assert_eq!(report.findings.len(), 1);
            if id == "restored-second" {
                assert_eq!(report.findings[0].lifecycle, FindingLifecycle::SelfResolved);
                assert!(
                    report.findings[0]
                        .missing_data
                        .iter()
                        .all(|message| !message.starts_with("Recovery unverified:"))
                );
            } else {
                assert!(!report.findings[0].lifecycle.is_resolved());
            }
            active = report.findings;
            baseline = current;
        }
    }

    #[test]
    fn filtered_stale_unknown_or_different_host_cannot_resolve() {
        let reporter = MonitorReporter::new();
        let previous = snapshot("previous");
        let active = vec![previous_finding(&previous)];
        for case in 0..5 {
            let mut current = snapshot("current");
            let filter = match case {
                0 => Some(vec![MonitorDomain::Disks]),
                1 => {
                    current.collected_at -= chrono::Duration::minutes(10);
                    None
                }
                2 => {
                    current.domains.collection.host_id = Some("other-host".into());
                    None
                }
                3 => {
                    current.domains.collection.host_id = None;
                    None
                }
                _ => {
                    current.domains.collection.domains.clear();
                    None
                }
            };
            let report = reporter.finish_report(
                &mut CorrelationEngine::default(),
                current,
                vec![],
                Some(&previous),
                Some(&active),
                filter.as_deref(),
            );
            assert!(
                report
                    .findings
                    .iter()
                    .all(|finding| !finding.lifecycle.is_resolved()),
                "false recovery case {case}"
            );
            assert_eq!(report.findings.len(), 1);
        }
    }

    #[test]
    fn empty_report_and_legacy_snapshot_expose_unknown_coverage() {
        let reporter = MonitorReporter::new();
        let mut current = snapshot("legacy");
        current.domains.collection = Default::default();
        let mut legacy = serde_json::to_value(&current.domains).unwrap();
        legacy.as_object_mut().unwrap().remove("collection");
        current.domains = serde_json::from_value(legacy).unwrap();
        let report = reporter.finish_report(
            &mut CorrelationEngine::default(),
            current,
            vec![],
            None,
            None,
            None,
        );
        let json: serde_json::Value = serde_json::from_str(&report.render_json()).unwrap();
        assert!(json["findings"].as_array().unwrap().is_empty());
        assert_eq!(json["coverage"]["services"], "unknown");
        assert!(report.render_text().contains("health unknown"));
        assert!(report.render_markdown().contains("health unknown"));
        assert_eq!(report.domains_checked.len(), 16);
    }

    #[test]
    fn observation_errors_override_success_and_serialization_preserves_coverage() {
        let mut current = snapshot("current");
        let domains: crate::SnapshotDomains =
            serde_json::from_str(&serde_json::to_string(&current.domains).unwrap()).unwrap();
        assert_eq!(domains.collection, current.domains.collection);
        current.collector_errors.push(crate::CollectorError {
            domain: "services".into(),
            message: "denied".into(),
            is_timeout: false,
        });
        assert_eq!(
            current.observation_status("services"),
            ObservationStatus::Failed
        );
        current.collector_errors.clear();
        current.collected_at -= chrono::Duration::minutes(10);
        assert_eq!(
            current.observation_status("services"),
            ObservationStatus::Stale
        );
    }
}
