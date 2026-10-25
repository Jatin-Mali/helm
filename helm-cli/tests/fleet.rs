use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn fleet_reports_each_ssh_host_and_marks_a_dead_one_unknown_without_local_substitution() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let ssh = bin.join("ssh");
    fs::write(
        &ssh,
        "#!/bin/sh\nfor a; do host=$prev; prev=$a; cmd=$a; done\n\
         case \"$host\" in down.invalid) echo 'connect refused' >&2; exit 255;; esac\n\
         case \"$cmd\" in *monitor*) echo '{\"schema_version\":1,\"findings\":[]}';; *) echo '{}';; esac\n",
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    let config = dir.path().join("config").join("helm");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("remotes.toml"),
        "[[remotes]]\nname = \"up\"\nhost = \"up.invalid\"\n\n[[remotes]]\nname = \"down\"\nhost = \"down.invalid\"\n",
    )
    .unwrap();
    let run = || {
        let output = Command::new(env!("CARGO_BIN_EXE_helm"))
            .args(["monitor", "--fleet", "--json"])
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let rows = run();
    assert_eq!(rows[0]["host"], "up");
    assert_eq!(rows[0]["status"], "observed");
    assert_eq!(rows[1]["host"], "down");
    assert_eq!(rows[1]["status"], "unavailable");
    assert!(
        rows[1]["error"]
            .as_str()
            .unwrap()
            .contains("connect refused")
    );
    assert!(rows[1]["stale_last_known"].is_null());
    // Later outage of the first host keeps its previous answer, labeled stale.
    fs::write(&ssh, "#!/bin/sh\necho 'gone' >&2\nexit 255\n").unwrap();
    let rows = run();
    assert_eq!(rows[0]["status"], "unavailable");
    assert!(rows[0]["stale_last_known"]["observed_at"].is_string());
}
