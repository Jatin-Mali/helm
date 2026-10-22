use std::sync::atomic::{AtomicUsize, Ordering};

use helm_monitor::{
    CommandPreview, ExecutionEngine, PlanSource, PlanStep, TroubleshootingPlan,
    execute::{ChangeSetStatus, StepStatus},
};

fn plan(commands: Vec<CommandPreview>) -> TroubleshootingPlan {
    TroubleshootingPlan {
        id: "approval-regression".into(),
        source: PlanSource::UserQuestion("isolated approval test".into()),
        snapshot_id: "fixture".into(),
        hypotheses: vec![],
        read_only_steps: vec![],
        proposed_fix_steps: commands
            .into_iter()
            .map(|command| PlanStep {
                title: "duplicate title".into(),
                command,
                hypothesis_id: None,
                expected_output: None,
                interpretation_guide: None,
            })
            .collect(),
        approval_required: true,
    }
}

#[tokio::test]
async fn denied_preflight_cannot_execute_shell_payload() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("denied-marker");
    let command = format!(
        "systemctl start helm-audit-absent; printf denied > {}",
        marker.display()
    );
    let plan = plan(vec![CommandPreview::new(
        "shell",
        &command,
        "must be denied",
    )]);
    let mut engine = ExecutionEngine::new(Box::new(|_| false));
    let result = engine.execute(&plan).await;
    assert_eq!(result.steps[0].status, StepStatus::Skipped);
    assert!(
        !marker.exists(),
        "denied preflight executed a shell payload"
    );
}

#[tokio::test]
async fn idempotent_step_cannot_shift_approval_to_denied_step() {
    let temp = tempfile::tempdir().unwrap();
    let existing = temp.path().join("existing");
    let marker = temp.path().join("denied-marker");
    std::fs::write(&existing, "unchanged").unwrap();
    let mut first = CommandPreview::new("fs_write", "same content", "already satisfied");
    first.input = serde_json::json!({"path": existing, "content": "unchanged"});
    let plan = plan(vec![
        first,
        CommandPreview::new(
            "shell",
            &format!("printf denied > {}", marker.display()),
            "denied",
        ),
    ]);
    let position = AtomicUsize::new(0);
    let mut engine = ExecutionEngine::new(Box::new(move |_| {
        position.fetch_add(1, Ordering::Relaxed) == 0
    }));
    let result = engine.execute(&plan).await;
    assert_eq!(result.steps[1].status, StepStatus::Skipped);
    assert!(
        !marker.exists(),
        "a skipped step lent approval to a denied step"
    );
}

#[tokio::test]
async fn successful_exit_cannot_hide_verification_mismatch() {
    let mut plan = plan(vec![
        CommandPreview::new("shell", "printf changed", "fixture")
            .with_verification(CommandPreview::new("shell", "printf actual", "check")),
    ]);
    plan.proposed_fix_steps[0].expected_output = Some("expected".into());
    let mut engine = ExecutionEngine::new(Box::new(|_| true));
    let result = engine.execute(&plan).await;
    assert_eq!(result.steps[0].status, StepStatus::Failed);
    assert_eq!(result.status, ChangeSetStatus::Failed);
}

#[test]
fn review_binds_exact_steps_and_plan_revision() {
    let original = plan(vec![
        CommandPreview::new("shell", "printf first", "first"),
        CommandPreview::new("shell", "printf second", "second"),
    ]);
    let mut approvals = helm_monitor::PlanApprovals::review(&original).unwrap();
    approvals.decide(0, true).unwrap();
    assert!(approvals.allows(&original, 0));
    assert!(!approvals.allows(&original, 1));
    assert!(!approvals.allows(&original, 2));
    let mut changed = original.clone();
    changed.proposed_fix_steps.swap(0, 1);
    assert!(!approvals.allows(&changed, 0));
    changed = original.clone();
    changed.proposed_fix_steps[0].command.input = serde_json::json!({"command": "different"});
    assert!(!approvals.allows(&changed, 0));
    changed = original.clone();
    changed.proposed_fix_steps[0]
        .command
        .verification
        .push(CommandPreview::new("shell", "other", "other"));
    assert!(!approvals.allows(&changed, 0));
    changed = original.clone();
    changed.id = "replaced-plan".into();
    assert!(!approvals.allows(&changed, 0));
}

#[tokio::test]
async fn verification_cannot_hide_an_additional_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("hidden-write");
    let plan = plan(vec![
        CommandPreview::new("shell", "printf approved", "fixture").with_verification(
            CommandPreview::new(
                "shell",
                &format!("printf hidden > {}", marker.display()),
                "check",
            ),
        ),
    ]);
    let mut engine = ExecutionEngine::new(Box::new(|_| true));
    let result = engine.execute(&plan).await;
    assert!(
        !marker.exists(),
        "verification performed an unreviewed mutation"
    );
    assert_eq!(result.status, ChangeSetStatus::Failed);
}

#[test]
fn approving_remaining_preserves_prior_denials() {
    let plan = plan(vec![CommandPreview::new("shell", "true", "fixture"); 3]);
    let mut approvals = helm_monitor::PlanApprovals::review(&plan).unwrap();
    approvals.decide(0, false).unwrap();
    approvals.approve_remaining(1);
    assert!(!approvals.allows(&plan, 0));
    assert!(approvals.allows(&plan, 1));
    assert!(approvals.allows(&plan, 2));
}

#[tokio::test]
async fn changed_review_cannot_execute_and_approvals_are_consumed() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("unreviewed-marker");
    let mut plan = plan(vec![CommandPreview::new(
        "shell",
        "printf reviewed",
        "fixture",
    )]);
    let mut approvals = helm_monitor::PlanApprovals::review(&plan).unwrap();
    approvals.decide(0, true).unwrap();
    plan.proposed_fix_steps[0].command = CommandPreview::new(
        "shell",
        &format!("printf changed > {}", marker.display()),
        "unreviewed",
    );
    let mut engine = ExecutionEngine::with_approvals(approvals);
    assert_eq!(
        engine.execute(&plan).await.status,
        ChangeSetStatus::Rejected
    );
    plan.proposed_fix_steps[0].command = CommandPreview::new("shell", "printf reviewed", "fixture");
    assert_eq!(
        engine.execute(&plan).await.status,
        ChangeSetStatus::Rejected
    );
    assert!(!marker.exists());
}

#[test]
fn generated_plan_roundtrips_and_incompatible_formats_fail() {
    let steps = vec![helm_monitor::LlmFixStep {
        command: "printf approved".into(),
        purpose: "fixture".into(),
        expected_effect: "prints approved".into(),
        risk: helm_monitor::RiskLevel::Low,
        blast_radius: "service example".into(),
        rollback: "restart example".into(),
        required_capability: "shell.exec".into(),
        binary_warnings: vec![],
    }];
    let canonical = helm_monitor::llm_fix_steps(&steps);
    let json = helm_monitor::encode_plan_steps(&canonical).unwrap();
    assert_eq!(helm_monitor::decode_plan_steps(&json).unwrap(), canonical);
    assert_eq!(
        helm_monitor::decode_plan_steps(&serde_json::to_string(&canonical).unwrap()).unwrap(),
        canonical
    );
    assert!(helm_monitor::decode_plan_steps(&serde_json::to_string(&steps).unwrap()).is_err());
    assert!(helm_monitor::decode_plan_steps(r#"{"schema_version":2,"steps":[]}"#).is_err());
    assert!(helm_monitor::decode_plan_steps("not json").is_err());
    assert_eq!(
        canonical[0].command.rollback,
        helm_monitor::RollbackStatus::Unsupported
    );
}
