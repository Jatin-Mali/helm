use std::time::Duration;

use serde_json;
use tokio::time::sleep;

use super::{AlertPayload, AlertSink, SendFuture};

pub struct WebhookSink {
    pub url: String,
    retry_max: u32,
    retry_backoff_ms: u64,
    client: reqwest::Client,
}

impl WebhookSink {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            retry_max: 3,
            retry_backoff_ms: 1000,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_retry_policy(mut self, retry_max: u32, retry_backoff_ms: u64) -> Self {
        self.retry_max = retry_max.max(1);
        self.retry_backoff_ms = retry_backoff_ms.max(1);
        self
    }
}

impl AlertSink for WebhookSink {
    fn send<'a>(&'a self, alert: &'a AlertPayload) -> SendFuture<'a> {
        Box::pin(async move {
            let body = serde_json::to_string(alert)?;
            for attempt in 0u32..self.retry_max {
                if attempt > 0 {
                    let multiplier = 2u64.pow(attempt - 1);
                    sleep(Duration::from_millis(
                        self.retry_backoff_ms.saturating_mul(multiplier),
                    ))
                    .await;
                }
                let result = self
                    .client
                    .post(&self.url)
                    .header("Content-Type", "application/json")
                    .body(body.clone())
                    .send()
                    .await;
                match result {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            return Ok(());
                        } else if status.is_client_error() {
                            return Err(format!("Webhook rejected ({})", status).into());
                        }
                        eprintln!(
                            "WARN webhook attempt {}/{} got {}",
                            attempt + 1,
                            self.retry_max,
                            status
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "WARN webhook attempt {}/{} error: {}",
                            attempt + 1,
                            self.retry_max,
                            e
                        );
                    }
                }
            }
            Err(format!("Webhook delivery failed after {} attempts", self.retry_max).into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::{Confidence, Finding, MonitorDomain, Severity};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn make_payload() -> AlertPayload {
        let f = Finding::new(
            "snap",
            "det",
            "host/svc",
            "Test",
            Severity::Critical,
            Confidence::High,
            MonitorDomain::Services,
        );
        AlertPayload::from(&f)
    }

    async fn start_mock_server(responses: Vec<&'static [u8]>) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for response in responses {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let mut buf = vec![0u8; 8192];
                    let _ = socket.read(&mut buf).await;
                    let _ = socket.write_all(response).await;
                }
            }
        });
        addr
    }

    #[tokio::test]
    async fn send_success_on_200() {
        let addr = start_mock_server(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ])
        .await;
        let sink = WebhookSink::new(format!("http://{}", addr));
        let p = make_payload();
        sink.send(&p).await.unwrap();
    }

    #[tokio::test]
    async fn no_retry_on_400() {
        let addr = start_mock_server(vec![
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ])
        .await;
        let sink = WebhookSink::new(format!("http://{}", addr));
        let p = make_payload();
        let result = sink.send(&p).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn payload_serializes_required_fields() {
        let p = make_payload();
        let json = serde_json::to_value(&p).unwrap();
        assert!(json.get("fingerprint").is_some());
        assert!(json.get("severity").is_some());
        assert!(json.get("title").is_some());
        assert!(json.get("timestamp").is_some());
        assert!(json.get("affected_resource").is_some());
    }
}
