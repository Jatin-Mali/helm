use std::fmt;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::findings::{Finding, FindingLifecycle, MonitorDomain, Severity};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertPayload {
    pub fingerprint: String,
    pub severity: Severity,
    pub title: String,
    pub description: String,
    pub affected_resource: String,
    pub detector_id: String,
    pub category: MonitorDomain,
    pub timestamp: SystemTime,
    pub lifecycle: FindingLifecycle,
}

impl From<&Finding> for AlertPayload {
    fn from(f: &Finding) -> Self {
        let description = if !f.impact.is_empty() {
            f.impact.clone()
        } else if let Some(ev) = f.evidence.first() {
            format!("{}: {}", ev.source, ev.value)
        } else {
            f.title.clone()
        };
        Self {
            fingerprint: f.fingerprint(),
            severity: f.severity,
            title: f.title.clone(),
            description,
            affected_resource: f.affected_resource.clone(),
            detector_id: f.detector_id.clone(),
            category: f.category,
            timestamp: SystemTime::now(),
            lifecycle: f.lifecycle,
        }
    }
}

impl fmt::Display for AlertPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} — {} ({})",
            self.severity.as_str().to_uppercase(),
            self.title,
            self.affected_resource,
            self.fingerprint,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::{Confidence, Finding};

    fn make_finding() -> Finding {
        Finding::new(
            "snap-1",
            "test-detector",
            "host/service",
            "Test alert",
            Severity::Critical,
            Confidence::High,
            MonitorDomain::Services,
        )
    }

    #[test]
    fn payload_from_finding() {
        let f = make_finding();
        let p = AlertPayload::from(&f);
        assert_eq!(p.severity, Severity::Critical);
        assert_eq!(p.title, "Test alert");
        assert_eq!(p.lifecycle, FindingLifecycle::Open);
        assert!(!p.fingerprint.is_empty());
    }

    #[test]
    fn payload_display() {
        let f = make_finding();
        let p = AlertPayload::from(&f);
        let s = p.to_string();
        assert!(s.contains("CRITICAL"));
        assert!(s.contains("Test alert"));
    }

    #[test]
    fn resolved_lifecycle() {
        let mut f = make_finding();
        f.lifecycle = FindingLifecycle::Resolved;
        let p = AlertPayload::from(&f);
        assert!(p.lifecycle.is_resolved());
    }
}
