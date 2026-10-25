use std::process::Command;

#[test]
fn alternate_db_path_never_touches_default_database() {
    let dir = tempfile::tempdir().unwrap();
    let alt = dir.path().join("alt").join("helm.db");
    let data = dir.path().join("data");
    // Pre-create the default location so a stray write succeeds instead of failing to open.
    std::fs::create_dir_all(data.join("helm")).unwrap();
    for args in [
        vec!["snapshot", "--profile", "quick", "--json"],
        vec!["monitor", "--profile", "quick", "--json"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_helm"))
            .arg("--db-path")
            .arg(&alt)
            .args(&args)
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{args:?} failed: {text}");
        assert!(
            alt.exists(),
            "{args:?} did not create the requested database: {text}"
        );
        assert!(
            !data.join("helm").join("helm.db").exists(),
            "{args:?} wrote the default database"
        );
    }
}
