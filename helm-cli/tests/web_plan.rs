use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use serde_json::{Value, json};

const BROKEN_UNIT: &str = "helm-test-broken.service";

struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Session {
    _server: Server,
    client: reqwest::Client,
    url: String,
    cookie: String,
    csrf: String,
}

impl Session {
    async fn get(&self, path: &str) -> Value {
        self.client
            .get(format!("{}{path}", self.url))
            .header("Cookie", &self.cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        self.client
            .post(format!("{}{path}", self.url))
            .header("Origin", &self.url)
            .header("Cookie", &self.cookie)
            .header("x-helm-csrf", &self.csrf)
            .json(&body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn broken_finding(&self) -> Value {
        let snapshot = self.get("/api/snapshot").await;
        snapshot["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["title"].as_str().unwrap_or("").contains(BROKEN_UNIT))
            .cloned()
            .unwrap_or_else(|| panic!("no finding for {BROKEN_UNIT}: {snapshot}"))
    }
}

/// Starts `helm web` with isolated state, no daemon, and a fake `systemctl` that reports one
/// failed unit, so the live snapshot always carries a deterministic finding.
/// `provider_url`: an OpenAI-compatible endpoint to plan with; `None` leaves no provider usable.
async fn start(dir: &Path, provider_url: Option<&str>) -> Session {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let row = format!("{BROKEN_UNIT} loaded failed failed HELM test unit");
    let systemctl = bin.join("systemctl");
    std::fs::write(
        &systemctl,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n  *list-timers*) ;;\n  *list-units*|*--failed*) echo '{row}' ;;\nesac\nexit 0\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&systemctl, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut command = Command::new(env!("CARGO_BIN_EXE_helm"));
    command
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env_remove("GROQ_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("DEEPSEEK_API_KEY");
    if let Some(provider_url) = provider_url {
        command.args([
            "--provider",
            "openai-compat",
            "--model",
            "m",
            "--base-url",
            provider_url,
        ]);
    }
    command
        .args([
            "--db-path",
            dir.join("helm.db").to_str().unwrap(),
            "web",
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut server = Server(command.spawn().unwrap());
    let mut stderr = BufReader::new(server.0.stderr.take().unwrap());
    let code = loop {
        let mut line = String::new();
        assert!(
            stderr.read_line(&mut line).unwrap() > 0,
            "server exited before pairing"
        );
        if let Some((_, value)) = line.split_once("minutes): ") {
            break value.trim().to_owned();
        }
    };
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}");
    for _ in 0..100 {
        if client.get(&url).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let login = client
        .post(format!("{url}/api/login"))
        .header("Origin", &url)
        .json(&json!({"code": code}))
        .send()
        .await
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf = login.json::<Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_owned();
    Session {
        _server: server,
        client,
        url,
        cookie,
        csrf,
    }
}

#[tokio::test]
async fn plan_endpoint_resolves_live_snapshot_finding() {
    let dir = tempfile::tempdir().unwrap();
    let session = start(dir.path(), None).await;
    let finding = session.broken_finding().await;
    let plan = session
        .post("/api/plan", json!({"findingId": finding["id"]}))
        .await;
    assert!(plan.get("error").is_none(), "{plan}");
    let status = plan["plan_status"].as_str().unwrap_or_default();
    assert!(["disabled", "loading", "ready"].contains(&status), "{plan}");
}

const PLAN_TEXT: &str = "---NARRATIVE---
The unit failed on start.
---FIX PLAN---
COMMAND: systemctl status helm-test-broken.service
PURPOSE: Inspect the failed unit
EFFECT: Unit state is visible
RISK: none
BLAST: read-only service inspection
ROLLBACK: none
CAPABILITY: system.service
";

/// Serves every request with `status` and an OpenAI chat completion whose text is `content`.
fn provider(status: u16, content: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..n]);
                let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let length = String::from_utf8_lossy(&buffer[..end])
                    .to_lowercase()
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|v| v.trim().to_owned())
                    })
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                if buffer.len() >= end + 4 + length {
                    break;
                }
            }
            let body = json!({"id":"c","model":"m","choices":[{"message":{"role":"assistant","content":content},
                "finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":5,"total_tokens":10}})
            .to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

/// Polls `/api/plan` until it leaves `loading`.
async fn settled_plan(session: &Session, body: Value) -> Value {
    for _ in 0..100 {
        let plan = session.post("/api/plan", body.clone()).await;
        if plan["plan_status"] != "loading" {
            return plan;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("plan never left loading");
}

#[tokio::test]
async fn failed_plan_reports_error_and_retry_requeues() {
    let dir = tempfile::tempdir().unwrap();
    let url = provider(401, "unused");
    let session = start(dir.path(), Some(&url)).await;
    let finding = session.broken_finding().await;
    let id = json!({"findingId": finding["id"], "fingerprint": finding["fingerprint"]});
    let plan = settled_plan(&session, id.clone()).await;
    assert_eq!(plan["plan_status"], "failed", "{plan}");
    assert!(
        !plan["plan_error"].as_str().unwrap_or_default().is_empty(),
        "{plan}"
    );
    // Without an explicit retry the failure is reported, not silently regenerated.
    let again = session.post("/api/plan", id.clone()).await;
    assert_eq!(again["plan_status"], "failed", "{again}");
    let retried = session
        .post(
            "/api/plan",
            json!({"findingId": id["findingId"], "fingerprint": id["fingerprint"], "retry": true}),
        )
        .await;
    assert_eq!(retried["plan_status"], "loading", "{retried}");
}

#[tokio::test]
async fn snapshot_carries_ready_ai_plan() {
    let dir = tempfile::tempdir().unwrap();
    let url = provider(200, PLAN_TEXT);
    let session = start(dir.path(), Some(&url)).await;
    let finding = session.broken_finding().await;
    let id = json!({"findingId": finding["id"], "fingerprint": finding["fingerprint"]});
    let plan = settled_plan(&session, id).await;
    assert_eq!(plan["plan_status"], "ready", "{plan}");
    // Snapshot reads are cached for one interval; refresh publishes the plan state now.
    let refreshed = session.post("/api/refresh", json!({})).await;
    let finding = refreshed["snapshot"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["title"].as_str().unwrap_or("").contains(BROKEN_UNIT))
        .cloned()
        .unwrap();
    assert_eq!(finding["planStatus"], "ready", "{finding}");
    assert!(
        finding["aiPlan"]
            .as_str()
            .unwrap_or_default()
            .contains("The unit failed on start."),
        "{finding}"
    );
}

/// Finding ids embed the snapshot id, so a request made before the next collection carries a
/// stale id; the stable fingerprint must still resolve it.
#[tokio::test]
async fn stale_finding_id_resolves_by_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let session = start(dir.path(), None).await;
    let finding = session.broken_finding().await;
    let plan = session
        .post(
            "/api/plan",
            json!({"findingId": "from-an-older-snapshot", "fingerprint": finding["fingerprint"]}),
        )
        .await;
    assert!(plan.get("error").is_none(), "{plan}");
    assert_eq!(plan["finding_id"], finding["id"], "{plan}");
}

#[tokio::test]
async fn snapshot_reports_stopped_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let session = start(dir.path(), None).await;
    let snapshot = session.get("/api/snapshot").await;
    assert_eq!(
        snapshot["daemon"]["status"], "stopped",
        "{}",
        snapshot["daemon"]
    );
    assert_eq!(
        snapshot["daemon"]["refreshIntervalSecs"], 60,
        "{}",
        snapshot["daemon"]
    );
}

/// The header must describe the provider the server actually uses (CLI flags included).
#[tokio::test]
async fn snapshot_reports_the_served_provider() {
    let dir = tempfile::tempdir().unwrap();
    let url = provider(200, PLAN_TEXT);
    let session = start(dir.path(), Some(&url)).await;
    let snapshot = session.get("/api/snapshot").await;
    assert_eq!(
        snapshot["provider"], "openai-compat",
        "{}",
        snapshot["provider"]
    );
    assert_ne!(snapshot["aiState"], "off", "{}", snapshot["aiState"]);
}

/// Readers (page loads, extra tabs, websocket reconnects) share the background collection
/// instead of each running a full monitor cycle.
#[tokio::test]
async fn snapshot_reads_share_one_collection() {
    let dir = tempfile::tempdir().unwrap();
    let session = start(dir.path(), None).await;
    let first = session.get("/api/snapshot").await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let second = session.get("/api/snapshot").await;
    assert_eq!(first["collectedAt"], second["collectedAt"]);
}

/// An explicit refresh still forces a new collection.
#[tokio::test]
async fn refresh_forces_a_new_collection() {
    let dir = tempfile::tempdir().unwrap();
    let session = start(dir.path(), None).await;
    let first = session.get("/api/snapshot").await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let refreshed = session.post("/api/refresh", json!({})).await;
    assert_ne!(first["collectedAt"], refreshed["snapshot"]["collectedAt"]);
}

/// Accepts connections and never answers, like a provider that hangs.
fn hanging_provider() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            held.push(stream);
        }
    });
    url
}

/// A `loading` row whose generator died (server restart, outer timeout) must surface as a
/// failure with Retry instead of spinning forever.
#[tokio::test]
async fn stale_loading_plan_reports_failure() {
    let dir = tempfile::tempdir().unwrap();
    let url = hanging_provider();
    let session = start(dir.path(), Some(&url)).await;
    let finding = session.broken_finding().await;
    let body = json!({"findingId": finding["id"], "fingerprint": finding["fingerprint"]});
    assert_eq!(
        session.post("/api/plan", body.clone()).await["plan_status"],
        "loading"
    );
    let conn = rusqlite::Connection::open(dir.path().join("helm.db")).unwrap();
    let mut aged = 0;
    for _ in 0..50 {
        aged = conn
            .execute(
                "UPDATE troubleshooting_plans SET updated_at = updated_at - 600 \
                 WHERE dashboard_plan_status = 'loading'",
                [],
            )
            .unwrap();
        if aged > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(aged > 0, "no loading row was written");
    let plan = session.post("/api/plan", body).await;
    assert_eq!(plan["plan_status"], "failed", "{plan}");
    assert!(
        !plan["plan_error"].as_str().unwrap_or_default().is_empty(),
        "{plan}"
    );
}

/// Concurrent publishes must not let an older collection replace a newer cached snapshot.
#[tokio::test]
async fn concurrent_refreshes_keep_the_newest_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let session = start(dir.path(), None).await;
    let refreshes = futures_join(&session).await;
    let newest = refreshes
        .iter()
        .map(|r| r["snapshot"]["collectedAt"].as_str().unwrap().to_owned())
        .max()
        .unwrap();
    let cached = session.get("/api/snapshot").await;
    assert_eq!(cached["collectedAt"], newest);
}

async fn futures_join(session: &Session) -> Vec<Value> {
    let (a, b, c, d) = tokio::join!(
        session.post("/api/refresh", json!({})),
        session.post("/api/refresh", json!({})),
        session.post("/api/refresh", json!({})),
        session.post("/api/refresh", json!({})),
    );
    vec![a, b, c, d]
}

/// A page loaded after a plan was requested must show the plan's current state, even though
/// snapshot reads are served from the cached collection.
#[tokio::test]
async fn cached_snapshot_reflects_current_plan_state() {
    let dir = tempfile::tempdir().unwrap();
    let url = provider(200, PLAN_TEXT);
    let session = start(dir.path(), Some(&url)).await;
    let finding = session.broken_finding().await;
    let body = json!({"findingId": finding["id"], "fingerprint": finding["fingerprint"]});
    assert_eq!(settled_plan(&session, body).await["plan_status"], "ready");
    let reloaded = session.broken_finding().await;
    assert_eq!(reloaded["planStatus"], "ready", "{reloaded}");
    assert!(
        reloaded["aiPlan"]
            .as_str()
            .unwrap_or_default()
            .contains("The unit failed on start.")
    );
}
