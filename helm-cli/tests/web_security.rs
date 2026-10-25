use std::{
    io::{BufRead, BufReader},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::Duration,
};

struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn dashboard_rejects_anonymous_data_and_websocket_requests() {
    let dir = tempfile::tempdir().unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let marker = dir.path().join("service-called");
    let systemctl = bin.join("systemctl");
    std::fs::write(
        &systemctl,
        format!(
            "#!/bin/sh\nif [ \"$1\" = stop ]; then printf approved > '{}'; fi\n",
            marker.display()
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
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .args([
            "--db-path",
            dir.path().join("helm.db").to_str().unwrap(),
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
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}");
        for _ in 0..100 {
            if client.get(&url).send().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for path in ["/api/snapshot", "/ws"] {
            let response = client
                .get(format!("{url}{path}"))
                .header("Origin", &url)
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                reqwest::StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }
        let login = client
            .post(format!("{url}/api/login"))
            .header("Origin", &url)
            .json(&serde_json::json!({"code":code}))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), reqwest::StatusCode::OK);
        let cookie = login.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let csrf = login.json::<serde_json::Value>().await.unwrap()["csrf"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            client
                .post(format!("{url}/api/login"))
                .header("Origin", &url)
                .json(&serde_json::json!({"code":code}))
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        for (origin, token, expected) in [
            (
                "http://evil.test",
                csrf.as_str(),
                reqwest::StatusCode::FORBIDDEN,
            ),
            (url.as_str(), "wrong", reqwest::StatusCode::FORBIDDEN),
            (url.as_str(), csrf.as_str(), reqwest::StatusCode::OK),
        ] {
            let response = client
                .post(format!("{url}/api/service-action"))
                .header("Origin", origin)
                .header("Cookie", &cookie)
                .header("x-helm-csrf", token)
                .json(&serde_json::json!({"unit":"helm-never-target.service", "action":"stop"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            if expected == reqwest::StatusCode::OK {
                let body = response.json::<serde_json::Value>().await.unwrap();
                assert!(body["approvalId"].is_string());
                assert!(!body["ok"].as_bool().unwrap());
                assert!(!marker.exists());
                let approval = body["approvalId"].as_str().unwrap();
                let send = |unit: &str| client.post(format!("{url}/api/service-action")).header("Origin", &url).header("Cookie", &cookie).header("x-helm-csrf", &csrf).json(&serde_json::json!({"unit":unit, "action":"stop", "approvalId":approval}));
                let changed = send("changed.service").send().await.unwrap().json::<serde_json::Value>().await.unwrap();
                assert_eq!(changed["ok"], false);
                assert!(!marker.exists());
                let accepted = send("helm-never-target.service").send().await.unwrap().json::<serde_json::Value>().await.unwrap();
                assert_eq!(accepted["ok"], true, "{accepted}");
                assert_eq!(std::fs::read_to_string(&marker).unwrap(), "approved");
                std::fs::remove_file(&marker).unwrap();
                let replayed = send("helm-never-target.service").send().await.unwrap().json::<serde_json::Value>().await.unwrap();
                assert_eq!(replayed["ok"], false);
                assert!(!marker.exists());
            }
        }
        assert_eq!(
            client
                .get(format!("{url}/api/snapshot"))
                .header("Host", "attacker.test")
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
    });
}
