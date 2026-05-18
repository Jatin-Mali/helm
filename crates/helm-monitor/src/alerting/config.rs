use serde::Deserialize;

use crate::findings::Severity;

#[derive(Debug, Clone, Deserialize)]
pub struct AlertConfig {
    #[serde(default = "default_min_severity")]
    pub min_severity: Severity,
    #[serde(default = "default_dedup_window")]
    pub dedup_window_secs: u64,
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_min: u32,
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

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            min_severity: default_min_severity(),
            dedup_window_secs: default_dedup_window(),
            rate_limit_per_min: default_rate_limit(),
        }
    }
}

#[derive(Deserialize, Default)]
struct TomlRoot {
    #[serde(default)]
    alerting: Option<AlertConfigRaw>,
}

#[derive(Deserialize)]
struct AlertConfigRaw {
    min_severity: Option<String>,
    dedup_window_secs: Option<u64>,
    rate_limit_per_min: Option<u32>,
}

pub fn load_config() -> Result<AlertConfig, Box<dyn std::error::Error>> {
    let path = dirs_config_path();
    if !path.exists() {
        return Ok(AlertConfig::default());
    }
    let content = std::fs::read_to_string(&path)?;
    let root: TomlRoot = toml::from_str(&content)?;
    let Some(raw) = root.alerting else {
        return Ok(AlertConfig::default());
    };

    let min_severity = match raw.min_severity.as_deref() {
        Some("critical") => Severity::Critical,
        Some("info") => Severity::Info,
        _ => Severity::Warning,
    };

    Ok(AlertConfig {
        min_severity,
        dedup_window_secs: raw.dedup_window_secs.unwrap_or(300),
        rate_limit_per_min: raw.rate_limit_per_min.unwrap_or(60),
    })
}

fn dirs_config_path() -> std::path::PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            std::path::PathBuf::from(home).join(".config")
        });
    base.join("helm").join("thresholds.toml")
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
    }

    #[test]
    fn load_config_missing_file_returns_default() {
        let tmp = std::env::temp_dir().join("helm_alerting_test_no_file");
        let _ = std::fs::create_dir_all(&tmp);
        // SAFETY: single-threaded test binary; no concurrent env reads
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &tmp) };
        let c = load_config().unwrap();
        assert_eq!(c.min_severity, Severity::Warning);
    }

    #[test]
    fn load_config_parses_alerting_section() {
        let tmp = std::env::temp_dir().join("helm_alerting_test_parse");
        let helm_dir = tmp.join("helm");
        let _ = std::fs::create_dir_all(&helm_dir);
        let toml = "[alerting]\nmin_severity = \"critical\"\ndedup_window_secs = 120\nrate_limit_per_min = 10\n";
        std::fs::write(helm_dir.join("thresholds.toml"), toml).unwrap();
        // SAFETY: single-threaded test binary; no concurrent env reads
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &tmp) };
        let c = load_config().unwrap();
        assert_eq!(c.min_severity, Severity::Critical);
        assert_eq!(c.dedup_window_secs, 120);
        assert_eq!(c.rate_limit_per_min, 10);
    }
}
