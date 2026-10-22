//! Logs collector: journalctl bounded windows with grouped anomaly patterns.

use std::collections::HashMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    collectors::{Collector, err, run_timed},
    log_analysis::normalize_log_message,
    snapshot::{JournalContextLine, JournalPattern, LogSeverity, LogSnapshot, MonitorProfile},
};

#[derive(Default)]
pub struct LogsCollector;

#[derive(Debug, Clone)]
struct ParsedJournalEntry {
    timestamp: String,
    timestamp_unix_ms: i64,
    unit: String,
    severity: LogSeverity,
    message: String,
    raw: String,
}

#[derive(Debug, Clone)]
struct PatternAggregate {
    pattern_id: String,
    unit: String,
    severity: LogSeverity,
    normalized_message: String,
    sample_message: String,
    occurrence_count: u64,
    first_seen_unix_ms: i64,
    last_seen_unix_ms: i64,
    latest_entry_index: usize,
}

impl Collector for LogsCollector {
    type Output = LogSnapshot;

    fn domain(&self) -> &'static str {
        "logs"
    }

    async fn collect(
        self,
        profile: MonitorProfile,
    ) -> Result<Self::Output, crate::snapshot::CollectorError> {
        let mut out = LogSnapshot::default();
        let output = run_timed(
            "journalctl",
            &[
                "-p",
                "warning..alert",
                "--since",
                "24 hours ago",
                "--no-pager",
                "-n",
                "400",
                "-o",
                "json",
            ],
            profile,
        )
        .await?;
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        if diagnostics.contains("not seeing messages")
            || diagnostics.contains("Permission denied")
            || diagnostics.contains("No journal files")
        {
            return Err(err(
                "logs",
                helm_core::redact_secrets(&format!(
                    "journal coverage unavailable or incomplete: {diagnostics}"
                )),
            ));
        }

        let mut entries = parse_journal_entries(&output.stdout);
        if entries.is_empty() {
            return Ok(out);
        }
        entries.sort_by_key(|entry| entry.timestamp_unix_ms);

        let now_unix_ms = chrono::Utc::now().timestamp_millis();
        out.journal_errors_last_hour = entries
            .iter()
            .filter(|entry| {
                matches!(entry.severity, LogSeverity::Error | LogSeverity::Critical)
                    && entry.timestamp_unix_ms >= now_unix_ms.saturating_sub(60 * 60 * 1_000)
            })
            .count() as u64;

        out.kernel_errors = entries
            .iter()
            .filter(|entry| {
                matches!(entry.severity, LogSeverity::Error | LogSeverity::Critical)
                    && looks_like_kernel_entry(entry)
            })
            .rev()
            .take(20)
            .map(|entry| entry.raw.clone())
            .collect::<Vec<_>>();
        out.kernel_errors.reverse();

        out.auth_failures = entries
            .iter()
            .filter(|entry| looks_like_auth_failure(entry))
            .rev()
            .take(10)
            .map(|entry| entry.raw.clone())
            .collect::<Vec<_>>();
        out.auth_failures.reverse();

        if matches!(profile, MonitorProfile::Standard | MonitorProfile::Deep) {
            let recent_errors = entries
                .iter()
                .filter(|entry| {
                    matches!(entry.severity, LogSeverity::Error | LogSeverity::Critical)
                        && entry.timestamp_unix_ms >= now_unix_ms.saturating_sub(5 * 60 * 1_000)
                })
                .count() as f64;
            out.error_rate_per_minute = Some(recent_errors / 5.0);
        }

        out.patterns = build_patterns(&entries);
        Ok(out)
    }
}

fn parse_journal_entries(stdout: &[u8]) -> Vec<ParsedJournalEntry> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| parse_journal_line(line).ok())
        .collect()
}

fn parse_journal_line(line: &str) -> Result<ParsedJournalEntry, ()> {
    let value = serde_json::from_str::<Value>(line).map_err(|_| ())?;
    let message = string_field(&value, "MESSAGE").unwrap_or_default();
    if message.trim().is_empty() {
        return Err(());
    }

    let unit = string_field(&value, "_SYSTEMD_UNIT")
        .or_else(|| string_field(&value, "UNIT"))
        .or_else(|| string_field(&value, "SYSLOG_IDENTIFIER"))
        .or_else(|| string_field(&value, "_COMM"))
        .or_else(|| string_field(&value, "_EXE").and_then(executable_name))
        .unwrap_or_else(|| "journal".to_owned());
    let priority = string_field(&value, "PRIORITY")
        .and_then(|raw| raw.parse::<u8>().ok())
        .unwrap_or(4);
    let severity = LogSeverity::from_priority(priority);
    let timestamp_unix_ms = string_field(&value, "__REALTIME_TIMESTAMP")
        .and_then(|raw| raw.parse::<i64>().ok())
        .map(|micros| micros / 1_000)
        .unwrap_or_default();
    let timestamp = chrono::DateTime::from_timestamp_millis(timestamp_unix_ms)
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".to_owned());
    let raw = format!(
        "{timestamp} {} {:<8} {}",
        severity_label(severity),
        unit,
        message
    );

    Ok(ParsedJournalEntry {
        timestamp,
        timestamp_unix_ms,
        unit,
        severity,
        message,
        raw,
    })
}

fn executable_name(path: String) -> Option<String> {
    let name = path.rsplit('/').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(text)) => Some(text.trim().to_owned()),
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::Array(items)) => items
            .first()
            .and_then(|item| item.as_str())
            .map(|text| text.trim().to_owned()),
        _ => None,
    }
}

fn build_patterns(entries: &[ParsedJournalEntry]) -> Vec<JournalPattern> {
    let mut patterns: HashMap<String, PatternAggregate> = HashMap::new();

    for (idx, entry) in entries.iter().enumerate() {
        let normalized_message = normalize_log_message(&entry.message);
        let pattern_id = pattern_id_for(&entry.unit, entry.severity, &normalized_message);
        let aggregate = patterns
            .entry(pattern_id.clone())
            .or_insert_with(|| PatternAggregate {
                pattern_id: pattern_id.clone(),
                unit: entry.unit.clone(),
                severity: entry.severity,
                normalized_message: normalized_message.clone(),
                sample_message: entry.message.clone(),
                occurrence_count: 0,
                first_seen_unix_ms: entry.timestamp_unix_ms,
                last_seen_unix_ms: entry.timestamp_unix_ms,
                latest_entry_index: idx,
            });
        aggregate.occurrence_count += 1;
        aggregate.first_seen_unix_ms = aggregate.first_seen_unix_ms.min(entry.timestamp_unix_ms);
        if entry.timestamp_unix_ms >= aggregate.last_seen_unix_ms {
            aggregate.last_seen_unix_ms = entry.timestamp_unix_ms;
            aggregate.sample_message = entry.message.clone();
            aggregate.latest_entry_index = idx;
        }
    }

    let mut patterns = patterns
        .into_values()
        .map(|aggregate| JournalPattern {
            pattern_id: aggregate.pattern_id,
            unit: aggregate.unit,
            severity: aggregate.severity,
            normalized_message: aggregate.normalized_message,
            sample_message: aggregate.sample_message,
            occurrence_count: aggregate.occurrence_count,
            first_seen_unix_ms: aggregate.first_seen_unix_ms,
            last_seen_unix_ms: aggregate.last_seen_unix_ms,
            context_lines: context_lines_for(entries, aggregate.latest_entry_index),
        })
        .collect::<Vec<_>>();

    patterns.sort_by(|left, right| {
        severity_sort_rank(left.severity)
            .cmp(&severity_sort_rank(right.severity))
            .then(right.occurrence_count.cmp(&left.occurrence_count))
            .then(right.last_seen_unix_ms.cmp(&left.last_seen_unix_ms))
            .then(left.unit.cmp(&right.unit))
    });
    patterns
}

fn context_lines_for(
    entries: &[ParsedJournalEntry],
    latest_index: usize,
) -> Vec<JournalContextLine> {
    let start = latest_index.saturating_sub(1);
    let end = (latest_index + 1).min(entries.len().saturating_sub(1));
    let mut context = Vec::new();
    for (idx, entry) in entries.iter().enumerate().take(end + 1).skip(start) {
        context.push(JournalContextLine {
            offset: (idx as isize - latest_index as isize) as i8,
            timestamp: entry.timestamp.clone(),
            unit: entry.unit.clone(),
            severity: entry.severity,
            message: entry.message.clone(),
            raw: entry.raw.clone(),
        });
    }
    context
}

fn pattern_id_for(unit: &str, severity: LogSeverity, normalized_message: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(unit.as_bytes());
    hasher.update([0]);
    hasher.update(severity.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(normalized_message.as_bytes());
    hex::encode(hasher.finalize())
}

fn looks_like_kernel_entry(entry: &ParsedJournalEntry) -> bool {
    let unit = entry.unit.to_ascii_lowercase();
    let message = entry.message.to_ascii_lowercase();
    unit.contains("kernel")
        || unit.contains("kmsg")
        || message.contains("kernel:")
        || message.contains("out of memory")
        || message.contains("segfault")
}

fn looks_like_auth_failure(entry: &ParsedJournalEntry) -> bool {
    let unit = entry.unit.to_ascii_lowercase();
    let message = entry.message.to_ascii_lowercase();
    (unit.contains("ssh") || unit.contains("auth") || unit.contains("sudo"))
        && (message.contains("failed password")
            || message.contains("authentication failure")
            || message.contains("invalid user")
            || message.contains("pam_unix"))
}

fn severity_label(severity: LogSeverity) -> &'static str {
    match severity {
        LogSeverity::Warning => "WARN",
        LogSeverity::Error => "ERR",
        LogSeverity::Critical => "CRIT",
    }
}

fn severity_sort_rank(severity: LogSeverity) -> u8 {
    match severity {
        LogSeverity::Critical => 0,
        LogSeverity::Error => 1,
        LogSeverity::Warning => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_context_crosses_signed_byte_boundary() {
        let entries = (0..260)
            .map(|index| {
                parse_journal_line(
                    &serde_json::json!({
                        "MESSAGE": format!("entry {index}"), "_SYSTEMD_UNIT": "fixture.service",
                        "PRIORITY": "3", "__REALTIME_TIMESTAMP": "1000000",
                    })
                    .to_string(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        for index in [127, 128, 255, 256] {
            let context = context_lines_for(&entries, index);
            assert_eq!(
                context.iter().map(|line| line.offset).collect::<Vec<_>>(),
                [-1, 0, 1]
            );
            assert_eq!(context[1].message, format!("entry {index}"));
        }
    }
}
