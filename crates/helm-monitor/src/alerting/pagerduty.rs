use serde::Serialize;
use serde_json::{Value, json};

use crate::findings::{FindingLifecycle, Severity};

use super::{AlertPayload, AlertSink, SendFuture};

const PD_ENQUEUE_URL: &str = "https://events.pagerduty.com/v2/enqueue";

pub struct PagerDutySink {
    pub routing_key: String,
    client: reqwest::Client,
    /// Override the enqueue URL (used in tests).
    url: String,
}

impl PagerDutySink {
    pub fn new(routing_key: impl Into<String>) -> Self {
        Self {
            routing_key: routing_key.into(),
            client: reqwest::Client::new(),
            url: PD_ENQUEUE_URL.to_string(),
        }
    }

    #[cfg(test)]
    fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }
}

#[derive(Serialize)]
struct PdEvent {
    routing_key: String,
    event_action: String,
    dedup_key: String,
    payload: PdPayload,
}

#[derive(Serialize)]
struct PdPayload {
    summary: String,
    severity: String,
    source: String,
    custom_details: Value,
}

fn lifecycle_to_action(lc: FindingLifecycle) -> &'static str {
    if lc.is_resolved() {
        "resolve"
    } else {
        "trigger"
    }
}

fn severity_to_pd(sev: Severity) -> &'static str {
    match sev {
        Severity::Critical => "critical",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn build_event(alert: &AlertPayload, routing_key: &str) -> PdEvent {
    PdEvent {
        routing_key: routing_key.to_string(),
        event_action: lifecycle_to_action(alert.lifecycle).to_string(),
        dedup_key: alert.fingerprint.clone(),
        payload: PdPayload {
            summary: format!(
                "{}: {}",
                alert.severity.as_str().to_uppercase(),
                alert.title
            ),
            severity: severity_to_pd(alert.severity).to_string(),
            source: alert.detector_id.clone(),
            custom_details: json!({
                "fingerprint": alert.fingerprint,
                "affected_resource": alert.affected_resource,
                "description": alert.description,
            }),
        },
    }
}

impl AlertSink for PagerDutySink {
    fn send<'a>(&'a self, alert: &'a AlertPayload) -> SendFuture<'a> {
        Box::pin(async move {
            let event = build_event(alert, &self.routing_key);
            let resp = self
                .client
                .post(&self.url)
                .header("Content-Type", "application/json")
                .json(&event)
                .send()
                .await?;
            let status = resp.status();
            if status.as_u16() == 202 || status.is_success() {
                Ok(())
            } else {
                Err(format!("PagerDuty returned {}", status).into())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::{Confidence, Finding, FindingLifecycle, MonitorDomain, Severity};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn make_payload_with_lifecycle(sev: Severity, lc: FindingLifecycle) -> AlertPayload {
        let mut f = Finding::new(
            "snap",
            "det",
            "host/svc",
            "Test alert",
            sev,
            Confidence::High,
            MonitorDomain::Services,
        );
        f.lifecycle = lc;
        AlertPayload::from(&f)
    }

    async fn start_mock(response: &'static [u8]) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = socket.read(&mut buf).await;
                let _ = socket.write_all(response).await;
            }
        });
        addr
    }

    #[test]
    fn trigger_on_open() {
        let p = make_payload_with_lifecycle(Severity::Critical, FindingLifecycle::Open);
        let ev = build_event(&p, "key");
        assert_eq!(ev.event_action, "trigger");
    }

    #[test]
    fn resolve_on_resolved() {
        let p = make_payload_with_lifecycle(Severity::Critical, FindingLifecycle::Resolved);
        let ev = build_event(&p, "key");
        assert_eq!(ev.event_action, "resolve");
    }

    #[test]
    fn resolve_on_self_resolved() {
        let p = make_payload_with_lifecycle(Severity::Critical, FindingLifecycle::SelfResolved);
        let ev = build_event(&p, "key");
        assert_eq!(ev.event_action, "resolve");
    }

    #[test]
    fn dedup_key_is_fingerprint() {
        let p = make_payload_with_lifecycle(Severity::Warning, FindingLifecycle::Open);
        let ev = build_event(&p, "key");
        assert_eq!(ev.dedup_key, p.fingerprint);
    }

    #[test]
    fn severity_mapping() {
        assert_eq!(severity_to_pd(Severity::Critical), "critical");
        assert_eq!(severity_to_pd(Severity::Warning), "warning");
        assert_eq!(severity_to_pd(Severity::Info), "info");
    }

    #[test]
    fn custom_details_populated() {
        let p = make_payload_with_lifecycle(Severity::Critical, FindingLifecycle::Open);
        let ev = build_event(&p, "key");
        assert!(ev.payload.custom_details["fingerprint"].is_string());
        assert!(ev.payload.custom_details["affected_resource"].is_string());
        assert!(ev.payload.custom_details["description"].is_string());
    }

    #[tokio::test]
    async fn send_ok_on_202() {
        let addr = start_mock(
            b"HTTP/1.1 202 Accepted\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
        )
        .await;
        let sink = PagerDutySink::new("test-key").with_url(format!("http://{}", addr));
        let p = make_payload_with_lifecycle(Severity::Critical, FindingLifecycle::Open);
        sink.send(&p).await.unwrap();
    }
}
