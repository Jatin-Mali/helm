//! Process-local pairing and bounded browser sessions; credentials never enter URLs.
use axum::{
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use ring::rand::{SecureRandom, SystemRandom};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

const TTL: Duration = Duration::from_secs(15 * 60);
#[derive(Clone)]
pub struct Session {
    pub id: String,
    pub csrf: String,
    pub expires: Instant,
}
pub struct Auth {
    authority: String,
    pairing: Mutex<Option<(String, Instant)>>,
    sessions: Mutex<HashMap<String, Session>>,
}
fn token() -> String {
    let mut bytes = [0; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("OS random source unavailable");
    hex::encode(bytes)
}
impl Auth {
    pub fn new(port: u16) -> (Self, String) {
        let code = token();
        (
            Self {
                authority: format!("127.0.0.1:{port}"),
                pairing: Mutex::new(Some((code.clone(), Instant::now() + TTL))),
                sessions: Mutex::new(HashMap::new()),
            },
            code,
        )
    }
    pub fn check_host_origin(
        &self,
        headers: &HeaderMap,
        require_origin: bool,
    ) -> Result<(), StatusCode> {
        let host = headers.get("host").and_then(|h| h.to_str().ok());
        let localhost = self.authority.replacen("127.0.0.1", "localhost", 1);
        if !matches!(host, Some(h) if h == self.authority || h == localhost) {
            return Err(StatusCode::FORBIDDEN);
        }
        let origin = headers.get("origin").and_then(|h| h.to_str().ok());
        if origin.is_some_and(|o| o != format!("http://{}", host.unwrap()))
            || (require_origin && origin.is_none())
        {
            return Err(StatusCode::FORBIDDEN);
        }
        if headers
            .get("sec-fetch-site")
            .is_some_and(|h| h == "cross-site")
        {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(())
    }
    pub fn login(&self, code: &str) -> Option<Session> {
        let mut pairing = self.pairing.lock().ok()?;
        let (expected, deadline) = pairing.as_ref()?;
        if Instant::now() >= *deadline
            || code.len() != expected.len()
            || !constant_equal(code.as_bytes(), expected.as_bytes())
        {
            return None;
        }
        *pairing = None;
        let session = Session {
            id: token(),
            csrf: token(),
            expires: Instant::now() + TTL,
        };
        self.sessions
            .lock()
            .ok()?
            .insert(session.id.clone(), session.clone());
        Some(session)
    }
    pub fn authorize(&self, headers: &HeaderMap, method: &Method) -> Result<Session, StatusCode> {
        let cookie = headers
            .get("cookie")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let id = cookie
            .split(';')
            .filter_map(|p| p.trim().split_once('='))
            .find(|(k, _)| *k == "helm_session")
            .map(|(_, v)| v)
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        sessions.retain(|_, s| s.expires > Instant::now());
        let session = sessions.get(id).ok_or(StatusCode::UNAUTHORIZED)?;
        if method != Method::GET
            && method != Method::HEAD
            && headers.get("x-helm-csrf").and_then(|h| h.to_str().ok())
                != Some(session.csrf.as_str())
        {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(session.clone())
    }
}
fn constant_equal(a: &[u8], b: &[u8]) -> bool {
    // Pairing tokens have fixed length; compare via the existing constant-time HMAC verifier.
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"helm-local-pairing");
    ring::hmac::verify(&key, a, ring::hmac::sign(&key, b).as_ref()).is_ok()
}
pub fn harden(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert("cache-control", "no-store".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("referrer-policy", "no-referrer".parse().unwrap());
    headers.insert("content-security-policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    response
}
pub fn denied(status: StatusCode) -> Response {
    harden(
        (
            status,
            "Dashboard authentication or request validation failed",
        )
            .into_response(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_is_single_use_and_mutations_need_csrf() {
        let (auth, code) = Auth::new(9000);
        assert!(auth.login("wrong").is_none());
        let session = auth.login(&code).unwrap();
        assert!(auth.login(&code).is_none());
        let mut headers = HeaderMap::new();
        headers.insert("host", "127.0.0.1:9000".parse().unwrap());
        assert!(auth.check_host_origin(&headers, false).is_ok());
        assert!(auth.check_host_origin(&headers, true).is_err());
        headers.insert("origin", "http://evil.test".parse().unwrap());
        assert!(auth.check_host_origin(&headers, false).is_err());
        headers.insert(
            "cookie",
            format!("helm_session={}", session.id).parse().unwrap(),
        );
        assert!(auth.authorize(&headers, &Method::GET).is_ok());
        assert!(auth.authorize(&headers, &Method::POST).is_err());
        headers.insert("x-helm-csrf", session.csrf.parse().unwrap());
        assert!(auth.authorize(&headers, &Method::POST).is_ok());
        auth.sessions
            .lock()
            .unwrap()
            .get_mut(&session.id)
            .unwrap()
            .expires = Instant::now();
        assert!(auth.authorize(&headers, &Method::GET).is_err());
    }
}
