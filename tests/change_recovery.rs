use helm_monitor::{CommandPreview, ExecutionEngine, PlanSource, PlanStep, TroubleshootingPlan};

#[tokio::test]
async fn recovery_file_write_retains_original_material() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("original.bin");
    std::fs::write(&path, [0, 255, 42, 10]).unwrap();
    let mut command = CommandPreview::new("fs_write", "write fixture", "replace isolated file");
    command.input = serde_json::json!({"path": path, "content": "replacement"});
    let mut plan = TroubleshootingPlan::new(
        PlanSource::UserQuestion("backup fixture".into()),
        "fixture".into(),
    );
    plan.proposed_fix_steps.push(PlanStep {
        title: "write".into(),
        command,
        hypothesis_id: None,
        expected_output: None,
        interpretation_guide: None,
    });
    let result = ExecutionEngine::new(Box::new(|_| true))
        .execute(&plan)
        .await;
    assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
    assert_eq!(result.backups.len(), 1);
    assert!(
        !result.backups[0].checksum_before.is_empty(),
        "original checksum was lost"
    );
    assert!(
        !result.backups[0].backup_content.is_empty(),
        "original bytes were lost"
    );
}
