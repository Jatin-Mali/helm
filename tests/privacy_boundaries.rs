use helm_core::{Capability, Taint};
use helm_memory::{AuditEventInput, MemoryStore, SnapshotStore};
use helm_monitor::troubleshoot::{FindingSummaryFields, build_narrative_prompt};

const SENTINELS: [&str; 3] = [
    "hunter2-sentinel",
    "AKIAABCDEFGHIJKLMNOP",
    "/helm/secrets.toml",
];

fn assert_clean(label: &str, text: &str) {
    for sentinel in SENTINELS {
        assert!(
            !text.contains(sentinel),
            "{label} leaked {sentinel}: {text}"
        );
    }
}

#[test]
fn provider_prompt_never_carries_credentials_or_protected_paths() {
    let prompt = build_narrative_prompt(&FindingSummaryFields {
        title: "login failures".into(),
        severity: "warning".into(),
        affected_resource: "sshd".into(),
        evidence_summaries: vec![
            "app log: db_password=hunter2-sentinel".into(),
            "env: AWS key AKIAABCDEFGHIJKLMNOP".into(),
            "read /home/u/.config/helm/secrets.toml".into(),
        ],
        detector_id: "logs".into(),
        impact: "postgres://app:hunter2-sentinel@db/x unreachable".into(),
    });
    assert_clean("prompt", &prompt);
    assert!(
        prompt.contains("login failures"),
        "ordinary evidence survives"
    );
}

#[tokio::test]
async fn database_rows_and_audit_events_are_redacted_at_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("state").join("helm.db");
    let store = MemoryStore::open(&db).await.unwrap();
    store
        .append_audit_event(AuditEventInput {
            episode_id: Some("ep".into()),
            target: Some("host-1".into()),
            tool_name: "shell".into(),
            input_hash: "i".into(),
            output_hash: "o".into(),
            capability: Capability::ShellShell,
            taint: Taint::User,
            cwd: "/home/u/.config/helm/secrets.toml".into(),
            decision: "token=hunter2-sentinel".into(),
        })
        .await
        .unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    SnapshotStore::insert(
        &conn,
        r#"{"id":"s","host":{"hostname":"h"},"collected_at":"2026-01-01T00:00:00Z",
            "domains":{"logs":"api_key: hunter2-sentinel"},"collector_errors":[]}"#,
        r#"[{"evidence":"AKIAABCDEFGHIJKLMNOP"}]"#,
    )
    .unwrap();
    let mut dump = String::new();
    for sql in [
        "SELECT target||' '||cwd||' '||decision FROM audit_events",
        "SELECT domains_json||findings_json FROM snapshots",
    ] {
        let rows: Vec<String> = conn
            .prepare(sql)
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        dump.push_str(&rows.join("\n"));
    }
    assert_clean("database", &dump);
    assert!(
        store.verify_audit_chain().await.unwrap().ok,
        "redacted rows still verify"
    );
}

#[tokio::test]
async fn pre_upgrade_database_keeps_history_and_still_verifies() {
    // A database written by the previous (FNV, unversioned) audit format.
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("old.db");
    {
        let store = MemoryStore::open(&db).await.unwrap();
        drop(store);
        let conn = rusqlite::Connection::open(&db).unwrap();
        let payload = "GENESIS|ep||1|shell|i|o|shell.shell|clean|/|allow";
        let mut h = 0xcbf29ce484222325_u64;
        for b in payload.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        conn.execute(
            "INSERT INTO audit_events (episode_id,target,timestamp,tool_name,input_hash,output_hash,capability,taint,cwd,decision,previous_hash,event_hash)
             VALUES ('ep',NULL,1,'shell','i','o','shell.shell','clean','/','allow','GENESIS',?1)",
            [format!("{h:016x}")],
        )
        .unwrap();
    }
    let store = MemoryStore::open(&db).await.unwrap();
    store
        .append_audit_event(AuditEventInput {
            episode_id: Some("ep".into()),
            target: None,
            tool_name: "shell".into(),
            input_hash: "i".into(),
            output_hash: "o".into(),
            capability: Capability::ShellShell,
            taint: Taint::User,
            cwd: "/".into(),
            decision: "allow".into(),
        })
        .await
        .unwrap();
    let verification = store.verify_audit_chain().await.unwrap();
    assert!(verification.ok, "{verification:?}");
    assert_eq!(verification.checked, 2);
    assert_eq!(verification.legacy_events, 1);
}
