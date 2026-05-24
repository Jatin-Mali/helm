//! Config drift detector: compares the current snapshot against a declared baseline.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::Value;

use crate::{
    detectors::Detector,
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

pub struct ConfigDriftDetector;

#[derive(Debug, Clone, Deserialize, Default)]
struct BaselineFile {
    #[serde(default, rename = "baseline")]
    entries: Vec<BaselineEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct BaselineEntry {
    key: String,
    metric_path: String,
    expected: toml::Value,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    impact: Option<String>,
}

impl Detector for ConfigDriftDetector {
    fn id(&self) -> &'static str {
        "config-drift"
    }

    fn domain(&self) -> MonitorDomain {
        MonitorDomain::Services
    }

    fn detect(&self, snap: &SystemSnapshot, _previous: Option<&SystemSnapshot>) -> Vec<Finding> {
        detect_with_baseline_path(snap, &default_config_baseline_path())
    }
}

pub fn default_config_baseline_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("helm")
        .join("baseline.toml")
}

fn detect_with_baseline_path(snapshot: &SystemSnapshot, path: &Path) -> Vec<Finding> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(file) = toml::from_str::<BaselineFile>(&content) else {
        return Vec::new();
    };
    if file.entries.is_empty() {
        return Vec::new();
    }

    let snapshot_value = serde_json::to_value(snapshot).unwrap_or(Value::Null);
    let mut findings = Vec::new();

    for entry in file.entries {
        if entry.key.trim().is_empty() || entry.metric_path.trim().is_empty() {
            continue;
        }

        let expected = toml_to_json_value(entry.expected);
        let actual = metric_value(&snapshot_value, &entry.metric_path).cloned();
        if actual.as_ref() == Some(&expected) {
            continue;
        }

        let domain = domain_from_metric_path(&entry.metric_path).unwrap_or(MonitorDomain::Services);
        let expected_text = value_to_text(&expected);
        let actual_text = actual
            .as_ref()
            .map(value_to_text)
            .unwrap_or_else(|| "<missing>".to_owned());
        let severity = severity_from_entry(entry.severity.as_deref());

        let mut finding =
            Finding::new(
                &snapshot.id,
                "config-drift",
                &entry.key,
                &format!("Configuration drift detected for {}", entry.key),
                severity,
                Confidence::High,
                domain,
            )
            .with_evidence(
                &entry.metric_path,
                &expected_text,
                "declared baseline value",
            )
            .with_evidence(&entry.metric_path, &actual_text, "observed current value")
            .with_impact(entry.impact.unwrap_or_else(|| {
                format!("{} no longer matches the declared baseline.", entry.key)
            }))
            .with_read_only_check(format!("review baseline key {}", entry.metric_path));

        if actual.is_none() {
            finding = finding.with_missing_data(format!(
                "metric path `{}` was not present in the current snapshot",
                entry.metric_path
            ));
        }
        findings.push(finding);
    }

    findings
}

fn severity_from_entry(value: Option<&str>) -> Severity {
    match value.unwrap_or("warning").to_ascii_lowercase().as_str() {
        "critical" => Severity::Critical,
        "info" => Severity::Info,
        _ => Severity::Warning,
    }
}

fn toml_to_json_value(value: toml::Value) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn metric_value<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    resolve_metric_path(value, path).or_else(|| {
        (!path.starts_with("domains.") && !path.starts_with("host."))
            .then(|| resolve_metric_path(value, &format!("domains.{path}")))
            .flatten()
    })
}

fn resolve_metric_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        let (field, index) = parse_segment(segment)?;
        current = current.get(field)?;
        if let Some(index) = index {
            current = current.get(index)?;
        }
    }
    Some(current)
}

fn parse_segment(segment: &str) -> Option<(&str, Option<usize>)> {
    if let Some((field, rest)) = segment.split_once('[') {
        let index = rest.strip_suffix(']')?.parse::<usize>().ok()?;
        Some((field, Some(index)))
    } else {
        Some((segment, None))
    }
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::Null => "<null>".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "<unrenderable>".to_owned()),
    }
}

fn domain_from_metric_path(path: &str) -> Option<MonitorDomain> {
    let root = path.strip_prefix("domains.").unwrap_or(path);
    let domain = root.split('.').next()?;
    MonitorDomain::from_domain_str(domain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        HostIdentity, MonitorProfile, SnapshotDomains, SystemSnapshot, SystemdUnit,
    };
    use chrono::Utc;
    use proptest::prelude::*;

    fn test_snapshot_with_service(active: &str) -> SystemSnapshot {
        let mut domains = SnapshotDomains::default();
        domains.services.units = vec![SystemdUnit {
            name: "nginx.service".to_owned(),
            load: "loaded".to_owned(),
            active: active.to_owned(),
            sub: active.to_owned(),
            description: "nginx".to_owned(),
        }];
        SystemSnapshot {
            id: "snap-1".into(),
            host: HostIdentity::default(),
            collected_at: Utc::now(),
            profile: MonitorProfile::Standard,
            domains,
            collector_errors: vec![],
            redaction_version: "0.1".into(),
        }
    }

    #[test]
    fn no_finding_without_baseline_file() {
        let snapshot = test_snapshot_with_service("active");
        let findings = detect_with_baseline_path(&snapshot, Path::new("/tmp/does-not-exist"));
        assert!(findings.is_empty());
    }

    #[test]
    fn detects_declared_service_drift() {
        let dir = tempfile::tempdir().unwrap();
        let baseline_path = dir.path().join("baseline.toml");
        fs::write(
            &baseline_path,
            r#"
[[baseline]]
key = "service.nginx.active"
metric_path = "services.units[0].active"
expected = "active"
severity = "critical"
"#,
        )
        .unwrap();

        let snapshot = test_snapshot_with_service("failed");
        let findings = detect_with_baseline_path(&snapshot, &baseline_path);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].title.contains("service.nginx.active"));
        assert_eq!(findings[0].severity, Severity::Critical);
        assert!(
            findings[0]
                .evidence
                .iter()
                .any(|item| item.value == "active")
        );
        assert!(
            findings[0]
                .evidence
                .iter()
                .any(|item| item.value == "failed")
        );
    }

    proptest! {
        #[test]
        fn property_32_config_drift_detection_includes_key_expected_actual(
            expected in prop_oneof![Just("active".to_owned()), Just("failed".to_owned()), Just("inactive".to_owned())],
            actual in prop_oneof![Just("active".to_owned()), Just("failed".to_owned()), Just("inactive".to_owned())]
        ) {
            let dir = tempfile::tempdir().unwrap();
            let baseline_path = dir.path().join("baseline.toml");
            fs::write(
                &baseline_path,
                format!(
                    "\n[[baseline]]\nkey = \"service.nginx.active\"\nmetric_path = \"services.units[0].active\"\nexpected = \"{}\"\n",
                    expected
                ),
            )
            .unwrap();

            let snapshot = test_snapshot_with_service(&actual);
            let findings = detect_with_baseline_path(&snapshot, &baseline_path);
            if expected == actual {
                prop_assert!(findings.is_empty());
            } else {
                prop_assert_eq!(findings.len(), 1);
                let finding = &findings[0];
                prop_assert_eq!(finding.affected_resource.as_str(), "service.nginx.active");
                prop_assert!(finding.evidence.iter().any(|item| item.value == expected.as_str()));
                prop_assert!(finding.evidence.iter().any(|item| item.value == actual.as_str()));
            }
        }
    }
}
