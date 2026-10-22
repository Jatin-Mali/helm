use std::time::Duration;

use helm_monitor::{CommandPreview, ExecutionEngine, PlanSource, PlanStep, TroubleshootingPlan};

#[tokio::test]
async fn cancelling_execution_stops_the_action_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let started = dir.path().join("started");
    let escaped = dir.path().join("escaped");
    let command = format!(
        "(sleep 1; printf escaped > {}) & printf started > {}; wait",
        escaped.display(),
        started.display(),
    );
    let mut plan = TroubleshootingPlan::new(
        PlanSource::UserQuestion("cancel fixture".into()),
        "fixture".into(),
    );
    plan.proposed_fix_steps.push(PlanStep {
        title: "cancellable operation".into(),
        command: CommandPreview::new("shell", &command, "isolated test"),
        hypothesis_id: None,
        expected_output: None,
        interpretation_guide: None,
    });
    let task = tokio::spawn(async move {
        ExecutionEngine::new(Box::new(|_| true))
            .execute(&plan)
            .await
    });
    tokio::time::timeout(Duration::from_secs(30), async {
        while !started.exists() {
            assert!(
                !task.is_finished(),
                "execution exited before the action started"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        !escaped.exists(),
        "a descendant survived cancelled execution"
    );
}

#[tokio::test]
async fn execution_modes_prevent_shell_and_file_mutations() {
    let dir = tempfile::tempdir().unwrap();
    for policy in [
        helm_monitor::ExecutionPolicy {
            read_only: true,
            ..Default::default()
        },
        helm_monitor::ExecutionPolicy {
            dry_run: true,
            ..Default::default()
        },
        helm_monitor::ExecutionPolicy {
            sandbox_requested: true,
            ..Default::default()
        },
    ] {
        let marker = dir.path().join("blocked");
        let mut plan = TroubleshootingPlan::new(
            PlanSource::UserQuestion("mode fixture".into()),
            "fixture".into(),
        );
        let mut file = CommandPreview::new("fs_write", "write marker", "blocked");
        file.input = serde_json::json!({"path": marker, "content": "blocked"});
        for command in [
            file,
            CommandPreview::new(
                "shell",
                &format!("printf blocked > {}", marker.display()),
                "blocked",
            ),
        ] {
            plan.proposed_fix_steps.push(PlanStep {
                title: "blocked".into(),
                command,
                hypothesis_id: None,
                expected_output: None,
                interpretation_guide: None,
            });
        }
        let result = ExecutionEngine::new(Box::new(|_| panic!("mode must block before approval")))
            .with_policy(policy)
            .execute(&plan)
            .await;
        assert!(
            result
                .steps
                .iter()
                .all(|step| step.status == helm_monitor::StepStatus::Skipped)
        );
        assert!(!marker.exists());
    }
}
