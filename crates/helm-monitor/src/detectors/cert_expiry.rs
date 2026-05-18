//! Certificate expiry detector: warns when TLS certificates are near expiry.
//! Per TRD v2.2: certificate expiry checking for listening services.

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct CertExpiryDetector;

impl Detector for CertExpiryDetector {
    fn id(&self) -> &'static str {
        "cert-expiry"
    }

    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Certs
    }

    fn detect(
        &self,
        snapshot: &SystemSnapshot,
        _previous: Option<&SystemSnapshot>,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();

        for cert in &snapshot.domains.certs.certificates {
            // Skip entries with connection errors
            if !cert.error.is_empty() {
                continue;
            }

            let severity = if cert.days_remaining <= 7 {
                Severity::Critical
            } else if cert.days_remaining <= 30 {
                Severity::Warning
            } else if cert.days_remaining <= 60 {
                Severity::Info
            } else {
                continue; // No finding for healthy certs
            };

            let title = format!(
                "TLS certificate for {} expires in {} days",
                cert.address, cert.days_remaining
            );

            let mut summary = format!(
                "Subject: {}\nIssuer: {}\nNot after: {}\nDays remaining: {}",
                cert.subject_cn, cert.issuer_cn, cert.not_after, cert.days_remaining
            );

            if cert.self_signed {
                summary.push_str("\n⚠ Self-signed certificate");
            }

            let mut f = Finding::new(
                &snapshot.id,
                self.id(),
                &cert.address,
                &title,
                severity,
                if cert.days_remaining <= 14 {
                    Confidence::High
                } else {
                    Confidence::Medium
                },
                MonitorDomain::Certs,
            )
            .with_evidence(
                &format!("certs.certificates[{}]", cert.address),
                &summary,
                "TLS certificate approaching expiry",
            )
            .with_impact(format!(
                "Service at {} will be unreachable via TLS after {} unless renewed",
                cert.address, cert.not_after
            ));

            if severity >= Severity::Warning {
                let renewal_hint = if cert.self_signed {
                    "Regenerate self-signed cert or replace with CA-signed"
                } else {
                    "Renew with CA (certbot/letsencrypt, or provider-managed)"
                };
                f = f.with_read_only_check(format!(
                    "echo | openssl s_client -connect {} 2>/dev/null | openssl x509 -noout -dates",
                    cert.address
                ));
                f = f.with_assumption(format!("Renewal needed: {}", renewal_hint));
            }

            findings.push(f);
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{CertEntry, CertSnapshot, SnapshotDomains, SystemSnapshot};

    fn test_snapshot(certs: Vec<CertEntry>) -> SystemSnapshot {
        let mut domains = SnapshotDomains::default();
        domains.certs = CertSnapshot {
            certificates: certs,
            openssl_available: true,
        };
        SystemSnapshot {
            id: "test-1".into(),
            host: crate::snapshot::HostIdentity::default(),
            collected_at: chrono::Utc::now(),
            profile: crate::snapshot::MonitorProfile::Standard,
            domains,
            collector_errors: Vec::new(),
            redaction_version: "0.1.0".into(),
        }
    }

    #[test]
    fn detects_critical_expiry() {
        let snapshot = test_snapshot(vec![CertEntry {
            address: "example.com:443".into(),
            subject_cn: "example.com".into(),
            issuer_cn: "Let's Encrypt".into(),
            not_after: "May 19 2026 GMT".into(),
            days_remaining: 5,
            sans: vec!["DNS:example.com".into()],
            self_signed: false,
            error: String::new(),
        }]);
        let detector = CertExpiryDetector;
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0].severity, Severity::Critical));
    }

    #[test]
    fn detects_warning_expiry() {
        let snapshot = test_snapshot(vec![CertEntry {
            address: "api.example.com:443".into(),
            subject_cn: "api.example.com".into(),
            issuer_cn: "Let's Encrypt".into(),
            not_after: "Jun 15 2026 GMT".into(),
            days_remaining: 21,
            sans: Vec::new(),
            self_signed: false,
            error: String::new(),
        }]);
        let detector = CertExpiryDetector;
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0].severity, Severity::Warning));
    }

    #[test]
    fn ignores_healthy_cert() {
        let snapshot = test_snapshot(vec![CertEntry {
            address: "safe.example.com:443".into(),
            subject_cn: "safe.example.com".into(),
            issuer_cn: "Let's Encrypt".into(),
            not_after: "Dec 01 2027 GMT".into(),
            days_remaining: 500,
            sans: Vec::new(),
            self_signed: false,
            error: String::new(),
        }]);
        let detector = CertExpiryDetector;
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn ignores_error_entries() {
        let snapshot = test_snapshot(vec![CertEntry {
            address: "broken:443".into(),
            subject_cn: String::new(),
            issuer_cn: String::new(),
            not_after: String::new(),
            days_remaining: i64::MAX,
            sans: Vec::new(),
            self_signed: false,
            error: "connection refused".into(),
        }]);
        let detector = CertExpiryDetector;
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 0);
    }

    #[test]
    fn flags_self_signed_in_summary() {
        let snapshot = test_snapshot(vec![CertEntry {
            address: "self.example.com:443".into(),
            subject_cn: "self.example.com".into(),
            issuer_cn: "self.example.com".into(),
            not_after: "May 25 2026 GMT".into(),
            days_remaining: 10,
            sans: Vec::new(),
            self_signed: true,
            error: String::new(),
        }]);
        let detector = CertExpiryDetector;
        let findings = detector.detect(&snapshot, None);
        assert_eq!(findings.len(), 1);
        assert!(findings[0]
            .evidence
            .iter()
            .any(|e| e.value.to_lowercase().contains("self-signed")));
    }
}
