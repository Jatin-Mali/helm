use serde_json::{Value, json};

use crate::findings::Severity;

use super::{AlertPayload, AlertSink, SendFuture};

pub struct SlackSink {
    pub webhook_url: String,
    pub channel: Option<String>,
    client: reqwest::Client,
}

impl SlackSink {
    pub fn new(webhook_url: impl Into<String>) -> Self {
        Self {
            webhook_url: webhook_url.into(),
            channel: None,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }
}

fn severity_to_color(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "#d32f2f",
        Severity::Warning => "#f9a825",
        Severity::Info => "#1565c0",
    }
}

fn build_message(alert: &AlertPayload, channel: Option<&str>) -> Value {
    let timestamp = format!("{:?}", alert.timestamp);
    let mut msg = json!({
        "attachments": [{
            "fallback": format!("{}: {}", alert.severity.as_str().to_uppercase(), alert.title),
            "color": severity_to_color(alert.severity),
            "title": alert.title,
            "fields": [
                { "title": "Severity",    "value": alert.severity.as_str(), "short": true },
                { "title": "Detector",    "value": alert.detector_id,       "short": true },
                { "title": "Resource",    "value": alert.affected_resource, "short": false },
                { "title": "Fingerprint", "value": alert.fingerprint,       "short": false },
                { "title": "Category",    "value": alert.category.as_str(), "short": true },
                { "title": "Timestamp",   "value": timestamp,               "short": true },
            ]
        }]
    });
    if let Some(ch) = channel {
        msg["channel"] = json!(ch);
    }
    msg
}

impl AlertSink for SlackSink {
    fn send<'a>(&'a self, alert: &'a AlertPayload) -> SendFuture<'a> {
        Box::pin(async move {
            let body = build_message(alert, self.channel.as_deref());
            let resp = self
                .client
                .post(&self.webhook_url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;
            let status = resp.status();
            if status.is_success() {
                Ok(())
            } else {
                Err(format!("Slack webhook returned {}", status).into())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::{Confidence, Finding, MonitorDomain, Severity};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn make_payload(sev: Severity) -> AlertPayload {
        let f = Finding::new(
            "snap",
            "det",
            "host/svc",
            "Test alert",
            sev,
            Confidence::High,
            MonitorDomain::Services,
        );
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
    fn critical_color() {
        assert_eq!(severity_to_color(Severity::Critical), "#d32f2f");
    }

    #[test]
    fn warning_color() {
        assert_eq!(severity_to_color(Severity::Warning), "#f9a825");
    }

    #[test]
    fn info_color() {
        assert_eq!(severity_to_color(Severity::Info), "#1565c0");
    }

    #[test]
    fn message_includes_required_fields() {
        let p = make_payload(Severity::Critical);
        let msg = build_message(&p, None);
        let att = &msg["attachments"][0];
        assert_eq!(att["color"], "#d32f2f");
        assert_eq!(att["title"], "Test alert");
        let fields: Vec<&str> = att["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["title"].as_str().unwrap())
            .collect();
        assert!(fields.contains(&"Resource"));
        assert!(fields.contains(&"Fingerprint"));
        assert!(fields.contains(&"Severity"));
    }

    #[test]
    fn channel_override_in_message() {
        let p = make_payload(Severity::Warning);
        let msg = build_message(&p, Some("#incidents"));
        assert_eq!(msg["channel"], "#incidents");
    }

    #[tokio::test]
    async fn send_returns_ok_on_200() {
        let addr =
            start_mock(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        let sink = SlackSink::new(format!("http://{}", addr));
        let p = make_payload(Severity::Critical);
        sink.send(&p).await.unwrap();
    }
}
