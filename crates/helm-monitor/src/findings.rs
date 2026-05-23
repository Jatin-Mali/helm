//! Finding model — the output of detectors. Per TRD §4.2.
#![allow(clippy::needless_borrows_for_generic_args)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::snapshot::SnapshotId;

/// Opaque finding identifier.
pub type FindingId = String;

/// Opaque plan identifier.
pub type PlanId = String;

// ── Severity ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

// ── Confidence ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

// ── FindingCategory / MonitorDomain ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MonitorDomain {
    Disks,
    Services,
    Containers,
    Ports,
    Load,
    Logs,
    Backups,
    Packages,
    Network,
    Timers,
    Processes,
    Firewall,
    Kubernetes,
    Libvirt,
    Compose,
    Certs,
}

impl MonitorDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disks => "disks",
            Self::Services => "services",
            Self::Containers => "containers",
            Self::Ports => "ports",
            Self::Load => "load",
            Self::Logs => "logs",
            Self::Backups => "backups",
            Self::Packages => "packages",
            Self::Network => "network",
            Self::Timers => "timers",
            Self::Processes => "processes",
            Self::Firewall => "firewall",
            Self::Kubernetes => "kubernetes",
            Self::Libvirt => "libvirt",
            Self::Compose => "compose",
            Self::Certs => "certs",
        }
    }

    pub fn from_domain_str(s: &str) -> Option<Self> {
        match s {
            "disks" => Some(Self::Disks),
            "services" => Some(Self::Services),
            "containers" => Some(Self::Containers),
            "ports" => Some(Self::Ports),
            "load" => Some(Self::Load),
            "logs" => Some(Self::Logs),
            "backups" => Some(Self::Backups),
            "packages" => Some(Self::Packages),
            "network" => Some(Self::Network),
            "timers" => Some(Self::Timers),
            "processes" => Some(Self::Processes),
            "firewall" => Some(Self::Firewall),
            "kubernetes" => Some(Self::Kubernetes),
            "libvirt" => Some(Self::Libvirt),
            "compose" => Some(Self::Compose),
            "certs" => Some(Self::Certs),
            _ => None,
        }
    }
}

// ── FindingLifecycle ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FindingLifecycle {
    #[default]
    Open,
    Resolved,
    SelfResolved,
    Suppressed,
}

impl FindingLifecycle {
    pub fn is_resolved(self) -> bool {
        matches!(self, Self::Resolved | Self::SelfResolved)
    }
}

// ── Finding ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: FindingId,
    #[serde(default)]
    pub detector_id: String,
    #[serde(default)]
    pub fingerprint: String,
    pub snapshot_id: SnapshotId,
    #[serde(default = "default_detected_at")]
    pub detected_at: DateTime<Utc>,
    pub severity: Severity,
    pub confidence: Confidence,
    pub category: MonitorDomain,
    pub title: String,
    pub affected_resource: String,
    pub evidence: Vec<EvidenceRef>,
    pub assumptions: Vec<String>,
    pub missing_data: Vec<String>,
    pub impact: String,
    pub read_only_checks: Vec<String>,
    pub fix_plan: Option<String>,
    #[serde(default)]
    pub lifecycle: FindingLifecycle,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub root_cause_finding_id: Option<String>,
    #[serde(default)]
    pub correlated_finding_ids: Vec<String>,
}

/// Concrete reference to a snapshot field or measurement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    /// What was inspected (e.g. "load.load_average", "disks.filesystems[0].used_bytes").
    pub source: String,
    /// Observed value.
    pub value: String,
    /// Why this evidence matters for the finding.
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingValidationError {
    MissingDetectorId,
    MissingAffectedResource,
    MissingEvidence,
    MissingEvidenceSource(usize),
    MissingEvidenceValue(usize),
    TitleTooLong(usize),
    TitleMissingResource,
    TitleNotPlainLanguage,
}

impl Finding {
    /// Create a new finding with a deterministic ID based on snapshot + detector + resource.
    pub fn new(
        snapshot_id: &str,
        detector_id: &str,
        affected_resource: &str,
        title: &str,
        severity: Severity,
        confidence: Confidence,
        category: MonitorDomain,
    ) -> Self {
        let fingerprint = compute_fingerprint(detector_id, category, affected_resource, title);
        Self {
            id: compute_finding_id(snapshot_id, detector_id, affected_resource),
            detector_id: detector_id.into(),
            fingerprint,
            snapshot_id: snapshot_id.into(),
            detected_at: Utc::now(),
            severity,
            confidence,
            category,
            title: title.into(),
            affected_resource: affected_resource.into(),
            evidence: Vec::new(),
            assumptions: Vec::new(),
            missing_data: Vec::new(),
            impact: String::new(),
            read_only_checks: Vec::new(),
            fix_plan: None,
            lifecycle: FindingLifecycle::Open,
            correlation_id: None,
            root_cause_finding_id: None,
            correlated_finding_ids: Vec::new(),
        }
    }

    pub fn with_evidence(mut self, source: &str, value: &str, note: &str) -> Self {
        self.evidence.push(EvidenceRef {
            source: source.into(),
            value: value.into(),
            note: note.into(),
        });
        self
    }

    pub fn with_impact(mut self, impact: impl Into<String>) -> Self {
        self.impact = impact.into();
        self
    }

    pub fn with_assumption(mut self, assumption: impl Into<String>) -> Self {
        self.assumptions.push(assumption.into());
        self
    }

    pub fn with_missing_data(mut self, data: impl Into<String>) -> Self {
        self.missing_data.push(data.into());
        self
    }

    pub fn with_read_only_check(mut self, check: impl Into<String>) -> Self {
        self.read_only_checks.push(check.into());
        self
    }

    pub fn fingerprint(&self) -> String {
        if self.fingerprint.is_empty() {
            compute_fingerprint(
                &self.detector_id,
                self.category,
                &self.affected_resource,
                &self.title,
            )
        } else {
            self.fingerprint.clone()
        }
    }

    pub fn clear_correlation(&mut self) {
        self.correlation_id = None;
        self.root_cause_finding_id = None;
        self.correlated_finding_ids.clear();
    }

    pub fn normalize_for_emission(
        mut self,
        fallback_detector_id: &str,
        detected_at: DateTime<Utc>,
    ) -> Self {
        self.detector_id = normalized_or_fallback(&self.detector_id, fallback_detector_id);
        self.affected_resource = normalized_or_fallback(&self.affected_resource, "system");
        self.snapshot_id = self.snapshot_id.trim().to_owned();
        self.detected_at = detected_at;
        self.title = normalize_title(&self.title, &self.affected_resource);
        self.evidence = normalize_evidence(
            &self.evidence,
            &self.detector_id,
            &self.affected_resource,
            &self.title,
        );
        if self.impact.trim().is_empty() {
            self.impact = format!("{} requires investigation.", self.affected_resource);
        } else {
            self.impact = collapse_whitespace(self.impact.trim());
        }
        self.fingerprint = self.fingerprint();
        self.id = compute_finding_id(
            &self.snapshot_id,
            &self.detector_id,
            &self.affected_resource,
        );
        self
    }

    pub fn validate_for_emission(&self) -> Result<(), Vec<FindingValidationError>> {
        let mut errors = Vec::new();
        if self.detector_id.trim().is_empty() {
            errors.push(FindingValidationError::MissingDetectorId);
        }
        if self.affected_resource.trim().is_empty() {
            errors.push(FindingValidationError::MissingAffectedResource);
        }
        if self.evidence.is_empty() {
            errors.push(FindingValidationError::MissingEvidence);
        }
        for (index, evidence) in self.evidence.iter().enumerate() {
            if evidence.source.trim().is_empty() {
                errors.push(FindingValidationError::MissingEvidenceSource(index));
            }
            if evidence.value.trim().is_empty() {
                errors.push(FindingValidationError::MissingEvidenceValue(index));
            }
        }
        let title = self.title.trim();
        if title.len() > 80 {
            errors.push(FindingValidationError::TitleTooLong(title.len()));
        }
        if !title_contains_resource(title, &self.affected_resource) {
            errors.push(FindingValidationError::TitleMissingResource);
        }
        if !title_is_plain_language(title) {
            errors.push(FindingValidationError::TitleNotPlainLanguage);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn title_is_readable(&self) -> bool {
        let title = self.title.trim();
        title.len() <= 80
            && title_contains_resource(title, &self.affected_resource)
            && title_is_plain_language(title)
    }
}

pub fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|left, right| {
        right
            .severity
            .cmp(&left.severity)
            .then(right.detected_at.cmp(&left.detected_at))
            .then(left.title.cmp(&right.title))
            .then(left.id.cmp(&right.id))
    });
}

pub fn self_resolved_findings(
    previous_active: &[Finding],
    current_active: &[Finding],
    snapshot_id: &str,
    detected_at: DateTime<Utc>,
) -> Vec<Finding> {
    let current_fingerprints: std::collections::HashSet<String> =
        current_active.iter().map(Finding::fingerprint).collect();
    let mut resolved = previous_active
        .iter()
        .filter(|finding| !finding.lifecycle.is_resolved())
        .filter(|finding| !current_fingerprints.contains(&finding.fingerprint()))
        .map(|finding| {
            let mut next = finding.clone();
            next.id = compute_finding_id(
                snapshot_id,
                &finding.detector_id,
                &finding.affected_resource,
            );
            next.snapshot_id = snapshot_id.to_owned();
            next.detected_at = detected_at;
            next.lifecycle = FindingLifecycle::SelfResolved;
            next.clear_correlation();
            next
        })
        .collect::<Vec<_>>();
    sort_findings(&mut resolved);
    resolved
}

fn compute_finding_id(snapshot_id: &str, detector_id: &str, affected_resource: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut h = std::collections::hash_map::DefaultHasher::new();
    snapshot_id.hash(&mut h);
    detector_id.hash(&mut h);
    affected_resource.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn compute_fingerprint(
    detector_id: &str,
    category: MonitorDomain,
    affected_resource: &str,
    title: &str,
) -> String {
    use std::hash::{Hash, Hasher};

    let mut h = std::collections::hash_map::DefaultHasher::new();
    detector_id.hash(&mut h);
    category.hash(&mut h);
    affected_resource.hash(&mut h);
    if detector_id.is_empty() {
        title.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn default_detected_at() -> DateTime<Utc> {
    Utc::now()
}

fn normalized_or_fallback(value: &str, fallback: &str) -> String {
    let trimmed = collapse_whitespace(value.trim());
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed
    }
}

fn normalize_evidence(
    evidence: &[EvidenceRef],
    detector_id: &str,
    affected_resource: &str,
    title: &str,
) -> Vec<EvidenceRef> {
    let mut normalized = evidence
        .iter()
        .filter_map(|item| {
            let source = collapse_whitespace(item.source.trim());
            let value = collapse_whitespace(item.value.trim());
            if source.is_empty() || value.is_empty() {
                return None;
            }
            let note = collapse_whitespace(item.note.trim());
            Some(EvidenceRef {
                source,
                value,
                note: if note.is_empty() {
                    format!("Evidence captured for {title}")
                } else {
                    note
                },
            })
        })
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        normalized.push(EvidenceRef {
            source: format!("detector.{detector_id}"),
            value: affected_resource.to_owned(),
            note: format!("Detector emitted finding for {title}"),
        });
    }
    normalized
}

fn normalize_title(title: &str, affected_resource: &str) -> String {
    let resource = normalized_or_fallback(affected_resource, "system");
    let mut cleaned = collapse_whitespace(
        title
            .trim()
            .trim_matches(':')
            .replace('_', " ")
            .replace("domains.", "")
            .replace("load_average", "load average")
            .replace("used_bytes", "used bytes")
            .replace(['[', ']'], " ")
            .as_str(),
    );
    if cleaned.is_empty() {
        cleaned = "needs attention".to_owned();
    }
    let mut candidate = if title_contains_resource(&cleaned, &resource) {
        cleaned
    } else {
        format!("{resource}: {cleaned}")
    };
    candidate = collapse_whitespace(candidate.trim());
    truncate_title(&candidate, 80)
}

fn truncate_title(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_owned();
    }
    let mut truncated = value
        .chars()
        .take(max_len.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

fn title_contains_resource(title: &str, affected_resource: &str) -> bool {
    let resource = affected_resource.trim();
    resource.is_empty()
        || resource == "system"
        || title.to_lowercase().contains(&resource.to_lowercase())
}

fn title_is_plain_language(title: &str) -> bool {
    let lowered = title.to_lowercase();
    ![
        "domains.",
        "filesystems[",
        "used_bytes",
        "load_average",
        "collector_errors",
    ]
    .iter()
    .any(|token| lowered.contains(token))
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use proptest::prelude::*;

    #[test]
    fn finding_builder_produces_valid_finding() {
        let f = Finding::new(
            "snap-1",
            "disk-usage",
            "/",
            "Root filesystem usage is high",
            Severity::Warning,
            Confidence::High,
            MonitorDomain::Disks,
        )
        .with_evidence(
            "disks.filesystems[/].used_bytes",
            "450G",
            "88% of 500G used",
        )
        .with_impact("System may run out of disk space soon")
        .with_read_only_check("du -sh /var/log/*");

        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.confidence, Confidence::High);
        assert_eq!(f.evidence.len(), 1);
        assert_eq!(f.read_only_checks.len(), 1);
        assert!(f.correlation_id.is_none());
        assert!(f.root_cause_finding_id.is_none());
        assert!(f.correlated_finding_ids.is_empty());
        assert!(f.detected_at <= Utc::now());
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Critical > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
    }

    #[test]
    fn monitor_domain_round_trips() {
        for domain in &[
            "disks",
            "services",
            "containers",
            "ports",
            "load",
            "logs",
            "backups",
        ] {
            let d = MonitorDomain::from_domain_str(domain).unwrap();
            assert_eq!(d.as_str(), *domain);
        }
    }

    #[test]
    fn normalize_for_emission_repairs_title_and_evidence() {
        let finding = Finding::new(
            "snap-1",
            "",
            "",
            "domains.load.load_average.one is high",
            Severity::Warning,
            Confidence::Medium,
            MonitorDomain::Load,
        )
        .normalize_for_emission("high-load", Utc::now());

        assert_eq!(finding.detector_id, "high-load");
        assert_eq!(finding.affected_resource, "system");
        assert!(!finding.evidence.is_empty());
        assert!(finding.title_is_readable());
        assert!(finding.validate_for_emission().is_ok());
    }

    proptest! {
        #[test]
        fn property_severity_ordering(
            inputs in proptest::collection::vec((0u8..=2, 0i64..10_000), 0..64)
        ) {
            let mut findings = inputs
                .into_iter()
                .enumerate()
                .map(|(index, (severity_code, ts))| {
                    let severity = match severity_code {
                        0 => Severity::Info,
                        1 => Severity::Warning,
                        _ => Severity::Critical,
                    };
                    let mut finding = Finding::new(
                        &format!("snap-{index}"),
                        "prop",
                        &format!("resource-{index}"),
                        "resource title",
                        severity,
                        Confidence::Medium,
                        MonitorDomain::Services,
                    );
                    finding.detected_at = Utc.timestamp_opt(ts, 0).single().unwrap();
                    finding
                })
                .collect::<Vec<_>>();

            sort_findings(&mut findings);

            for pair in findings.windows(2) {
                let left = &pair[0];
                let right = &pair[1];
                prop_assert!(
                    left.severity > right.severity
                        || (left.severity == right.severity && left.detected_at >= right.detected_at)
                );
            }
        }

        #[test]
        fn property_self_resolution(
            previous_count in 1usize..24,
            current_count in 0usize..24,
            snapshot_suffix in 1u32..5000
        ) {
            let previous = (0..previous_count)
                .map(|index| {
                    Finding::new(
                        "snap-prev",
                        "prop",
                        &format!("service-{index}"),
                        &format!("service-{index}: unhealthy"),
                        Severity::Warning,
                        Confidence::High,
                        MonitorDomain::Services,
                    )
                    .normalize_for_emission("prop", Utc::now())
                })
                .collect::<Vec<_>>();

            let current = (0..current_count.min(previous_count))
                .map(|index| {
                    Finding::new(
                        "snap-current",
                        "prop",
                        &format!("service-{index}"),
                        &format!("service-{index}: unhealthy"),
                        Severity::Warning,
                        Confidence::High,
                        MonitorDomain::Services,
                    )
                    .normalize_for_emission("prop", Utc::now())
                })
                .collect::<Vec<_>>();

            let resolved = self_resolved_findings(
                &previous,
                &current,
                &format!("snap-{snapshot_suffix}"),
                Utc::now(),
            );

            let expected = previous_count.saturating_sub(current.len());
            prop_assert_eq!(resolved.len(), expected);
            for finding in resolved {
                prop_assert_eq!(finding.lifecycle, FindingLifecycle::SelfResolved);
                prop_assert!(!current.iter().any(|active| active.fingerprint() == finding.fingerprint()));
            }
        }
    }
}
