//! Custom detection rules loaded from `rules.toml`.

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{
    findings::{Confidence, Finding, MonitorDomain, Severity},
    snapshot::SystemSnapshot,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomRule {
    pub id: String,
    pub title: String,
    pub severity: Severity,
    pub condition: RuleCondition,
    pub impact: String,
    pub fix_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuleCondition {
    Simple {
        metric_path: String,
        operator: CompareOp,
        threshold: ThresholdValue,
    },
    Compound {
        logic: LogicOp,
        conditions: Vec<RuleCondition>,
    },
    Rate {
        metric_path: String,
        count_threshold: u32,
        window_seconds: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    Gt,
    Lt,
    Eq,
    Ne,
    Contains,
    Matches,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicOp {
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ThresholdValue {
    Numeric(f64),
    Text(String),
    Regex(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleValidationError {
    pub line: usize,
    pub description: String,
}

#[derive(Debug, Default)]
pub struct CustomRulesEngine {
    rules: Vec<CustomRule>,
    config_path: Option<PathBuf>,
    last_modified: Option<std::time::SystemTime>,
    rate_history: Mutex<HashMap<String, VecDeque<DateTime<Utc>>>>,
}

impl CustomRulesEngine {
    pub fn load(path: &Path) -> Result<Self, Vec<RuleValidationError>> {
        let content = std::fs::read_to_string(path).map_err(|error| {
            vec![RuleValidationError {
                line: 1,
                description: format!("failed to read {}: {error}", path.display()),
            }]
        })?;
        let (rules, errors) = parse_rules_document_partial(&content)?;
        if !errors.is_empty() {
            return Err(errors);
        }
        let last_modified = std::fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok());
        Ok(Self {
            rules,
            config_path: Some(path.to_path_buf()),
            last_modified,
            rate_history: Mutex::new(HashMap::new()),
        })
    }

    pub fn load_default() -> Self {
        let path = default_rules_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let (rules, _) = parse_rules_document_partial(&content).unwrap_or_default();
                let last_modified = std::fs::metadata(&path)
                    .ok()
                    .and_then(|meta| meta.modified().ok());
                Self {
                    rules,
                    config_path: Some(path),
                    last_modified,
                    rate_history: Mutex::new(HashMap::new()),
                }
            }
            Err(_) => Self {
                rules: Vec::new(),
                config_path: Some(path),
                last_modified: None,
                rate_history: Mutex::new(HashMap::new()),
            },
        }
    }

    pub fn rules(&self) -> &[CustomRule] {
        &self.rules
    }

    pub fn evaluate(&self, snapshot: &SystemSnapshot) -> Vec<Finding> {
        let snapshot_value = serde_json::to_value(snapshot).unwrap_or_default();
        self.rules
            .iter()
            .filter_map(|rule| {
                if condition_matches(
                    &rule.condition,
                    &snapshot_value,
                    snapshot.collected_at,
                    &self.rate_history,
                ) {
                    Some(build_finding(rule, snapshot, &snapshot_value))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn reload(&mut self, path: &Path) -> Result<(), Vec<RuleValidationError>> {
        let modified = std::fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok());
        if modified.is_some() && modified == self.last_modified {
            return Ok(());
        }
        let content = std::fs::read_to_string(path).map_err(|error| {
            vec![RuleValidationError {
                line: 1,
                description: format!("failed to read {}: {error}", path.display()),
            }]
        })?;
        let (rules, errors) = parse_rules_document_partial(&content)?;
        self.rules = rules;
        self.config_path = Some(path.to_path_buf());
        self.last_modified = modified;
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

pub fn default_rules_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("helm")
        .join("rules.toml")
}

#[derive(Debug, Default, Deserialize)]
struct CustomRulesFileRaw {
    #[serde(default)]
    rules: Vec<CustomRuleRaw>,
}

#[derive(Debug, Deserialize)]
struct CustomRuleRaw {
    id: Option<String>,
    title: Option<String>,
    severity: Option<String>,
    condition: Option<RuleConditionRaw>,
    impact: Option<String>,
    fix_hint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum RuleConditionRaw {
    Simple {
        metric_path: String,
        operator: String,
        threshold: ThresholdValueRaw,
    },
    Compound {
        logic: String,
        conditions: Vec<RuleConditionRaw>,
    },
    Rate {
        metric_path: String,
        count_threshold: u32,
        window_seconds: u64,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ThresholdValueRaw {
    Numeric(f64),
    Text(String),
    Detailed {
        numeric: Option<f64>,
        text: Option<String>,
        regex: Option<String>,
    },
}

fn parse_rules_document_partial(
    content: &str,
) -> Result<(Vec<CustomRule>, Vec<RuleValidationError>), Vec<RuleValidationError>> {
    let raw = toml::from_str::<CustomRulesFileRaw>(content).map_err(|error| {
        vec![RuleValidationError {
            line: toml_error_line(content, &error).unwrap_or(1),
            description: format!("TOML syntax error: {error}"),
        }]
    })?;

    let anchors = rule_anchor_lines(content);
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    for (index, raw_rule) in raw.rules.into_iter().enumerate() {
        let line = anchors.get(index).copied().unwrap_or(1);
        match validate_rule(raw_rule, line) {
            Ok(rule) => rules.push(rule),
            Err(mut validation_errors) => errors.append(&mut validation_errors),
        }
    }

    Ok((rules, errors))
}

fn validate_rule(raw: CustomRuleRaw, line: usize) -> Result<CustomRule, Vec<RuleValidationError>> {
    let mut errors = Vec::new();
    let id = require_text(raw.id, line, "missing rule id", &mut errors);
    let title = require_text(raw.title, line, "missing rule title", &mut errors);
    let impact = require_text(raw.impact, line, "missing rule impact", &mut errors);
    let severity = match raw.severity.as_deref() {
        Some("info") => Some(Severity::Info),
        Some("warning") => Some(Severity::Warning),
        Some("critical") => Some(Severity::Critical),
        Some(other) => {
            errors.push(RuleValidationError {
                line,
                description: format!("invalid severity `{other}`; expected info|warning|critical"),
            });
            None
        }
        None => {
            errors.push(RuleValidationError {
                line,
                description: "missing rule severity".to_owned(),
            });
            None
        }
    };
    let condition = match raw.condition {
        Some(raw_condition) => validate_condition(raw_condition, line, &mut errors),
        None => {
            errors.push(RuleValidationError {
                line,
                description: "missing rule condition".to_owned(),
            });
            None
        }
    };

    if errors.is_empty() {
        Ok(CustomRule {
            id: id.unwrap_or_default(),
            title: title.unwrap_or_default(),
            severity: severity.unwrap_or(Severity::Warning),
            condition: condition.unwrap_or_else(|| RuleCondition::Compound {
                logic: LogicOp::And,
                conditions: Vec::new(),
            }),
            impact: impact.unwrap_or_default(),
            fix_hint: raw.fix_hint.filter(|value| !value.trim().is_empty()),
        })
    } else {
        Err(errors)
    }
}

fn validate_condition(
    raw: RuleConditionRaw,
    line: usize,
    errors: &mut Vec<RuleValidationError>,
) -> Option<RuleCondition> {
    match raw {
        RuleConditionRaw::Simple {
            metric_path,
            operator,
            threshold,
        } => {
            let operator = match operator.as_str() {
                "gt" => Some(CompareOp::Gt),
                "lt" => Some(CompareOp::Lt),
                "eq" => Some(CompareOp::Eq),
                "ne" => Some(CompareOp::Ne),
                "contains" => Some(CompareOp::Contains),
                "matches" => Some(CompareOp::Matches),
                other => {
                    errors.push(RuleValidationError {
                        line,
                        description: format!(
                            "invalid operator `{other}`; expected gt|lt|eq|ne|contains|matches"
                        ),
                    });
                    None
                }
            };
            let threshold = validate_threshold(threshold, line, errors);
            if metric_path.trim().is_empty() {
                errors.push(RuleValidationError {
                    line,
                    description: "metric_path cannot be empty".to_owned(),
                });
            }
            match (operator, threshold) {
                (Some(operator), Some(threshold)) => Some(RuleCondition::Simple {
                    metric_path,
                    operator,
                    threshold,
                }),
                _ => None,
            }
        }
        RuleConditionRaw::Compound { logic, conditions } => {
            let logic = match logic.as_str() {
                "and" => Some(LogicOp::And),
                "or" => Some(LogicOp::Or),
                other => {
                    errors.push(RuleValidationError {
                        line,
                        description: format!("invalid logic `{other}`; expected and|or"),
                    });
                    None
                }
            };
            let validated = conditions
                .into_iter()
                .filter_map(|condition| validate_condition(condition, line, errors))
                .collect::<Vec<_>>();
            if validated.is_empty() {
                errors.push(RuleValidationError {
                    line,
                    description: "compound rule requires at least one valid condition".to_owned(),
                });
            }
            logic.map(|logic| RuleCondition::Compound {
                logic,
                conditions: validated,
            })
        }
        RuleConditionRaw::Rate {
            metric_path,
            count_threshold,
            window_seconds,
        } => {
            if metric_path.trim().is_empty() {
                errors.push(RuleValidationError {
                    line,
                    description: "metric_path cannot be empty".to_owned(),
                });
            }
            if count_threshold == 0 {
                errors.push(RuleValidationError {
                    line,
                    description: "count_threshold must be greater than zero".to_owned(),
                });
            }
            if window_seconds == 0 {
                errors.push(RuleValidationError {
                    line,
                    description: "window_seconds must be greater than zero".to_owned(),
                });
            }
            if metric_path.trim().is_empty() || count_threshold == 0 || window_seconds == 0 {
                None
            } else {
                Some(RuleCondition::Rate {
                    metric_path,
                    count_threshold,
                    window_seconds,
                })
            }
        }
    }
}

fn validate_threshold(
    raw: ThresholdValueRaw,
    line: usize,
    errors: &mut Vec<RuleValidationError>,
) -> Option<ThresholdValue> {
    match raw {
        ThresholdValueRaw::Numeric(value) => Some(ThresholdValue::Numeric(value)),
        ThresholdValueRaw::Text(value) => Some(ThresholdValue::Text(value)),
        ThresholdValueRaw::Detailed {
            numeric,
            text,
            regex,
        } => {
            if let Some(value) = numeric {
                Some(ThresholdValue::Numeric(value))
            } else if let Some(value) = regex {
                Some(ThresholdValue::Regex(value))
            } else if let Some(value) = text {
                Some(ThresholdValue::Text(value))
            } else {
                errors.push(RuleValidationError {
                    line,
                    description: "threshold must include numeric, text, or regex".to_owned(),
                });
                None
            }
        }
    }
}

fn require_text(
    value: Option<String>,
    line: usize,
    message: &str,
    errors: &mut Vec<RuleValidationError>,
) -> Option<String> {
    match value {
        Some(value) if !value.trim().is_empty() => Some(value),
        _ => {
            errors.push(RuleValidationError {
                line,
                description: message.to_owned(),
            });
            None
        }
    }
}

fn condition_matches(
    condition: &RuleCondition,
    snapshot: &serde_json::Value,
    collected_at: DateTime<Utc>,
    rate_history: &Mutex<HashMap<String, VecDeque<DateTime<Utc>>>>,
) -> bool {
    match condition {
        RuleCondition::Simple {
            metric_path,
            operator,
            threshold,
        } => evaluate_simple_condition(snapshot, metric_path, *operator, threshold),
        RuleCondition::Compound { logic, conditions } => match logic {
            LogicOp::And => conditions
                .iter()
                .all(|rule| condition_matches(rule, snapshot, collected_at, rate_history)),
            LogicOp::Or => conditions
                .iter()
                .any(|rule| condition_matches(rule, snapshot, collected_at, rate_history)),
        },
        RuleCondition::Rate {
            metric_path,
            count_threshold,
            window_seconds,
        } => evaluate_rate_condition(
            snapshot,
            metric_path,
            *count_threshold,
            *window_seconds,
            collected_at,
            rate_history,
        ),
    }
}

fn evaluate_simple_condition(
    snapshot: &serde_json::Value,
    metric_path: &str,
    operator: CompareOp,
    threshold: &ThresholdValue,
) -> bool {
    let Some(value) = metric_value(snapshot, metric_path) else {
        return false;
    };
    match operator {
        CompareOp::Gt => value.as_f64().is_some_and(
            |current| matches!(threshold, ThresholdValue::Numeric(limit) if current > *limit),
        ),
        CompareOp::Lt => value.as_f64().is_some_and(
            |current| matches!(threshold, ThresholdValue::Numeric(limit) if current < *limit),
        ),
        CompareOp::Eq => match threshold {
            ThresholdValue::Numeric(limit) => value
                .as_f64()
                .is_some_and(|current| (current - *limit).abs() < f64::EPSILON),
            ThresholdValue::Text(expected) => {
                metric_as_string(value).is_some_and(|current| current == *expected)
            }
            ThresholdValue::Regex(pattern) => {
                metric_as_string(value).is_some_and(|current| current == *pattern)
            }
        },
        CompareOp::Ne => match threshold {
            ThresholdValue::Numeric(limit) => value
                .as_f64()
                .is_some_and(|current| (current - *limit).abs() >= f64::EPSILON),
            ThresholdValue::Text(expected) => {
                metric_as_string(value).is_some_and(|current| current != *expected)
            }
            ThresholdValue::Regex(pattern) => {
                metric_as_string(value).is_some_and(|current| current != *pattern)
            }
        },
        CompareOp::Contains => match threshold {
            ThresholdValue::Text(needle) | ThresholdValue::Regex(needle) => {
                metric_as_string(value).is_some_and(|current| current.contains(needle))
            }
            ThresholdValue::Numeric(_) => false,
        },
        CompareOp::Matches => match threshold {
            ThresholdValue::Text(pattern) | ThresholdValue::Regex(pattern) => Regex::new(pattern)
                .ok()
                .and_then(|regex| metric_as_string(value).map(|current| regex.is_match(&current)))
                .unwrap_or(false),
            ThresholdValue::Numeric(_) => false,
        },
    }
}

fn evaluate_rate_condition(
    snapshot: &serde_json::Value,
    metric_path: &str,
    count_threshold: u32,
    window_seconds: u64,
    collected_at: DateTime<Utc>,
    rate_history: &Mutex<HashMap<String, VecDeque<DateTime<Utc>>>>,
) -> bool {
    let Some(value) = metric_value(snapshot, metric_path) else {
        return false;
    };
    if !metric_is_event(value) {
        return false;
    }

    let mut guard = rate_history
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let entry = guard.entry(metric_path.to_owned()).or_default();
    entry.push_back(collected_at);
    while let Some(front) = entry.front().copied() {
        if collected_at.signed_duration_since(front).num_seconds() > window_seconds as i64 {
            entry.pop_front();
        } else {
            break;
        }
    }
    entry.len() as u32 >= count_threshold
}

fn build_finding(
    rule: &CustomRule,
    snapshot: &SystemSnapshot,
    snapshot_value: &serde_json::Value,
) -> Finding {
    let resource = first_metric_path(&rule.condition)
        .and_then(domain_from_metric_path)
        .map(|domain| domain.as_str().to_owned())
        .unwrap_or_else(|| "custom rule".to_owned());
    let mut finding = Finding::new(
        &snapshot.id,
        &format!("custom-rule:{}", rule.id),
        &resource,
        &rule.title,
        rule.severity,
        Confidence::Medium,
        infer_domain(rule),
    )
    .with_impact(rule.impact.clone())
    .with_assumption(format!("Triggered by custom rule `{}`", rule.id));

    if let Some(metric_path) = first_metric_path(&rule.condition)
        && let Some(value) = metric_value(snapshot_value, metric_path)
    {
        finding = finding.with_evidence(
            metric_path,
            &metric_as_string(value).unwrap_or_else(|| value.to_string()),
            "Matched custom rule condition",
        );
    }
    if let Some(fix_hint) = &rule.fix_hint {
        finding.fix_plan = Some(fix_hint.clone());
    }
    finding.normalize_for_emission(&format!("custom-rule:{}", rule.id), snapshot.collected_at)
}

fn infer_domain(rule: &CustomRule) -> MonitorDomain {
    first_metric_path(&rule.condition)
        .and_then(domain_from_metric_path)
        .unwrap_or(MonitorDomain::Load)
}

fn first_metric_path(condition: &RuleCondition) -> Option<&str> {
    match condition {
        RuleCondition::Simple { metric_path, .. } => Some(metric_path.as_str()),
        RuleCondition::Rate { metric_path, .. } => Some(metric_path.as_str()),
        RuleCondition::Compound { conditions, .. } => conditions.iter().find_map(first_metric_path),
    }
}

fn metric_value<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    resolve_metric_path(value, path).or_else(|| {
        (!path.starts_with("domains."))
            .then(|| resolve_metric_path(value, &format!("domains.{path}")))
            .flatten()
    })
}

fn resolve_metric_path<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        let (field, index) = parse_segment(segment);
        current = current.get(field)?;
        if let Some(index) = index {
            current = current.get(index)?;
        }
    }
    Some(current)
}

fn parse_segment(segment: &str) -> (&str, Option<usize>) {
    if let Some((field, rest)) = segment.split_once('[') {
        (field, rest.trim_end_matches(']').parse::<usize>().ok())
    } else {
        (segment, None)
    }
}

fn metric_as_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn domain_from_metric_path(path: &str) -> Option<MonitorDomain> {
    let mut segments = path.split('.');
    let first = segments.next()?;
    let domain_segment = if first == "domains" {
        segments.next().unwrap_or(first)
    } else {
        first
    };
    MonitorDomain::from_domain_str(domain_segment)
}

fn metric_is_event(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(value) => value.as_f64().is_some_and(|current| current > 0.0),
        serde_json::Value::String(value) => !value.trim().is_empty() && value != "0",
        serde_json::Value::Array(value) => !value.is_empty(),
        serde_json::Value::Object(value) => !value.is_empty(),
    }
}

fn rule_anchor_lines(content: &str) -> Vec<usize> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| (line.trim() == "[[rules]]").then_some(index + 1))
        .collect()
}

fn toml_error_line(content: &str, error: &toml::de::Error) -> Option<usize> {
    let span = error.span()?;
    let prefix = &content[..span.start.min(content.len())];
    Some(prefix.chars().filter(|c| *c == '\n').count() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::*;

    #[test]
    fn parses_and_evaluates_simple_rule() {
        let (rules, errors) = parse_rules_document_partial(
            r#"
[[rules]]
id = "root-disk-hot"
title = "Root disk almost full"
severity = "critical"
impact = "Writes may fail"
fix_hint = "Free space on root"

[rules.condition]
kind = "simple"
metric_path = "domains.disks.filesystems[0].used_bytes"
operator = "gt"
threshold = 90
"#,
        )
        .unwrap();
        assert!(errors.is_empty());
        let engine = CustomRulesEngine {
            rules,
            config_path: None,
            last_modified: None,
            rate_history: Mutex::new(HashMap::new()),
        };
        let findings = engine.evaluate(&snapshot_with_metrics(4.0, 20, 1));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert!(findings[0].validate_for_emission().is_ok());
        assert!(findings[0].title_is_readable());
    }

    #[test]
    fn compound_and_rate_rules_trigger() {
        let (rules, errors) = parse_rules_document_partial(
            r#"
[[rules]]
id = "pressure-combo"
title = "Pressure stack"
severity = "warning"
impact = "Multiple signals are elevated"

[rules.condition]
kind = "compound"
logic = "and"
conditions = [
  { kind = "simple", metric_path = "domains.load.load_average.one", operator = "gt", threshold = 5 },
  { kind = "simple", metric_path = "domains.load.swap_used", operator = "gt", threshold = 10 }
]

[[rules]]
id = "error-burst"
title = "Journal error burst"
severity = "warning"
impact = "Errors are recurring quickly"

[rules.condition]
kind = "rate"
metric_path = "domains.logs.journal_errors_last_hour"
count_threshold = 2
window_seconds = 300
"#,
        )
        .unwrap();
        assert!(errors.is_empty());
        let engine = CustomRulesEngine {
            rules,
            config_path: None,
            last_modified: None,
            rate_history: Mutex::new(HashMap::new()),
        };

        let first = engine.evaluate(&snapshot_with_metrics(6.0, 20, 5));
        assert_eq!(first.len(), 1);

        let second = engine.evaluate(&snapshot_with_metrics(6.0, 20, 5));
        assert_eq!(second.len(), 2);
        assert!(
            second
                .iter()
                .any(|finding| finding.title.contains("Pressure stack"))
        );
        assert!(
            second
                .iter()
                .any(|finding| finding.title.contains("Journal error burst"))
        );
        assert!(
            second
                .iter()
                .all(|finding| finding.validate_for_emission().is_ok())
        );
        assert!(second.iter().all(|finding| finding.title_is_readable()));
    }

    #[test]
    fn validation_errors_include_line_numbers() {
        let (_, errors) = parse_rules_document_partial(
            r#"
[[rules]]
id = "broken"
title = "Broken rule"
severity = "urgent"
impact = "bad"

[rules.condition]
kind = "simple"
metric_path = "domains.load.load_average.one"
operator = "gte"
threshold = 10
"#,
        )
        .unwrap();

        assert!(errors.iter().all(|error| error.line >= 2));
        assert!(
            errors
                .iter()
                .any(|error| error.description.contains("invalid severity"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.description.contains("invalid operator"))
        );
    }

    fn snapshot_with_metrics(load_one: f64, swap_used: u64, journal_errors: u64) -> SystemSnapshot {
        let mut domains = SnapshotDomains::default();
        domains.load.cpu_logical_count = 4;
        domains.load.load_average.one = load_one;
        domains.load.swap_total = 100;
        domains.load.swap_used = swap_used;
        domains.logs.journal_errors_last_hour = journal_errors;
        domains.disks.filesystems.push(FilesystemEntry {
            device: "/dev/sda1".into(),
            mount_point: "/".into(),
            fs_type: "ext4".into(),
            total_bytes: 100,
            used_bytes: 95,
            available_bytes: 5,
        });
        SystemSnapshot {
            id: "snap-1".into(),
            host: HostIdentity::default(),
            collected_at: Utc::now(),
            profile: MonitorProfile::Standard,
            domains,
            collector_errors: Vec::new(),
            redaction_version: "test".into(),
        }
    }
}
