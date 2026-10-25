//! Inbound pipeline webhook authentication, verified against the raw request bytes.
//!
//! Each provider has its own scheme; a provider without an implemented scheme is rejected
//! rather than accepted unauthenticated.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ring::hmac;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Generic scheme clock tolerance, and how long deliveries are remembered for deduplication.
pub const GENERIC_TOLERANCE_SECS: i64 = 300;
pub const REPLAY_WINDOW_DAYS: i64 = 7;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebhookAuthError {
    #[error("webhook ingress is not supported for provider `{0}`; use github, gitlab or generic")]
    Unsupported(String),
    #[error("missing or malformed `{0}` header")]
    MissingHeader(&'static str),
    #[error("signature does not match the request body")]
    BadSignature,
    #[error("timestamp is outside the accepted window")]
    Stale,
}

/// Header names must be lower-case.
pub type Headers = HashMap<String, String>;

fn header<'a>(headers: &'a Headers, name: &'static str) -> Result<&'a str, WebhookAuthError> {
    headers
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(WebhookAuthError::MissingHeader(name))
}

fn hmac_sha256_matches(secret: &[u8], message: &[&[u8]], claimed_hex: &str) -> bool {
    let Ok(claimed) = hex::decode(claimed_hex.trim()) else {
        return false;
    };
    // `hmac::verify` compares tags in constant time.
    hmac::verify(
        &hmac::Key::new(hmac::HMAC_SHA256, secret),
        &message.concat(),
        &claimed,
    )
    .is_ok()
}

/// Constant-time equality of two secrets of any length.
fn secrets_equal(left: &[u8], right: &[u8]) -> bool {
    let key = hmac::Key::new(hmac::HMAC_SHA256, b"helm-webhook-token-compare");
    hmac::verify(&key, right, hmac::sign(&key, left).as_ref()).is_ok()
}

/// Verifies the request and returns the delivery id used for replay protection.
pub fn verify(
    provider: &str,
    secret: &[u8],
    headers: &Headers,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<String, WebhookAuthError> {
    match provider.to_ascii_lowercase().as_str() {
        "github" | "github_actions" | "github-actions" => {
            let signature = header(headers, "x-hub-signature-256")?
                .strip_prefix("sha256=")
                .ok_or(WebhookAuthError::MissingHeader("x-hub-signature-256"))?;
            let delivery = header(headers, "x-github-delivery")?;
            if !hmac_sha256_matches(secret, &[body], signature) {
                return Err(WebhookAuthError::BadSignature);
            }
            Ok(format!("github:{delivery}"))
        }
        "gitlab" | "gitlab_ci" | "gitlab-ci" => {
            let token = header(headers, "x-gitlab-token")?;
            if !secrets_equal(token.as_bytes(), secret) {
                return Err(WebhookAuthError::BadSignature);
            }
            // Older GitLab versions send no delivery id; the body digest then identifies a replay.
            Ok(
                match headers.get("x-gitlab-event-uuid").filter(|v| !v.is_empty()) {
                    Some(uuid) => format!("gitlab:{uuid}"),
                    None => format!("gitlab:{}", hex::encode(Sha256::digest(body))),
                },
            )
        }
        "generic" | "helm" => {
            let timestamp = header(headers, "x-helm-timestamp")?;
            let signature = header(headers, "x-helm-signature")?
                .strip_prefix("sha256=")
                .ok_or(WebhookAuthError::MissingHeader("x-helm-signature"))?;
            let sent: i64 = timestamp
                .parse()
                .map_err(|_| WebhookAuthError::MissingHeader("x-helm-timestamp"))?;
            if (now.timestamp() - sent).abs() > GENERIC_TOLERANCE_SECS {
                return Err(WebhookAuthError::Stale);
            }
            if !hmac_sha256_matches(secret, &[timestamp.as_bytes(), b".", body], signature) {
                return Err(WebhookAuthError::BadSignature);
            }
            Ok(format!("helm:{signature}"))
        }
        other => Err(WebhookAuthError::Unsupported(other.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &[u8], parts: &[&[u8]]) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        let mut context = hmac::Context::with_key(&key);
        for part in parts {
            context.update(part);
        }
        hex::encode(context.sign().as_ref())
    }

    fn headers(pairs: &[(&str, &str)]) -> Headers {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn hmac_matches_the_published_github_example() {
        // https://docs.github.com/en/webhooks (validating webhook deliveries) test vector.
        let secret = b"It's a Secret to Everybody";
        let body = b"Hello, World!";
        let expected = "757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert_eq!(sign(secret, &[body]), expected);
        let h = headers(&[
            ("x-hub-signature-256", &format!("sha256={expected}")),
            ("x-github-delivery", "d-1"),
        ]);
        assert_eq!(
            verify("github", secret, &h, body, Utc::now()).unwrap(),
            "github:d-1"
        );
    }

    #[test]
    fn tampered_body_wrong_secret_and_missing_headers_are_rejected() {
        let secret = b"s3cret";
        let sig = sign(secret, &[b"{\"status\":\"ok\"}"]);
        let h = headers(&[
            ("x-hub-signature-256", &format!("sha256={sig}")),
            ("x-github-delivery", "d"),
        ]);
        assert_eq!(
            verify("github", secret, &h, b"{\"status\":\"failed\"}", Utc::now()),
            Err(WebhookAuthError::BadSignature)
        );
        assert_eq!(
            verify("github", b"other", &h, b"{\"status\":\"ok\"}", Utc::now()),
            Err(WebhookAuthError::BadSignature)
        );
        assert_eq!(
            verify("github", secret, &headers(&[]), b"x", Utc::now()),
            Err(WebhookAuthError::MissingHeader("x-hub-signature-256"))
        );
        let not_hex = headers(&[
            ("x-hub-signature-256", "sha256=zz"),
            ("x-github-delivery", "d"),
        ]);
        assert_eq!(
            verify("github", secret, &not_hex, b"x", Utc::now()),
            Err(WebhookAuthError::BadSignature)
        );
    }

    #[test]
    fn gitlab_token_is_compared_exactly_and_replays_have_stable_ids() {
        let h = headers(&[("x-gitlab-token", "tok")]);
        let id = verify("gitlab", b"tok", &h, b"body", Utc::now()).unwrap();
        assert_eq!(
            id,
            verify("gitlab", b"tok", &h, b"body", Utc::now()).unwrap()
        );
        assert_ne!(
            id,
            verify("gitlab", b"tok", &h, b"other", Utc::now()).unwrap()
        );
        assert_eq!(
            verify("gitlab", b"tok2", &h, b"body", Utc::now()),
            Err(WebhookAuthError::BadSignature)
        );
        assert_eq!(
            verify("gitlab", b"to", &h, b"body", Utc::now()),
            Err(WebhookAuthError::BadSignature)
        );
    }

    #[test]
    fn generic_scheme_binds_the_timestamp_and_rejects_old_requests() {
        let secret = b"k";
        let now = Utc::now();
        let ts = now.timestamp().to_string();
        let sig = sign(secret, &[ts.as_bytes(), b".", b"{}"]);
        let h = headers(&[
            ("x-helm-timestamp", &ts),
            ("x-helm-signature", &format!("sha256={sig}")),
        ]);
        assert!(verify("generic", secret, &h, b"{}", now).is_ok());
        // Same signature, much later: stale.
        let later = now + chrono::Duration::seconds(GENERIC_TOLERANCE_SECS + 1);
        assert_eq!(
            verify("generic", secret, &h, b"{}", later),
            Err(WebhookAuthError::Stale)
        );
        // Changing the timestamp header invalidates the signature.
        let forged_ts = (now.timestamp() + 1).to_string();
        let forged = headers(&[
            ("x-helm-timestamp", &forged_ts),
            ("x-helm-signature", &format!("sha256={sig}")),
        ]);
        assert_eq!(
            verify("generic", secret, &forged, b"{}", now),
            Err(WebhookAuthError::BadSignature)
        );
    }

    #[test]
    fn providers_without_a_verification_scheme_stay_disabled() {
        assert_eq!(
            verify("jenkins", b"k", &headers(&[]), b"{}", Utc::now()),
            Err(WebhookAuthError::Unsupported("jenkins".into()))
        );
    }
}
