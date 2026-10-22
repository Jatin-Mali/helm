use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn cli_execution_modes_block_host_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    fs::create_dir(&bin_dir).unwrap();
    let marker = dir.path().join("unexpected-launch");
    for name in ["ssh", "scp", "bwrap"] {
        let path = bin_dir.join(name);
        fs::write(
            &path,
            "#!/bin/sh\nprintf called >> \"$HELM_TEST_MARKER\"\nexit 23\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    for (flags, operation, succeeds, message) in [
        (
            vec!["--read-only"],
            vec!["bootstrap", "fixture.invalid"],
            false,
            "read-only",
        ),
        (
            vec!["--dry-run"],
            vec!["bootstrap", "fixture.invalid"],
            true,
            "Dry run",
        ),
        (
            vec!["--sandbox"],
            vec!["bootstrap", "fixture.invalid"],
            false,
            "refusing host execution",
        ),
        (
            vec!["--read-only"],
            vec!["agent", "install", "fixture", "--host", "fixture.invalid"],
            false,
            "read-only",
        ),
        (
            vec!["--dry-run"],
            vec!["agent", "install", "fixture", "--host", "fixture.invalid"],
            true,
            "Dry run",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_helm"))
            .args(&flags)
            .args(&operation)
            .env("PATH", &bin_dir)
            .env("HELM_TEST_MARKER", &marker)
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.status.success(),
            succeeds,
            "{flags:?} {operation:?}: {text}"
        );
        assert!(text.contains(message), "{flags:?} {operation:?}: {text}");
        assert!(
            !marker.exists(),
            "restricted mode launched an external mutator"
        );
    }
}
