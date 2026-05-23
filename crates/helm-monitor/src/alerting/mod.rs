pub mod config;
pub mod pagerduty;
pub mod payload;
pub mod slack;
pub mod webhook;

pub use config::{
    AlertConfig, AlertSinkConfig, ResolvedSinkPolicy, SinkKind, SinkPolicyConfig,
    default_config_path, load_config, load_config_from_path,
};
pub use pagerduty::PagerDutySink;
pub use payload::AlertPayload;
pub use slack::SlackSink;
pub use webhook::WebhookSink;

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::findings::Finding;

pub type SendFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<(), Box<dyn std::error::Error>>> + Send + 'a>,
>;
type AuditCallback = dyn Fn(&str, &str, &str) + Send + Sync;

pub trait AlertSink: Send + Sync {
    fn send<'a>(&'a self, alert: &'a AlertPayload) -> SendFuture<'a>;
}

struct ConfiguredSink {
    name: String,
    kind: SinkKind,
    policy: ResolvedSinkPolicy,
    sink: Box<dyn AlertSink>,
    dedup_state: HashMap<String, Instant>,
    sent_timestamps: VecDeque<Instant>,
}

impl ConfiguredSink {
    fn new(
        name: impl Into<String>,
        kind: SinkKind,
        policy: ResolvedSinkPolicy,
        sink: Box<dyn AlertSink>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            policy,
            sink,
            dedup_state: HashMap::new(),
            sent_timestamps: VecDeque::new(),
        }
    }

    fn should_bypass_guards(&self, payload: &AlertPayload) -> bool {
        payload.lifecycle.is_resolved() && self.kind == SinkKind::PagerDuty
    }

    fn allows_severity(&self, payload: &AlertPayload) -> bool {
        self.should_bypass_guards(payload) || payload.severity >= self.policy.min_severity
    }

    fn is_deduped(&self, payload: &AlertPayload, now: Instant) -> bool {
        if self.should_bypass_guards(payload) {
            return false;
        }

        match self.dedup_state.get(&payload.fingerprint) {
            Some(last) => now.duration_since(*last).as_secs() < self.policy.dedup_window_secs,
            None => false,
        }
    }

    fn is_rate_limited(&mut self, payload: &AlertPayload, now: Instant) -> bool {
        if self.should_bypass_guards(payload) {
            return false;
        }

        while let Some(ts) = self.sent_timestamps.front().copied() {
            if now.duration_since(ts).as_secs() < 60 {
                break;
            }
            self.sent_timestamps.pop_front();
        }

        self.sent_timestamps.len() >= self.policy.rate_limit_per_min as usize
    }

    fn record_delivery(&mut self, payload: &AlertPayload, now: Instant) {
        if payload.lifecycle.is_resolved() {
            self.dedup_state.remove(&payload.fingerprint);
            return;
        }

        self.dedup_state.insert(payload.fingerprint.clone(), now);
        self.sent_timestamps.push_back(now);
    }
}

pub struct AlertRouter {
    config: AlertConfig,
    sinks: Vec<ConfiguredSink>,
    /// Optional audit callback: (action, fingerprint, reason)
    audit_callback: Option<Box<AuditCallback>>,
}

/// Notification decision for audit trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationDecision {
    Sent,
    SkippedSeverity,
    SkippedDedup,
    SkippedRateLimit,
    Resolved,
    Suppressed,
}

impl NotificationDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::SkippedSeverity => "skipped-severity",
            Self::SkippedDedup => "skipped-dedup",
            Self::SkippedRateLimit => "skipped-ratelimit",
            Self::Resolved => "resolved",
            Self::Suppressed => "suppressed",
        }
    }
}

impl AlertRouter {
    pub fn new(config: AlertConfig) -> Self {
        let mut router = Self {
            sinks: Vec::new(),
            config,
            audit_callback: None,
        };
        let configured_sinks = router.config.sinks.clone();
        for sink in configured_sinks {
            router.add_sink_from_config(sink);
        }
        router
    }

    pub fn with_sink(mut self, sink: Box<dyn AlertSink>) -> Self {
        self.add_sink(sink);
        self
    }

    pub fn with_sink_policy(
        mut self,
        sink: Box<dyn AlertSink>,
        kind: SinkKind,
        name: impl Into<String>,
        policy: ResolvedSinkPolicy,
    ) -> Self {
        self.add_sink_with_policy(sink, kind, name, policy);
        self
    }

    pub fn with_audit<F>(mut self, callback: F) -> Self
    where
        F: Fn(&str, &str, &str) + Send + Sync + 'static,
    {
        self.audit_callback = Some(Box::new(callback));
        self
    }

    pub fn add_sink(&mut self, sink: Box<dyn AlertSink>) {
        self.add_sink_with_policy(
            sink,
            SinkKind::Webhook,
            "custom-sink",
            ResolvedSinkPolicy {
                min_severity: self.config.min_severity,
                dedup_window_secs: self.config.dedup_window_secs,
                rate_limit_per_min: self.config.rate_limit_per_min,
            },
        );
    }

    pub fn add_sink_with_policy(
        &mut self,
        sink: Box<dyn AlertSink>,
        kind: SinkKind,
        name: impl Into<String>,
        policy: ResolvedSinkPolicy,
    ) {
        self.sinks
            .push(ConfiguredSink::new(name, kind, policy, sink));
    }

    pub fn add_sink_from_config(&mut self, sink_config: AlertSinkConfig) {
        let policy = sink_config.policy().resolve(&self.config);
        let name = sink_config.name();
        match sink_config {
            AlertSinkConfig::Webhook {
                url,
                retry_max,
                retry_backoff_ms,
                ..
            } => self.add_sink_with_policy(
                Box::new(WebhookSink::new(url).with_retry_policy(retry_max, retry_backoff_ms)),
                SinkKind::Webhook,
                name,
                policy,
            ),
            AlertSinkConfig::Slack {
                webhook_url,
                channel,
                ..
            } => {
                let sink = match channel {
                    Some(channel) => SlackSink::new(webhook_url).with_channel(channel),
                    None => SlackSink::new(webhook_url),
                };
                self.add_sink_with_policy(Box::new(sink), SinkKind::Slack, name, policy);
            }
            AlertSinkConfig::PagerDuty { routing_key, .. } => self.add_sink_with_policy(
                Box::new(PagerDutySink::new(routing_key)),
                SinkKind::PagerDuty,
                name,
                policy,
            ),
        }
    }

    pub async fn route(&mut self, finding: &Finding) -> Result<(), Box<dyn std::error::Error>> {
        let payload = AlertPayload::from(finding);
        let audit = self.audit_callback.as_deref();

        for sink in &mut self.sinks {
            if !sink.allows_severity(&payload) {
                audit_sink(
                    audit,
                    NotificationDecision::SkippedSeverity,
                    &payload.fingerprint,
                    &sink.name,
                    "severity below threshold",
                );
                continue;
            }

            let now = Instant::now();

            if sink.is_deduped(&payload, now) {
                audit_sink(
                    audit,
                    NotificationDecision::SkippedDedup,
                    &payload.fingerprint,
                    &sink.name,
                    "within dedup window",
                );
                continue;
            }

            if sink.is_rate_limited(&payload, now) {
                audit_sink(
                    audit,
                    NotificationDecision::SkippedRateLimit,
                    &payload.fingerprint,
                    &sink.name,
                    "rate limit exceeded",
                );
                continue;
            }

            if let Err(error) = sink.sink.send(&payload).await {
                eprintln!("WARN alerting sink {} error: {error}", sink.name);
                continue;
            }

            sink.record_delivery(&payload, now);
            let decision = if payload.lifecycle.is_resolved() && sink.kind == SinkKind::PagerDuty {
                NotificationDecision::Resolved
            } else {
                NotificationDecision::Sent
            };
            audit_sink(
                audit,
                decision,
                &payload.fingerprint,
                &sink.name,
                "notification delivered",
            );
        }

        Ok(())
    }

    /// Record a lifecycle-driven notification update (resolve/suppress).
    pub fn notify_lifecycle(&mut self, fingerprint: &str, decision: NotificationDecision) {
        for sink in &mut self.sinks {
            sink.dedup_state.remove(fingerprint);
        }
        audit_sink(
            self.audit_callback.as_deref(),
            decision,
            fingerprint,
            "all-sinks",
            "lifecycle update",
        );
    }
}

fn audit_sink(
    callback: Option<&AuditCallback>,
    decision: NotificationDecision,
    fingerprint: &str,
    sink_name: &str,
    reason: &str,
) {
    if let Some(cb) = callback {
        let reason = format!("sink={sink_name} {reason}");
        cb(decision.as_str(), fingerprint, &reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::{Confidence, Finding, FindingLifecycle, MonitorDomain, Severity};
    use proptest::prelude::*;
    use std::sync::{Arc, Mutex};

    fn make_finding(sev: Severity, fingerprint_key: &str) -> Finding {
        let mut f = Finding::new(
            "snap",
            fingerprint_key,
            "resource",
            "title",
            sev,
            Confidence::High,
            MonitorDomain::Services,
        );
        f.lifecycle = FindingLifecycle::Open;
        f
    }

    fn severity_from_index(index: u8) -> Severity {
        match index % 3 {
            0 => Severity::Info,
            1 => Severity::Warning,
            _ => Severity::Critical,
        }
    }

    struct CaptureSink(Arc<Mutex<Vec<String>>>);

    impl AlertSink for CaptureSink {
        fn send<'a>(&'a self, alert: &'a AlertPayload) -> crate::alerting::SendFuture<'a> {
            let captured = self.0.clone();
            let fp = alert.fingerprint.clone();
            Box::pin(async move {
                captured.lock().unwrap().push(fp);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn routes_critical_finding() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Box::new(CaptureSink(captured.clone()));
        let config = AlertConfig {
            min_severity: Severity::Warning,
            dedup_window_secs: 300,
            rate_limit_per_min: 60,
            sinks: Vec::new(),
        };
        let mut router = AlertRouter::new(config).with_sink(sink);
        let f = make_finding(Severity::Critical, "det-crit");
        router.route(&f).await.unwrap();
        assert_eq!(captured.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn skips_info_below_warning_threshold() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Box::new(CaptureSink(captured.clone()));
        let config = AlertConfig {
            min_severity: Severity::Warning,
            dedup_window_secs: 300,
            rate_limit_per_min: 60,
            sinks: Vec::new(),
        };
        let mut router = AlertRouter::new(config).with_sink(sink);
        let f = make_finding(Severity::Info, "det-info");
        router.route(&f).await.unwrap();
        assert_eq!(captured.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn dedup_prevents_second_send_per_sink() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Box::new(CaptureSink(captured.clone()));
        let config = AlertConfig {
            min_severity: Severity::Warning,
            dedup_window_secs: 300,
            rate_limit_per_min: 60,
            sinks: Vec::new(),
        };
        let mut router = AlertRouter::new(config).with_sink(sink);
        let f = make_finding(Severity::Critical, "det-dedup");
        router.route(&f).await.unwrap();
        router.route(&f).await.unwrap();
        assert_eq!(captured.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rate_limit_blocks_excess_alerts_per_sink() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Box::new(CaptureSink(captured.clone()));
        let config = AlertConfig {
            min_severity: Severity::Info,
            dedup_window_secs: 0,
            rate_limit_per_min: 2,
            sinks: Vec::new(),
        };
        let mut router = AlertRouter::new(config).with_sink(sink);
        for i in 0..5u32 {
            let f = make_finding(Severity::Critical, &format!("det-{i}"));
            router.route(&f).await.unwrap();
        }
        assert_eq!(captured.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn sink_policies_are_independent() {
        let warning_captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let critical_captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config = AlertConfig {
            min_severity: Severity::Info,
            dedup_window_secs: 300,
            rate_limit_per_min: 60,
            sinks: Vec::new(),
        };
        let mut router = AlertRouter::new(config)
            .with_sink_policy(
                Box::new(CaptureSink(warning_captured.clone())),
                SinkKind::Slack,
                "warning-sink",
                ResolvedSinkPolicy {
                    min_severity: Severity::Warning,
                    dedup_window_secs: 300,
                    rate_limit_per_min: 60,
                },
            )
            .with_sink_policy(
                Box::new(CaptureSink(critical_captured.clone())),
                SinkKind::PagerDuty,
                "critical-sink",
                ResolvedSinkPolicy {
                    min_severity: Severity::Critical,
                    dedup_window_secs: 300,
                    rate_limit_per_min: 60,
                },
            );

        let finding = make_finding(Severity::Warning, "warning-finding");
        router.route(&finding).await.unwrap();

        assert_eq!(warning_captured.lock().unwrap().len(), 1);
        assert_eq!(critical_captured.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn pagerduty_resolution_bypasses_threshold_and_limits() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config = AlertConfig {
            min_severity: Severity::Critical,
            dedup_window_secs: 300,
            rate_limit_per_min: 0,
            sinks: Vec::new(),
        };
        let mut router = AlertRouter::new(config).with_sink_policy(
            Box::new(CaptureSink(captured.clone())),
            SinkKind::PagerDuty,
            "pagerduty",
            ResolvedSinkPolicy {
                min_severity: Severity::Critical,
                dedup_window_secs: 300,
                rate_limit_per_min: 0,
            },
        );
        let mut finding = make_finding(Severity::Info, "resolved-finding");
        finding.lifecycle = FindingLifecycle::SelfResolved;

        router.route(&finding).await.unwrap();

        assert_eq!(captured.lock().unwrap().len(), 1);
    }

    proptest! {
        #[test]
        fn property_alert_severity_routing(
            finding_severity in 0u8..=2,
            sink_thresholds in prop::collection::vec(0u8..=2, 1..6),
        ) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let finding_severity = severity_from_index(finding_severity);
                let config = AlertConfig::default();
                let mut router = AlertRouter::new(config);
                let captures: Vec<_> = sink_thresholds.iter().map(|_| Arc::new(Mutex::new(Vec::new()))).collect();

                for (index, threshold_index) in sink_thresholds.iter().enumerate() {
                    router.add_sink_with_policy(
                        Box::new(CaptureSink(captures[index].clone())),
                        SinkKind::Slack,
                        format!("sink-{index}"),
                        ResolvedSinkPolicy {
                            min_severity: severity_from_index(*threshold_index),
                            dedup_window_secs: 0,
                            rate_limit_per_min: 60,
                        },
                    );
                }

                let finding = make_finding(finding_severity, "property-severity");
                router.route(&finding).await.unwrap();

                for (capture, threshold_index) in captures.iter().zip(sink_thresholds.iter()) {
                    let expected = usize::from(finding_severity >= severity_from_index(*threshold_index));
                    assert_eq!(capture.lock().unwrap().len(), expected);
                }
            });
        }

        #[test]
        fn property_alert_deduplication(window_secs in 1u64..600, attempts in 1usize..20) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
                let config = AlertConfig {
                    min_severity: Severity::Info,
                    dedup_window_secs: window_secs,
                    rate_limit_per_min: 60,
                    sinks: Vec::new(),
                };
                let mut router = AlertRouter::new(config).with_sink(Box::new(CaptureSink(captured.clone())));
                let finding = make_finding(Severity::Critical, "property-dedup");

                for _ in 0..attempts {
                    router.route(&finding).await.unwrap();
                }

                assert_eq!(captured.lock().unwrap().len(), 1);
            });
        }

        #[test]
        fn property_alert_rate_limiting(limit in 1u32..20, burst in 1usize..40) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
                let config = AlertConfig {
                    min_severity: Severity::Info,
                    dedup_window_secs: 0,
                    rate_limit_per_min: limit,
                    sinks: Vec::new(),
                };
                let mut router = AlertRouter::new(config).with_sink(Box::new(CaptureSink(captured.clone())));

                for index in 0..burst {
                    let finding = make_finding(Severity::Critical, &format!("property-rate-{index}"));
                    router.route(&finding).await.unwrap();
                }

                assert_eq!(
                    captured.lock().unwrap().len(),
                    usize::min(burst, limit as usize)
                );
            });
        }
    }
}
