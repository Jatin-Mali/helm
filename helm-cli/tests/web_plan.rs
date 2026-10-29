use std::{
    io::{BufRead, BufReader},
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
async fn start(dir: &Path) -> Session {
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
        .env_remove("DEEPSEEK_API_KEY")
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
    let session = start(dir.path()).await;
    let finding = session.broken_finding().await;
    let plan = session
        .post("/api/plan", json!({"findingId": finding["id"]}))
        .await;
    assert!(plan.get("error").is_none(), "{plan}");
    let status = plan["plan_status"].as_str().unwrap_or_default();
    assert!(["disabled", "loading", "ready"].contains(&status), "{plan}");
}
