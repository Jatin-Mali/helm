use std::process::Command;

fn helm(state: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_helm"))
        .env("XDG_CONFIG_HOME", state.join("config"))
        .env("XDG_DATA_HOME", state.join("data"))
        .env("XDG_CACHE_HOME", state.join("cache"))
        .args(args)
        .output()
        .unwrap()
}

/// A registered endpoint that keeps failing must surface as a finding in `monitor` output
/// (regression: findings built outside the snapshot were discarded as another snapshot's).
#[test]
fn repeatedly_failing_registered_service_is_reported() {
    let state = tempfile::tempdir().unwrap();
    // Port 9 (discard) on loopback is closed on any normal machine.
    let added = helm(
        state.path(),
        &[
            "service",
            "add",
            "--id",
            "closed-port",
            "--name",
            "closed-port",
            "--type",
            "tcp-endpoint",
            "--host",
            "127.0.0.1",
            "--port",
            "9",
        ],
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let mut titles = Vec::new();
    for _ in 0..4 {
        let out = helm(
            state.path(),
            &["monitor", "--format", "json", "--profile", "quick"],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        titles = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["title"].as_str().unwrap_or("").to_owned())
            .collect();
    }
    assert!(
        titles.iter().any(|t| t.contains("closed-port")),
        "service finding missing: {titles:?}"
    );
}
