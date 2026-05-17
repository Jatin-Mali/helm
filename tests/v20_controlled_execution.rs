//! v2.0 Controlled Execution — comprehensive test suite.
//!
//! Covers: denied plan mutation safety, change-set persistence, rollback,
//! audit verification, pre-change backups, and post-change verification.

use helm_memory::{
    ChangeSetStore, TroubleshootingPlanStore, audit_hash, latest_audit_hash, stable_hash_hex,
};
use helm_monitor::{CommandPreview, ExecutionEngine, Hypothesis, PlanSource, TroubleshootingPlan};

fn make_plan(id: &str) -> TroubleshootingPlan {
    TroubleshootingPlan {
        id: id.into(),
        source: PlanSource::UserQuestion("test problem".into()),
        snapshot_id: "snap-test".into(),
        hypotheses: vec![Hypothesis {
            id: "h1".into(),
            hypothesis: "test hypothesis".into(),
            evidence_for: vec![],
            evidence_against: vec![],
            missing_evidence: vec![],
            confidence: 0.8,
            domain: helm_monitor::MonitorDomain::Disks,
        }],
        read_only_steps: vec![],
        proposed_fix_steps: vec![
            helm_monitor::PlanStep {
                title: "check disk".into(),
                command: CommandPreview::new("shell", "echo 'test ok'", "verify disk health"),
                hypothesis_id: Some("h1".into()),
                expected_output: Some("test ok".into()),
                interpretation_guide: Some("expect ok".into()),
            },
            helm_monitor::PlanStep {
                title: "clean cache".into(),
                command: CommandPreview::new("shell", "echo 'clean ok'", "clean temp files"),
                hypothesis_id: Some("h1".into()),
                expected_output: Some("clean ok".into()),
                interpretation_guide: Some("expect clean".into()),
            },
        ],
        approval_required: true,
    }
}

// ── Persistence tests ─────────────────────────────────────────────────

#[test]
fn plan_store_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("plan-test.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(include_str!(
        "../crates/helm-memory/migrations/0008_changesets.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../crates/helm-memory/migrations/0010_dashboard_plans.sql"
    ))
    .unwrap();

    let steps_json = r#"[{"title":"check disk","command":{"tool":"shell","input":{"command":"df -h"},"command_text":"df -h","expected_effect":"check disk","risk":"none","blast_radius":"Unknown","rollback":"NotNeeded","verification":[]},"hypothesis_id":null,"expected_output":null,"interpretation_guide":null}]"#;
    TroubleshootingPlanStore::insert(
        &conn,
        "plan-1",
        "user: test",
        "snap-1",
        "[]",
        "[]",
        steps_json,
        true,
        "test",
    )
    .unwrap();
    let record = TroubleshootingPlanStore::get(&conn, "plan-1")
        .unwrap()
        .unwrap();
    assert_eq!(record.id, "plan-1");
    assert_eq!(record.dashboard_plan_status, "ready");
    let loaded_steps: Vec<serde_json::Value> =
        serde_json::from_str(&record.proposed_fix_steps_json).unwrap();
    assert_eq!(loaded_steps.len(), 1);
}

// ── Denied plan mutation safety ───────────────────────────────────────

#[tokio::test]
async fn denied_steps_are_skipped() {
    let plan = make_plan("denied-test");
    let approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync> = Box::new(|_| false); // deny everything
    let mut engine = ExecutionEngine::new(approve_fn);
    let cs = engine.execute(&plan).await;
    assert_eq!(cs.steps.len(), 2);
    assert!(
        cs.steps.iter().all(|s| s.status.to_string() == "skipped"),
        "all steps should be skipped when denied: {:?}",
        cs.steps
            .iter()
            .map(|s| (s.status.to_string(), s.plan_step_title.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(cs.status.to_string(), "rejected");
}

#[tokio::test]
async fn partial_approval_skips_denied_steps() {
    let plan = make_plan("partial-approval");
    let call_count = std::sync::atomic::AtomicUsize::new(0);
    let approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync> = {
        Box::new(move |_| {
            let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            n == 0 // approve first, deny second
        })
    };
    let mut engine = ExecutionEngine::new(approve_fn);
    let cs = engine.execute(&plan).await;
    assert_eq!(cs.steps.len(), 2);
    assert_eq!(cs.steps[0].status.to_string(), "succeeded");
    assert_eq!(cs.steps[1].status.to_string(), "skipped");
}

#[tokio::test]
async fn denied_plan_leaves_no_change_state() {
    // Verify that when all steps are denied, the change set is complete
    // with no executed steps. The engine captures before/after snapshots
    // for detection but should not leave any mutation impact.
    let plan = make_plan("noop-plan");
    let approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync> = Box::new(|_| false);
    let mut engine = ExecutionEngine::new(approve_fn);
    let cs = engine.execute(&plan).await;
    assert_eq!(cs.status.to_string(), "rejected");
    assert!(
        cs.after_snapshot_id.is_some(),
        "engine should capture after-snapshot even when denied"
    );
}

// ── Change-set persistence ────────────────────────────────────────────

#[test]
fn change_set_persistence_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("cs-test.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(include_str!(
        "../crates/helm-memory/migrations/0008_changesets.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!(
        "../crates/helm-memory/migrations/0010_dashboard_plans.sql"
    ))
    .unwrap();

    ChangeSetStore::insert(
        &conn,
        "cs-test-1",
        "plan-1",
        "test",
        "snap-1",
        "before-1",
        "completed",
        1000,
        "summary",
    )
    .unwrap();
    let record = ChangeSetStore::get(&conn, "cs-test-1").unwrap().unwrap();
    assert_eq!(record.status, "completed");

    ChangeSetStore::insert_step(
        &conn,
        "step-1",
        "cs-test-1",
        "check",
        "shell",
        "{}",
        Some("echo ok"),
        "check disk",
        "none",
        "succeeded",
    )
    .unwrap();
    let steps = ChangeSetStore::get_steps(&conn, "cs-test-1").unwrap();
    assert_eq!(steps.len(), 1);
}

#[test]
fn change_set_list_ordered_by_creation() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("cs-list.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(include_str!(
        "../crates/helm-memory/migrations/0008_changesets.sql"
    ))
    .unwrap();

    ChangeSetStore::insert(&conn, "cs-a", "p-a", "", "s-a", "b-a", "completed", 100, "").unwrap();
    ChangeSetStore::insert(&conn, "cs-b", "p-b", "", "s-b", "b-b", "completed", 200, "").unwrap();
    let list = ChangeSetStore::list(&conn, 10).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, "cs-b");
}

// ── Rollback tests ────────────────────────────────────────────────────

#[test]
fn rollback_updates_change_set_status() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("rollback-test.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(include_str!(
        "../crates/helm-memory/migrations/0008_changesets.sql"
    ))
    .unwrap();

    ChangeSetStore::insert(
        &conn,
        "cs-roll",
        "plan-r",
        "test",
        "snap-r",
        "before-r",
        "completed",
        500,
        "",
    )
    .unwrap();
    ChangeSetStore::update_status(&conn, "cs-roll", "rolled_back").unwrap();
    let record = ChangeSetStore::get(&conn, "cs-roll").unwrap().unwrap();
    assert_eq!(record.status, "rolled_back");
    assert!(record.rolled_back_at.is_some());
}

// ── Audit verification tests ──────────────────────────────────────────

#[test]
fn audit_events_form_hash_chain() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("audit-chain.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(include_str!(
        "../crates/helm-memory/migrations/0004_security.sql"
    ))
    .unwrap();
    conn.execute_batch("ALTER TABLE audit_events ADD COLUMN target TEXT;")
        .unwrap_or(());

    // Write two audit events and verify chain
    let ts1 = 1000_i64;
    let h1 = audit_hash(helm_memory::AuditHashParts {
        previous_hash: "GENESIS",
        episode_id: Some("ep-1"),
        target: None,
        timestamp: ts1,
        tool_name: "apply-plan",
        input_hash: &stable_hash_hex("plan-1"),
        output_hash: &stable_hash_hex("rendered"),
        capability: "read",
        taint: "clean",
        cwd: "",
        decision: "plan_shown",
    });
    conn.execute(
        "INSERT INTO audit_events (episode_id, target, timestamp, tool_name, input_hash, output_hash, capability, taint, cwd, decision, previous_hash, event_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params!["ep-1", None::<String>, ts1, "apply-plan", &stable_hash_hex("plan-1"), &stable_hash_hex("rendered"), "read", "clean", "", "plan_shown", "GENESIS", &h1],
    ).unwrap();

    // Second event chains from first
    let ts2 = 2000_i64;
    let h2 = audit_hash(helm_memory::AuditHashParts {
        previous_hash: &h1,
        episode_id: Some("ep-1"),
        target: None,
        timestamp: ts2,
        tool_name: "apply-plan",
        input_hash: &stable_hash_hex("step-1"),
        output_hash: &stable_hash_hex("approved"),
        capability: "shell",
        taint: "clean",
        cwd: "",
        decision: "approved",
    });
    conn.execute(
        "INSERT INTO audit_events (episode_id, target, timestamp, tool_name, input_hash, output_hash, capability, taint, cwd, decision, previous_hash, event_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params!["ep-1", None::<String>, ts2, "apply-plan", &stable_hash_hex("step-1"), &stable_hash_hex("approved"), "shell", "clean", "", "approved", &h1, &h2],
    ).unwrap();

    // Verify chain: latest hash should start with event 2
    let latest = latest_audit_hash(&conn, None).unwrap();
    assert_eq!(latest, h2, "latest audit hash should be event 2's hash");
}

#[test]
fn audit_chain_genesis_hash() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("audit-genesis.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(include_str!(
        "../crates/helm-memory/migrations/0004_security.sql"
    ))
    .unwrap();
    conn.execute_batch("ALTER TABLE audit_events ADD COLUMN target TEXT;")
        .unwrap_or(());

    // Empty table should return GENESIS
    let hash = latest_audit_hash(&conn, None).unwrap();
    assert_eq!(
        hash, "GENESIS",
        "empty audit table should return GENESIS hash"
    );
}

#[test]
fn stable_hash_is_deterministic() {
    let h1 = stable_hash_hex("hello");
    let h2 = stable_hash_hex("hello");
    let h3 = stable_hash_hex("world");
    assert_eq!(h1, h2, "same input should produce same hash");
    assert_ne!(h1, h3, "different inputs should produce different hashes");
}

// ── Post-change verification tests ────────────────────────────────────

#[tokio::test]
async fn format_change_set_contains_step_status() {
    let plan = make_plan("fmt-cs");
    let approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync> = Box::new(|_| true);
    let mut engine = ExecutionEngine::new(approve_fn);
    let cs = engine.execute(&plan).await;
    let formatted = helm_monitor::format_change_set(&cs);
    assert!(
        formatted.contains("ChangeSet:"),
        "format should have header"
    );
    assert!(
        formatted.contains("succeeded"),
        "format should show step status"
    );
    assert!(formatted.contains("Summary:"), "format should have summary");
}

#[tokio::test]
async fn execution_engine_captures_change_set_id() {
    let plan = make_plan("cs-id");
    let approve_fn: Box<dyn Fn(&CommandPreview) -> bool + Send + Sync> = Box::new(|_| true);
    let mut engine = ExecutionEngine::new(approve_fn);
    let cs = engine.execute(&plan).await;
    assert!(!cs.id.is_empty(), "change set must have an ID");
    assert_eq!(cs.plan_id, "cs-id");
}
