pub mod config;
pub mod pagerduty;
pub mod payload;
pub mod slack;
pub mod webhook;

pub use config::{AlertConfig, load_config};
pub use pagerduty::PagerDutySink;
pub use payload::AlertPayload;
pub use slack::SlackSink;
pub use webhook::WebhookSink;

use std::collections::HashMap;
use std::time::Instant;

use crate::findings::Finding;

pub type SendFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Box<dyn std::error::Error>>> + Send + 'a>>;

pub trait AlertSink: Send + Sync {
    fn send<'a>(&'a self, alert: &'a AlertPayload) -> SendFuture<'a>;
}

pub struct AlertRouter {
    sinks: Vec<Box<dyn AlertSink>>,
    config: AlertConfig,
    dedup_state: HashMap<String, Instant>,
    last_window_start: Instant,
    sent_this_window: u32,
}

impl AlertRouter {
    pub fn new(config: AlertConfig) -> Self {
        Self {
            sinks: Vec::new(),
            config,
            dedup_state: HashMap::new(),
            last_window_start: Instant::now(),
            sent_this_window: 0,
        }
    }

    pub fn with_sink(mut self, sink: Box<dyn AlertSink>) -> Self {
        self.sinks.push(sink);
        self
    }

    pub fn add_sink(&mut self, sink: Box<dyn AlertSink>) {
        self.sinks.push(sink);
    }

    pub async fn route(&mut self, finding: &Finding) -> Result<(), Box<dyn std::error::Error>> {
        let payload = AlertPayload::from(finding);

        // severity gate
        if payload.severity < self.config.min_severity {
            return Ok(());
        }

        let now = Instant::now();

        // dedup window
        if let Some(&last) = self.dedup_state.get(&payload.fingerprint) {
            if now.duration_since(last).as_secs() < self.config.dedup_window_secs {
                return Ok(());
            }
        }

        // rate limit — reset window every 60s
        if now.duration_since(self.last_window_start).as_secs() >= 60 {
            self.last_window_start = now;
            self.sent_this_window = 0;
        }
        if self.sent_this_window >= self.config.rate_limit_per_min {
            return Ok(());
        }

        // fan-out — each sink failure is logged but does not block others
        for sink in &self.sinks {
            if let Err(e) = sink.send(&payload).await {
                eprintln!("WARN alerting sink error: {e}");
            }
        }

        self.dedup_state.insert(payload.fingerprint.clone(), now);
        self.sent_this_window += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::{Confidence, Finding, FindingLifecycle, MonitorDomain, Severity};
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
        };
        let mut router = AlertRouter::new(config).with_sink(sink);
        let f = make_finding(Severity::Info, "det-info");
        router.route(&f).await.unwrap();
        assert_eq!(captured.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn dedup_prevents_second_send() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Box::new(CaptureSink(captured.clone()));
        let config = AlertConfig {
            min_severity: Severity::Warning,
            dedup_window_secs: 300,
            rate_limit_per_min: 60,
        };
        let mut router = AlertRouter::new(config).with_sink(sink);
        let f = make_finding(Severity::Critical, "det-dedup");
        router.route(&f).await.unwrap();
        router.route(&f).await.unwrap(); // same fingerprint, should be deduped
        assert_eq!(captured.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rate_limit_blocks_excess_alerts() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Box::new(CaptureSink(captured.clone()));
        let config = AlertConfig {
            min_severity: Severity::Info,
            dedup_window_secs: 0, // no dedup so each different fp passes
            rate_limit_per_min: 2,
        };
        let mut router = AlertRouter::new(config).with_sink(sink);
        for i in 0..5u32 {
            let f = make_finding(Severity::Critical, &format!("det-{i}"));
            router.route(&f).await.unwrap();
        }
        // Only 2 should pass the rate limit
        assert_eq!(captured.lock().unwrap().len(), 2);
    }
}
