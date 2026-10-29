use std::process::Command;

#[test]
fn help_and_version_succeed_and_invalid_arguments_fail() {
    for binary in [env!("CARGO_BIN_EXE_helm"), env!("CARGO_BIN_EXE_helmops")] {
        for args in [
            vec!["--help"],
            vec!["--version"],
            vec!["remote", "add", "--help"],
        ] {
            let output = Command::new(binary).args(&args).output().unwrap();
            assert!(output.status.success(), "{binary} {args:?}");
            assert!(!output.stdout.is_empty());
        }
        assert!(
            !Command::new(binary)
                .args(["monitor", "--bogus"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn every_subcommand_has_a_description() {
    let output = Command::new(env!("CARGO_BIN_EXE_helm"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8(output.stdout).unwrap();
    let undocumented: Vec<&str> = help
        .lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter(|line| line.split_whitespace().count() < 2)
        .map(str::trim)
        .collect();
    assert!(undocumented.is_empty(), "no description: {undocumented:?}");
}

#[test]
fn config_get_without_a_config_file_reports_unset() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_helm"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .args(["config", "get", "provider.kind"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "an unset key is not a value");
    assert!(stderr.contains("not set"), "{stderr}");
    assert!(!stderr.contains("os error"), "{stderr}");
}
