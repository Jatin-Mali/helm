//! End-to-end `helm run` workflows against a scripted OpenAI-compatible server.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    /// Lower-cased request heads, to check what credentials were actually sent.
    heads: Arc<Mutex<Vec<String>>>,
}

fn tool_call(name: &str, arguments: Value) -> Value {
    json!({"id":"c","model":"m","choices":[{"message":{"role":"assistant","content":null,
        "tool_calls":[{"id":"call_1","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},
        "finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":5,"total_tokens":10}})
}
fn text(content: &str) -> Value {
    json!({"id":"c","model":"m","choices":[{"message":{"role":"assistant","content":content},
        "finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":5,"total_tokens":10}})
}

/// Serves `script` in order; `Value::Null` entries hang the connection open.
fn mock(script: Vec<Value>) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let heads = Arc::new(Mutex::new(Vec::new()));
    let seen_heads = heads.clone();
    std::thread::spawn(move || {
        let mut script = script.into_iter();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 4096];
            let body = loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break Vec::new();
                }
                buffer.extend_from_slice(&chunk[..n]);
                let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
                if buffer.len() == end + 4 || seen_heads.lock().unwrap().is_empty() {
                    seen_heads.lock().unwrap().push(head.clone());
                }
                let length = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buffer.len() >= end + 4 + length {
                    break buffer[end + 4..end + 4 + length].to_vec();
                }
            };
            if let Ok(value) = serde_json::from_slice::<Value>(&body) {
                seen.lock().unwrap().push(value);
            }
            match script.next() {
                Some(Value::Null) | None => {
                    std::thread::sleep(Duration::from_secs(60));
                }
                Some(reply) => {
                    let payload = reply.to_string();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                        payload.len()
                    );
                }
            }
        }
    });
    Mock {
        url,
        requests,
        heads,
    }
}

fn command(state: &Path, workspace: &Path, server: &Mock, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_helm"));
    command
        .env("XDG_CONFIG_HOME", state.join("config"))
        .env("XDG_DATA_HOME", state.join("data"))
        .env("XDG_CACHE_HOME", state.join("cache"))
        .args(["--provider", "openai-compat", "--model", "m", "--base-url"])
        .arg(&server.url)
        .args(["--db-path", state.join("helm.db").to_str().unwrap()])
        .arg("run")
        .args(["--workspace", workspace.to_str().unwrap()])
        .args(args);
    command
}
fn sessions(state: &Path) -> Vec<Value> {
    let dir = state.join("agent-sessions");
    let mut found: Vec<Value> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| serde_json::from_slice(&std::fs::read(e.path()).unwrap()).unwrap())
        .collect();
    found.sort_by_key(|s| s["created_at"].as_i64());
    found
}

#[test]
fn read_task_completes_with_evidence_and_a_redacted_private_session() {
    let (state, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let secret = "sk-proj-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    std::fs::write(
        work.path().join("notes.txt"),
        format!("token {secret}\nhello"),
    )
    .unwrap();
    let server = mock(vec![
        tool_call("fs_read", json!({"path": "notes.txt"})),
        text("The file says hello."),
    ]);
    let output = command(state.path(), work.path(), &server, &["read notes.txt"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("hello"));
    let all = sessions(state.path());
    assert_eq!(all.len(), 1);
    assert_eq!(all[0]["outcome"], "completed");
    assert!(all[0]["evidence"].as_array().is_some_and(|e| !e.is_empty()));
    let raw = serde_json::to_string(&all[0]).unwrap();
    assert!(!raw.contains(secret), "secret persisted: {raw}");
    for request in server.requests.lock().unwrap().iter() {
        assert!(
            !request.to_string().contains(secret),
            "secret sent to provider"
        );
    }
    use std::os::unix::fs::PermissionsExt;
    let file = std::fs::read_dir(state.path().join("agent-sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn write_without_a_terminal_is_pending_never_executed_and_resume_does_not_replay() {
    let (state, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let server = mock(vec![
        tool_call("fs_write", json!({"path": "out.txt", "content": "x"})),
        text("Understood; not writing."),
    ]);
    let output = command(state.path(), work.path(), &server, &["write out.txt"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!work.path().join("out.txt").exists());
    let first = &sessions(state.path())[0];
    assert_eq!(first["outcome"], "waiting_for_approval");
    assert!(
        first["log"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "approval" && e["decision"] == "pending")
    );
    let id = first["id"].as_str().unwrap().to_owned();

    // Resume in a new process: the unanswered call is dropped from history, not executed.
    let output = command(state.path(), work.path(), &server, &["--resume", &id])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!work.path().join("out.txt").exists());
    let requests = server.requests.lock().unwrap();
    let resumed = requests.last().unwrap()["messages"].to_string();
    assert!(
        !resumed.contains("call_1"),
        "dangling tool call replayed: {resumed}"
    );
}

#[test]
fn approved_write_runs_once_through_a_terminal_and_denial_blocks_it() {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("skipping: util-linux `script` unavailable for a pseudo-terminal");
        return;
    }
    for (answer, expect_file) in [("y", true), ("n", false)] {
        let (state, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let server = mock(vec![
            tool_call("fs_write", json!({"path": "out.txt", "content": "hello"})),
            text("finished"),
        ]);
        let helm = command(state.path(), work.path(), &server, &["write out.txt"]);
        let mut line = shell_words(&helm);
        line.insert_str(0, "exec ");
        let mut child = Command::new("script")
            .args(["-qec", &line, "/dev/null"])
            .envs(helm.get_envs().filter_map(|(k, v)| v.map(|v| (k, v))))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Wait for the prompt before answering so the answer is read by the approval prompt.
        let deadline = Instant::now() + Duration::from_secs(20);
        while server.requests.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(800));
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{answer}\n").as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let shown = String::from_utf8_lossy(&output.stdout);
        assert!(shown.contains("Approval required"), "{shown}");
        assert_eq!(work.path().join("out.txt").exists(), expect_file, "{shown}");
        if expect_file {
            assert_eq!(
                std::fs::read_to_string(work.path().join("out.txt")).unwrap(),
                "hello"
            );
        }
        let session = &sessions(state.path())[0];
        let decision = if expect_file { "approved" } else { "denied" };
        assert!(
            session["log"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == "approval" && e["decision"] == decision),
            "{session}"
        );
    }
}

fn shell_words(command: &Command) -> String {
    let quote = |s: &std::ffi::OsStr| format!("'{}'", s.to_string_lossy().replace('\'', "'\\''"));
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn interrupt_cancels_a_hung_provider_and_saves_the_session() {
    let (state, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let server = mock(vec![Value::Null]);
    let child = command(state.path(), work.path(), &server, &["hang"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while server.requests.lock().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !server.requests.lock().unwrap().is_empty(),
        "provider never called"
    );
    std::thread::sleep(Duration::from_millis(300));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .unwrap();
    let started = Instant::now();
    let output = child.wait_with_output().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!output.status.success());
    assert_eq!(sessions(state.path())[0]["outcome"], "cancelled");
}

#[test]
fn both_binaries_accept_the_run_command() {
    let state = tempfile::tempdir().unwrap();
    for binary in [env!("CARGO_BIN_EXE_helm"), env!("CARGO_BIN_EXE_helmops")] {
        let output = Command::new(binary)
            .env("XDG_CONFIG_HOME", state.path().join("config"))
            .env("XDG_DATA_HOME", state.path().join("data"))
            .env("XDG_CACHE_HOME", state.path().join("cache"))
            .args(["run", "--sessions"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{binary}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn run_refuses_a_remote_target_instead_of_acting_locally() {
    let state = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_helm"))
        .env("XDG_CONFIG_HOME", state.path().join("config"))
        .env("XDG_DATA_HOME", state.path().join("data"))
        .env("XDG_CACHE_HOME", state.path().join("cache"))
        .args(["--remote", "prod", "run", "do something"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("this machine only"));
}

#[test]
fn api_key_flag_is_sent_to_an_openai_compatible_endpoint() {
    let (state, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let server = mock(vec![text("ok")]);
    let output = command(state.path(), work.path(), &server, &["say ok"])
        .args(["--api-key", "test-key-123"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let heads = server.heads.lock().unwrap();
    assert!(
        heads
            .iter()
            .any(|h| h.contains("authorization: bearer test-key-123")),
        "{heads:?}"
    );
}
