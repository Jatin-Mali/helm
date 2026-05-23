use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::findings::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkKind {
    Webhook,
    Slack,
    #[serde(rename = "pagerduty")]
    PagerDuty,
}

impl SinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Slack => "slack",
            Self::PagerDuty => "pagerduty",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlertConfig {
    pub min_severity: Severity,
    pub dedup_window_secs: u64,
    pub rate_limit_per_min: u32,
    pub sinks: Vec<AlertSinkConfig>,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            min_severity: default_min_severity(),
            dedup_window_secs: default_dedup_window(),
            rate_limit_per_min: default_rate_limit(),
            sinks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AlertSinkConfig {
    Webhook {
        #[serde(flatten)]
        policy: SinkPolicyConfig,
        url: String,
        retry_max: u32,
        retry_backoff_ms: u64,
    },
    Slack {
        #[serde(flatten)]
        policy: SinkPolicyConfig,
        webhook_url: String,
        channel: Option<String>,
    },
    #[serde(rename = "pagerduty")]
    PagerDuty {
        #[serde(flatten)]
        policy: SinkPolicyConfig,
        routing_key: String,
    },
}

impl AlertSinkConfig {
    pub fn kind(&self) -> SinkKind {
        match self {
            Self::Webhook { .. } => SinkKind::Webhook,
            Self::Slack { .. } => SinkKind::Slack,
            Self::PagerDuty { .. } => SinkKind::PagerDuty,
        }
    }

    pub fn policy(&self) -> &SinkPolicyConfig {
        match self {
            Self::Webhook { policy, .. }
            | Self::Slack { policy, .. }
            | Self::PagerDuty { policy, .. } => policy,
        }
    }

    pub fn name(&self) -> String {
        format!("{}-sink", self.kind().as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct SinkPolicyConfig {
    pub min_severity: Option<Severity>,
    pub dedup_window_secs: Option<u64>,
    pub rate_limit_per_min: Option<u32>,
}

impl SinkPolicyConfig {
    pub fn resolve(&self, config: &AlertConfig) -> ResolvedSinkPolicy {
        ResolvedSinkPolicy {
            min_severity: self.min_severity.unwrap_or(config.min_severity),
            dedup_window_secs: self.dedup_window_secs.unwrap_or(config.dedup_window_secs),
            rate_limit_per_min: self.rate_limit_per_min.unwrap_or(config.rate_limit_per_min),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedSinkPolicy {
    pub min_severity: Severity,
    pub dedup_window_secs: u64,
    pub rate_limit_per_min: u32,
}

#[derive(Deserialize, Default)]
struct TomlRoot {
    #[serde(default)]
    config: Option<AlertConfigRaw>,
    #[serde(default)]
    alerting: Option<AlertConfigRaw>,
    #[serde(default, rename = "sink")]
    sinks: Vec<AlertSinkConfigRaw>,
}

#[derive(Debug, Deserialize, Default)]
struct AlertConfigRaw {
    min_severity: Option<String>,
    dedup_window_secs: Option<u64>,
    rate_limit_per_min: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AlertSinkConfigRaw {
    Webhook {
        #[serde(flatten)]
        policy: SinkPolicyRaw,
        url: String,
        retry_max: Option<u32>,
        retry_backoff_ms: Option<u64>,
    },
    Slack {
        #[serde(flatten)]
        policy: SinkPolicyRaw,
        webhook_url: String,
        channel: Option<String>,
    },
    #[serde(rename = "pagerduty")]
    PagerDuty {
        #[serde(flatten)]
        policy: SinkPolicyRaw,
        routing_key: String,
    },
}

#[derive(Debug, Deserialize, Default)]
struct SinkPolicyRaw {
    min_severity: Option<String>,
    dedup_window_secs: Option<u64>,
    rate_limit_per_min: Option<u32>,
}

pub fn default_config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            PathBuf::from(home).join(".config")
        });
    base.join("helm").join("alerting.toml")
}

pub fn load_config() -> Result<AlertConfig, Box<dyn std::error::Error>> {
    load_config_from_path(&default_config_path())
}

pub fn load_config_from_path(path: &Path) -> Result<AlertConfig, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(AlertConfig::default());
    }

    let content = std::fs::read_to_string(path)?;
    let root: TomlRoot = toml::from_str(&content)?;
    let raw_config = root.config.or(root.alerting).unwrap_or_default();

    let mut config = AlertConfig {
        min_severity: parse_severity(raw_config.min_severity.as_deref())
            .unwrap_or_else(default_min_severity),
        dedup_window_secs: raw_config
            .dedup_window_secs
            .unwrap_or_else(default_dedup_window),
        rate_limit_per_min: raw_config
            .rate_limit_per_min
            .unwrap_or_else(default_rate_limit),
        sinks: Vec::with_capacity(root.sinks.len()),
    };

    for sink in root.sinks {
        config.sinks.push(map_sink_config(sink)?);
    }

    Ok(config)
}

fn map_sink_config(raw: AlertSinkConfigRaw) -> Result<AlertSinkConfig, Box<dyn std::error::Error>> {
    Ok(match raw {
        AlertSinkConfigRaw::Webhook {
            policy,
            url,
            retry_max,
            retry_backoff_ms,
        } => AlertSinkConfig::Webhook {
            policy: map_policy(policy)?,
            url,
            retry_max: retry_max.unwrap_or(3).max(1),
            retry_backoff_ms: retry_backoff_ms.unwrap_or(1000).max(1),
        },
        AlertSinkConfigRaw::Slack {
            policy,
            webhook_url,
            channel,
        } => AlertSinkConfig::Slack {
            policy: map_policy(policy)?,
            webhook_url,
            channel,
        },
        AlertSinkConfigRaw::PagerDuty {
            policy,
            routing_key,
        } => AlertSinkConfig::PagerDuty {
            policy: map_policy(policy)?,
            routing_key,
        },
    })
}

fn map_policy(raw: SinkPolicyRaw) -> Result<SinkPolicyConfig, Box<dyn std::error::Error>> {
    Ok(SinkPolicyConfig {
        min_severity: parse_optional_severity(raw.min_severity.as_deref())?,
        dedup_window_secs: raw.dedup_window_secs,
        rate_limit_per_min: raw.rate_limit_per_min,
    })
}

fn parse_optional_severity(
    value: Option<&str>,
) -> Result<Option<Severity>, Box<dyn std::error::Error>> {
    match value {
        Some(raw) => parse_severity(Some(raw))
            .map(Some)
            .ok_or_else(|| format!("invalid alert severity: {raw}").into()),
        None => Ok(None),
    }
}

fn parse_severity(value: Option<&str>) -> Option<Severity> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "critical" => Some(Severity::Critical),
        "warning" | "warn" => Some(Severity::Warning),
        "info" => Some(Severity::Info),
        _ => None,
    }
}

fn default_min_severity() -> Severity {
    Severity::Warning
}

fn default_dedup_window() -> u64 {
    300
}

fn default_rate_limit() -> u32 {
    60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_sane() {
        let c = AlertConfig::default();
        assert_eq!(c.min_severity, Severity::Warning);
        assert_eq!(c.dedup_window_secs, 300);
        assert_eq!(c.rate_limit_per_min, 60);
        assert!(c.sinks.is_empty());
    }

    #[test]
    fn default_path_uses_alerting_toml() {
        assert!(default_config_path().ends_with("helm/alerting.toml"));
    }

    #[test]
    fn load_config_missing_file_returns_default() {
        let tmp = std::env::temp_dir().join("helm_alerting_test_missing.toml");
        let c = load_config_from_path(&tmp).unwrap();
        assert_eq!(c.min_severity, Severity::Warning);
        assert!(c.sinks.is_empty());
    }

    #[test]
    fn load_config_parses_config_and_sinks() {
        let tmp = std::env::temp_dir().join("helm_alerting_test_parse.toml");
        let toml = r#"
[config]
min_severity = "warning"
dedup_window_secs = 120
rate_limit_per_min = 10

[[sink]]
type = "webhook"
url = "https://hooks.internal/helmops"
retry_max = 4
retry_backoff_ms = 1500
min_severity = "critical"

[[sink]]
type = "pagerduty"
routing_key = "pd-key"
"#;
        std::fs::write(&tmp, toml).unwrap();
        let c = load_config_from_path(&tmp).unwrap();
        assert_eq!(c.min_severity, Severity::Warning);
        assert_eq!(c.dedup_window_secs, 120);
        assert_eq!(c.rate_limit_per_min, 10);
        assert_eq!(c.sinks.len(), 2);
        match &c.sinks[0] {
            AlertSinkConfig::Webhook {
                policy,
                retry_max,
                retry_backoff_ms,
                ..
            } => {
                assert_eq!(policy.min_severity, Some(Severity::Critical));
                assert_eq!(*retry_max, 4);
                assert_eq!(*retry_backoff_ms, 1500);
            }
            other => panic!("unexpected sink config: {other:?}"),
        }
    }

    #[test]
    fn load_config_accepts_legacy_alerting_section() {
        let tmp = std::env::temp_dir().join("helm_alerting_test_legacy.toml");
        std::fs::write(
            &tmp,
            "[alerting]\nmin_severity = \"critical\"\ndedup_window_secs = 90\nrate_limit_per_min = 5\n",
        )
        .unwrap();
        let c = load_config_from_path(&tmp).unwrap();
        assert_eq!(c.min_severity, Severity::Critical);
        assert_eq!(c.dedup_window_secs, 90);
        assert_eq!(c.rate_limit_per_min, 5);
    }

    #[test]
    fn load_config_rejects_invalid_sink_severity() {
        let tmp = std::env::temp_dir().join("helm_alerting_test_invalid.toml");
        std::fs::write(
            &tmp,
            "[config]\n\n[[sink]]\ntype = \"slack\"\nwebhook_url = \"https://hooks.slack.test\"\nmin_severity = \"urgent\"\n",
        )
        .unwrap();
        let error = load_config_from_path(&tmp).unwrap_err();
        assert!(error.to_string().contains("invalid alert severity"));
    }
}
