//! Finding model — the output of detectors. Per TRD §4.2.
#![allow(clippy::needless_borrows_for_generic_args)]

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
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        snapshot_id.hash(&mut h);
        detector_id.hash(&mut h);
        affected_resource.hash(&mut h);
        let hash = h.finish();
        let fingerprint = compute_fingerprint(detector_id, category, affected_resource, title);
        Self {
            id: format!("{:016x}", hash),
            detector_id: detector_id.into(),
            fingerprint,
            snapshot_id: snapshot_id.into(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
