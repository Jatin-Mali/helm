use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn init_exits_when_standard_input_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_helm"))
        .args(["init", "--provider", "ollama", "--no-validate"])
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            let output = child.wait_with_output().unwrap();
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("input closed"),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!dir.path().join("config/helm/config.toml").exists());
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("init loops after EOF instead of reporting input closed");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
