use std::{
    path::Path,
    process::{Command, Output},
    time::{Duration, Instant},
};

fn cli(dir: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_helm"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .args(["--db-path", dir.join("helm.db").to_str().unwrap()])
        .args(arguments)
        .output()
        .unwrap()
}
fn state_path(dir: &Path) -> std::path::PathBuf {
    dir.join("data/helm/daemon-state.json")
}
struct StopDaemon(std::path::PathBuf);
impl Drop for StopDaemon {
    fn drop(&mut self) {
        let _ = cli(&self.0, &["daemon", "stop"]);
    }
}

#[test]
fn stale_heartbeat_is_not_reported_as_running() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(state_path(dir.path()).parent().unwrap()).unwrap();
    std::fs::write(
        state_path(dir.path()),
        serde_json::json!({
            "pid": std::process::id(), "status":"running", "started_at":null,
            "last_heartbeat":"2000-01-01T00:00:00Z", "last_success_at":null,
            "last_error":null, "last_snapshot_id":null, "last_finding_count":0,
            "refresh_interval_secs":5, "db_path":"fixture"
        })
        .to_string(),
    )
    .unwrap();
    let output = cli(dir.path(), &["daemon", "status"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("helm daemon stale"), "{stdout}");
}

#[test]
fn daemon_starts_stops_and_restarts_with_isolated_state() {
    let dir = tempfile::tempdir().unwrap();
    let _cleanup = StopDaemon(dir.path().to_owned());
    for _ in 0..2 {
        let start = cli(dir.path(), &["daemon", "start", "--interval", "5"]);
        assert!(
            start.status.success(),
            "{}",
            String::from_utf8_lossy(&start.stderr)
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let pid = loop {
            let state: serde_json::Value =
                serde_json::from_slice(&std::fs::read(state_path(dir.path())).unwrap()).unwrap();
            if state["pid"].is_u64() && state["last_heartbeat"].is_string() {
                break state["pid"].as_u64().unwrap();
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(Path::new(&format!("/proc/{pid}")).exists());
        let stop = cli(dir.path(), &["daemon", "stop"]);
        assert!(
            stop.status.success(),
            "{}",
            String::from_utf8_lossy(&stop.stderr)
        );
        let status = cli(dir.path(), &["daemon", "status"]);
        assert!(
            String::from_utf8_lossy(&status.stdout).contains("stopped"),
            "{}",
            String::from_utf8_lossy(&status.stdout)
        );
    }
}
