//! CLI acceptance for the shared execution outcomes: approval, denial, read-only, dry-run,
//! unsupported sandbox, failed verification. Each prints a consistent state, exits accordingly
//! and never changes the host unless the exact step was approved and policy allows it.
use helm_monitor::{CommandPreview, PlanSource, PlanStep, TroubleshootingPlan};
use std::{path::Path, process::{Command, Stdio}, io::Write};

fn run(state: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_helm"))
        .env("XDG_CONFIG_HOME", state.join("config"))
        .env("XDG_DATA_HOME", state.join("data"))
        .env("XDG_CACHE_HOME", state.join("cache"))
        .args(["--db-path", state.join("helm.db").to_str().unwrap()])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn store_plan(state: &Path, id: &str, steps: Vec<PlanStep>) {
    let plan = TroubleshootingPlan {
        id: id.into(),
        source: PlanSource::UserQuestion("cross-frontend".into()),
        snapshot_id: "fixture".into(),
        hypotheses: vec![],
        read_only_steps: vec![],
        proposed_fix_steps: steps,
        approval_required: true,
    };
    let (code, _, err) = run(state, &["audit", "verify"], ""); // creates and migrates the database
    assert!(code == 0, "{err}");
    let conn = rusqlite::Connection::open(state.join("helm.db")).unwrap();
    helm_memory::TroubleshootingPlanStore::insert(
        &conn,
        id,
        "user: cross-frontend",
        "fixture",
        "",
        "[]",
        "[]",
        &helm_monitor::encode_plan_steps(&plan.proposed_fix_steps).unwrap(),
        true,
        "fixture",
    )
    .unwrap();
}

fn step(command: &str, verify: Option<(&str, &str)>) -> PlanStep {
    let mut preview = CommandPreview::new("shell", command, "fixture");
    if let Some((check, _)) = verify {
        preview = preview.with_verification(CommandPreview::new("shell", check, "check"));
    }
    PlanStep {
        title: "fixture step".into(),
        command: preview,
        hypothesis_id: None,
        expected_output: verify.map(|(_, expected)| expected.to_owned()),
        interpretation_guide: None,
    }
}

#[test]
fn apply_plan_outcomes_agree_across_denial_modes_and_failed_verification() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("marker");
    let touch = format!("touch {}", marker.display());
    let mut seen = Vec::new();
    // (label, flags, stdin, expect_success, marker_expected)
    let cases: [(&str, &[&str], &str, bool, bool); 5] = [
        ("denied", &[], "n\n", true, false),
        ("read-only", &["--read-only"], "y\n", false, false),
        ("dry-run", &["--dry-run"], "y\n", true, false),
        ("sandbox-unavailable", &["--sandbox"], "y\n", false, false),
        ("approved", &[], "y\n", true, true),
    ];
    for (label, flags, stdin, success, marker_expected) in cases {
        let state = root.path().join(label);
        let id = format!("plan-{label}");
        store_plan(&state, &id, vec![step(&touch, None)]);
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(["apply-plan", &id, "--json"]);
        let (code, out, err) = run(&state, &args, stdin);
        let text = format!("{out}{err}");
        assert_eq!(code == 0, success, "{label}: exit {code}: {text}");
        assert_eq!(marker.exists(), marker_expected, "{label}: {text}");
        // Whatever happened, the audit trail it left must stay readable and verifiable.
        let (verify, said, err) = run(&state, &["audit", "verify"], "");
        assert_eq!(verify, 0, "{label}: audit verify: {said}{err}");
        seen.push((label, code, text));
        let _ = std::fs::remove_file(&marker);
    }
    // Same wording families regardless of path.
    let text_of = |label: &str| &seen.iter().find(|s| s.0 == label).unwrap().2;
    assert!(text_of("read-only").to_lowercase().contains("read-only"));
    assert!(text_of("dry-run").to_lowercase().contains("dry"));
    assert!(text_of("sandbox-unavailable").contains("refusing host execution"));
    assert!(text_of("denied").to_lowercase().contains("denied") || text_of("denied").to_lowercase().contains("skipped"));
}

#[test]
fn failed_verification_is_a_failed_change_with_a_failing_exit() {
    let state = tempfile::tempdir().unwrap();
    store_plan(
        state.path(),
        "plan-verify",
        vec![step("printf changed", Some(("printf actual", "expected")))],
    );
    let (code, out, err) = run(state.path(), &["apply-plan", "plan-verify", "--json"], "y\n");
    let text = format!("{out}{err}");
    assert_ne!(code, 0, "{text}");
    assert!(text.to_lowercase().contains("fail"), "{text}");
    let (_, list, _) = run(state.path(), &["change-set", "list"], "");
    assert!(list.to_lowercase().contains("failed"), "{list}");
}
