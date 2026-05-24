use std::{collections::HashSet, sync::OnceLock};

use regex::Regex;

use crate::snapshot::{JournalPattern, LogSeverity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogTimeRange {
    FifteenMinutes,
    OneHour,
    SixHours,
    TwentyFourHours,
    All,
}

impl LogTimeRange {
    pub fn duration_ms(self) -> Option<i64> {
        match self {
            Self::FifteenMinutes => Some(15 * 60 * 1_000),
            Self::OneHour => Some(60 * 60 * 1_000),
            Self::SixHours => Some(6 * 60 * 60 * 1_000),
            Self::TwentyFourHours => Some(24 * 60 * 60 * 1_000),
            Self::All => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPatternFilter<'a> {
    pub unit_query: Option<&'a str>,
    pub severity: Option<LogSeverity>,
    pub range: LogTimeRange,
    pub now_unix_ms: i64,
}

pub fn filter_log_patterns(
    patterns: &[JournalPattern],
    filter: &LogPatternFilter<'_>,
) -> Vec<JournalPattern> {
    let unit_query = filter
        .unit_query
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let cutoff = filter
        .range
        .duration_ms()
        .map(|duration| filter.now_unix_ms.saturating_sub(duration));

    patterns
        .iter()
        .filter(|pattern| {
            let unit_matches = unit_query.is_none_or(|query| {
                pattern
                    .unit
                    .to_ascii_lowercase()
                    .contains(&query.to_ascii_lowercase())
            });
            let severity_matches = filter
                .severity
                .is_none_or(|severity| pattern.severity == severity);
            let time_matches =
                cutoff.is_none_or(|threshold| pattern.last_seen_unix_ms >= threshold);
            unit_matches && severity_matches && time_matches
        })
        .cloned()
        .collect()
}

pub fn log_pattern_ids_new_since_previous(
    current: &[JournalPattern],
    previous: &[JournalPattern],
) -> HashSet<String> {
    let previous_ids = previous
        .iter()
        .map(|pattern| pattern.pattern_id.clone())
        .collect::<HashSet<_>>();

    current
        .iter()
        .filter(|pattern| !previous_ids.contains(&pattern.pattern_id))
        .map(|pattern| pattern.pattern_id.clone())
        .collect()
}

pub fn normalize_log_message(message: &str) -> String {
    static UUID_RE: OnceLock<Regex> = OnceLock::new();
    static IPV4_RE: OnceLock<Regex> = OnceLock::new();
    static HEX_RE: OnceLock<Regex> = OnceLock::new();
    static DIGIT_RE: OnceLock<Regex> = OnceLock::new();
    static WHITESPACE_RE: OnceLock<Regex> = OnceLock::new();

    let uuid_re = UUID_RE.get_or_init(|| {
        Regex::new(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
            .expect("uuid regex")
    });
    let ipv4_re =
        IPV4_RE.get_or_init(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("ipv4 regex"));
    let hex_re = HEX_RE.get_or_init(|| Regex::new(r"\b(?:0x)?[0-9a-f]{6,}\b").expect("hex regex"));
    let digit_re = DIGIT_RE.get_or_init(|| Regex::new(r"\b\d+\b").expect("digit regex"));
    let whitespace_re = WHITESPACE_RE.get_or_init(|| Regex::new(r"\s+").expect("whitespace regex"));

    let normalized = message.to_ascii_lowercase();
    let normalized = uuid_re.replace_all(&normalized, "<id>");
    let normalized = ipv4_re.replace_all(&normalized, "<ip>");
    let normalized = hex_re.replace_all(&normalized, "<hex>");
    let normalized = digit_re.replace_all(&normalized, "<n>");
    whitespace_re
        .replace_all(&normalized, " ")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_pattern(
        id: &str,
        unit: &str,
        severity: LogSeverity,
        last_seen_unix_ms: i64,
    ) -> JournalPattern {
        JournalPattern {
            pattern_id: id.to_owned(),
            unit: unit.to_owned(),
            severity,
            normalized_message: format!("{unit}:{id}"),
            sample_message: format!("{unit} sample"),
            occurrence_count: 1,
            first_seen_unix_ms: last_seen_unix_ms.saturating_sub(1_000),
            last_seen_unix_ms,
            context_lines: Vec::new(),
        }
    }

    proptest! {
        #[test]
        fn property_31_log_filter_correctness(
            units in prop::collection::vec("[a-z]{3,8}", 1..20),
            severity_pick in prop_oneof![
                Just(None),
                Just(Some(LogSeverity::Warning)),
                Just(Some(LogSeverity::Error)),
                Just(Some(LogSeverity::Critical)),
            ],
            range_pick in prop_oneof![
                Just(LogTimeRange::FifteenMinutes),
                Just(LogTimeRange::OneHour),
                Just(LogTimeRange::SixHours),
                Just(LogTimeRange::TwentyFourHours),
                Just(LogTimeRange::All),
            ],
            query_fragment in prop_oneof![
                Just(None),
                "[a-z]{1,4}".prop_map(Some),
            ],
            offsets in prop::collection::vec(0i64..(30 * 60 * 60 * 1_000), 1..20),
        ) {
            let now_unix_ms = 2_000_000_000_000i64;
            let patterns = units.iter().enumerate().map(|(idx, unit)| {
                let severity = match idx % 3 {
                    0 => LogSeverity::Warning,
                    1 => LogSeverity::Error,
                    _ => LogSeverity::Critical,
                };
                let offset = offsets[idx % offsets.len()];
                sample_pattern(
                    &format!("pattern-{idx}"),
                    unit,
                    severity,
                    now_unix_ms.saturating_sub(offset),
                )
            }).collect::<Vec<_>>();

            let filter = LogPatternFilter {
                unit_query: query_fragment.as_deref(),
                severity: severity_pick,
                range: range_pick,
                now_unix_ms,
            };

            let expected = patterns
                .iter()
                .filter(|pattern| {
                    let unit_matches = filter.unit_query.is_none_or(|query| {
                        pattern.unit.to_ascii_lowercase().contains(&query.to_ascii_lowercase())
                    });
                    let severity_matches = filter.severity.is_none_or(|severity| pattern.severity == severity);
                    let time_matches = filter.range.duration_ms().is_none_or(|duration| {
                        pattern.last_seen_unix_ms >= now_unix_ms.saturating_sub(duration)
                    });
                    unit_matches && severity_matches && time_matches
                })
                .map(|pattern| pattern.pattern_id.clone())
                .collect::<Vec<_>>();

            let actual = filter_log_patterns(&patterns, &filter)
                .into_iter()
                .map(|pattern| pattern.pattern_id)
                .collect::<Vec<_>>();

            prop_assert_eq!(actual, expected);
        }

        #[test]
        fn property_36_new_log_pattern_highlighting(
            previous_ids in prop::collection::hash_set("pattern-[0-9]{1,3}", 0..20),
            current_ids in prop::collection::hash_set("pattern-[0-9]{1,3}", 0..20),
        ) {
            let previous = previous_ids.iter().map(|id| {
                sample_pattern(id, "sshd", LogSeverity::Error, 100)
            }).collect::<Vec<_>>();
            let current = current_ids.iter().map(|id| {
                sample_pattern(id, "sshd", LogSeverity::Error, 200)
            }).collect::<Vec<_>>();

            let expected = current_ids
                .difference(&previous_ids)
                .cloned()
                .collect::<HashSet<_>>();
            let actual = log_pattern_ids_new_since_previous(&current, &previous);

            prop_assert_eq!(actual, expected);
        }
    }
}
