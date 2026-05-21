//! Shared thresholds and dashboard tuning configuration.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Clone, PartialEq)]
pub struct DetectorThresholdOverride {
    pub info: Option<f64>,
    pub warning: Option<f64>,
    pub critical: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdsConfig {
    pub refresh_interval_secs: u64,
    pub sparkline_history_depth: usize,
    pub disabled_detectors: HashSet<String>,
    pub detector_thresholds: HashMap<String, DetectorThresholdOverride>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdsLoadOutcome {
    pub config: ThresholdsConfig,
    pub warnings: Vec<String>,
}

impl ThresholdsConfig {
    pub fn detector_override(&self, detector_id: &str) -> Option<&DetectorThresholdOverride> {
        self.detector_thresholds.get(detector_id)
    }

    pub fn detector_enabled(&self, detector_id: &str) -> bool {
        !self.disabled_detectors.contains(detector_id)
    }
}

impl Default for ThresholdsConfig {
    fn default() -> Self {
        Self {
            refresh_interval_secs: 60,
            sparkline_history_depth: 60,
            disabled_detectors: HashSet::new(),
            detector_thresholds: HashMap::new(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ThresholdsConfigRaw {
    refresh_interval_secs: Option<u64>,
    sparkline_history_depth: Option<usize>,
    disabled_detectors: Option<Vec<String>>,
    #[serde(default)]
    detectors: HashMap<String, DetectorThresholdOverrideRaw>,
}

#[derive(Debug, Default, Deserialize)]
struct DetectorThresholdOverrideRaw {
    info: Option<f64>,
    warning: Option<f64>,
    critical: Option<f64>,
}

pub fn default_thresholds_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("helm")
        .join("thresholds.toml")
}

pub fn load_thresholds_config() -> ThresholdsConfig {
    let outcome = load_thresholds_config_from_path(&default_thresholds_path());
    for warning_msg in &outcome.warnings {
        warn!(config = "thresholds.toml", "{warning_msg}");
    }
    outcome.config
}

pub fn load_thresholds_config_from_path(path: &Path) -> ThresholdsLoadOutcome {
    let mut warnings = Vec::new();
    let mut config = ThresholdsConfig::default();

    let Ok(content) = std::fs::read_to_string(path) else {
        return ThresholdsLoadOutcome { config, warnings };
    };
    let raw = match toml::from_str::<ThresholdsConfigRaw>(&content) {
        Ok(raw) => raw,
        Err(error) => {
            warnings.push(format!(
                "failed to parse {}: {error}; using defaults",
                path.display()
            ));
            return ThresholdsLoadOutcome { config, warnings };
        }
    };

    if let Some(refresh) = raw.refresh_interval_secs {
        if (5..=3600).contains(&refresh) {
            config.refresh_interval_secs = refresh;
        } else {
            warnings.push(format!(
                "refresh_interval_secs={} is invalid; using default {}",
                refresh, config.refresh_interval_secs
            ));
        }
    }

    if let Some(depth) = raw.sparkline_history_depth {
        if (10..=300).contains(&depth) {
            config.sparkline_history_depth = depth;
        } else {
            warnings.push(format!(
                "sparkline_history_depth={} is invalid; using default {}",
                depth, config.sparkline_history_depth
            ));
        }
    }

    if let Some(disabled) = raw.disabled_detectors {
        config.disabled_detectors = disabled
            .into_iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect();
    }

    for (detector_id, raw_override) in raw.detectors {
        let override_cfg = DetectorThresholdOverride {
            info: validate_threshold_value(&mut warnings, &detector_id, "info", raw_override.info),
            warning: validate_threshold_value(
                &mut warnings,
                &detector_id,
                "warning",
                raw_override.warning,
            ),
            critical: validate_threshold_value(
                &mut warnings,
                &detector_id,
                "critical",
                raw_override.critical,
            ),
        };
        if override_cfg.info.is_some()
            || override_cfg.warning.is_some()
            || override_cfg.critical.is_some()
        {
            config.detector_thresholds.insert(detector_id, override_cfg);
        }
    }

    ThresholdsLoadOutcome { config, warnings }
}

fn validate_threshold_value(
    warnings: &mut Vec<String>,
    detector_id: &str,
    field: &str,
    value: Option<f64>,
) -> Option<f64> {
    match value {
        Some(value) if value.is_finite() && value >= 0.0 => Some(value),
        Some(value) => {
            warnings.push(format!(
                "detectors.{detector_id}.{field}={value} is invalid; ignoring override"
            ));
            None
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_uses_defaults() {
        let path = std::env::temp_dir().join("helm_thresholds_missing.toml");
        let outcome = load_thresholds_config_from_path(&path);
        assert_eq!(outcome.config, ThresholdsConfig::default());
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let path = std::env::temp_dir().join("helm_thresholds_invalid.toml");
        std::fs::write(
            &path,
            r#"
refresh_interval_secs = 1
sparkline_history_depth = 1000

[detectors.disk-usage]
warning = -1
critical = 92
"#,
        )
        .unwrap();

        let outcome = load_thresholds_config_from_path(&path);
        assert_eq!(outcome.config.refresh_interval_secs, 60);
        assert_eq!(outcome.config.sparkline_history_depth, 60);
        let disk = outcome
            .config
            .detector_thresholds
            .get("disk-usage")
            .unwrap();
        assert_eq!(disk.warning, None);
        assert_eq!(disk.critical, Some(92.0));
        assert_eq!(outcome.warnings.len(), 3);
    }

    #[test]
    fn valid_config_is_loaded() {
        let path = std::env::temp_dir().join("helm_thresholds_valid.toml");
        std::fs::write(
            &path,
            r#"
refresh_interval_secs = 30
sparkline_history_depth = 120
disabled_detectors = ["db-health", "web-server"]

[detectors.disk-usage]
warning = 88
critical = 94
"#,
        )
        .unwrap();

        let outcome = load_thresholds_config_from_path(&path);
        assert_eq!(outcome.config.refresh_interval_secs, 30);
        assert_eq!(outcome.config.sparkline_history_depth, 120);
        assert!(outcome.config.disabled_detectors.contains("db-health"));
        let disk = outcome
            .config
            .detector_thresholds
            .get("disk-usage")
            .unwrap();
        assert_eq!(disk.warning, Some(88.0));
        assert_eq!(disk.critical, Some(94.0));
    }
}
