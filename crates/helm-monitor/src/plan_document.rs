//! Versioned executable plans and local, exact-operation approval decisions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BlastRadius, CommandPreview, LlmFixStep, PlanStep, RollbackStatus, TroubleshootingPlan,
};

#[derive(Serialize, Deserialize)]
struct PlanStepsDocument {
    schema_version: u32,
    steps: Vec<PlanStep>,
}

pub fn encode_plan_steps(steps: &[PlanStep]) -> Result<String, serde_json::Error> {
    serde_json::to_string(&PlanStepsDocument {
        schema_version: 1,
        steps: steps.to_vec(),
    })
}

pub fn decode_plan_steps(json: &str) -> Result<Vec<PlanStep>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if value.is_array() {
        // Existing typed plans remain readable; incompatible old formats fail explicitly.
        return serde_json::from_value(value).map_err(|e| format!("invalid legacy plan: {e}"));
    }
    let document: PlanStepsDocument =
        serde_json::from_value(value).map_err(|e| format!("invalid plan document: {e}"))?;
    if document.schema_version != 1 {
        return Err(format!(
            "unsupported plan schema version {}",
            document.schema_version
        ));
    }
    Ok(document.steps)
}

pub fn decode_read_only_steps(json: &str) -> Result<Vec<PlanStep>, String> {
    if let Ok(instructions) = serde_json::from_str::<Vec<String>>(json) {
        return Ok(instructions
            .into_iter()
            .map(|instruction| PlanStep {
                title: instruction.clone(),
                command: CommandPreview {
                    tool: "instruction".into(),
                    input: serde_json::Value::Null,
                    command_text: None,
                    expected_effect: instruction,
                    risk: crate::RiskLevel::None,
                    blast_radius: BlastRadius::Unknown,
                    rollback: RollbackStatus::NotNeeded,
                    verification: vec![],
                },
                hypothesis_id: None,
                expected_output: None,
                interpretation_guide: None,
            })
            .collect());
    }
    decode_plan_steps(json)
}

pub fn llm_fix_steps(steps: &[LlmFixStep]) -> Vec<PlanStep> {
    steps
        .iter()
        .enumerate()
        .map(|(index, step)| PlanStep {
            title: format!("step-{}", index + 1),
            command: CommandPreview::new("shell", &step.command, &step.expected_effect)
                .with_risk(step.risk)
                .with_blast(BlastRadius::Unknown)
                // A model's rollback suggestion is not a verified inverse operation.
                .with_rollback(RollbackStatus::Unsupported),
            hypothesis_id: None,
            expected_output: None,
            interpretation_guide: Some(format!(
                "{}\nScope suggestion: {}\nRollback suggestion: {}",
                step.purpose, step.blast_radius, step.rollback
            )),
        })
        .collect()
}

/// A review snapshot. Titles and callback order are never authorization identities.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanApprovals {
    digest: Vec<u8>,
    cwd: PathBuf,
    boot_id: String,
    decisions: Vec<bool>,
    expires_at: std::time::Instant,
}

impl PlanApprovals {
    pub fn review(plan: &TroubleshootingPlan) -> Result<Self, String> {
        Ok(Self {
            digest: plan_digest(plan)?,
            cwd: std::env::current_dir().map_err(|e| e.to_string())?,
            boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .map_err(|e| format!("cannot bind approvals to this host: {e}"))?,
            decisions: vec![false; plan.proposed_fix_steps.len()],
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(15 * 60),
        })
    }

    pub fn decide(&mut self, index: usize, approved: bool) -> Result<(), String> {
        *self
            .decisions
            .get_mut(index)
            .ok_or("unknown approval step")? = approved;
        Ok(())
    }

    pub fn approve_remaining(&mut self, from: usize) {
        for decision in self.decisions.iter_mut().skip(from) {
            *decision = true;
        }
    }

    pub fn matches(&self, plan: &TroubleshootingPlan) -> bool {
        std::time::Instant::now() < self.expires_at
            && plan_digest(plan).is_ok_and(|digest| digest == self.digest)
            && std::env::current_dir().is_ok_and(|cwd| cwd == self.cwd)
            && std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .is_ok_and(|boot_id| boot_id == self.boot_id)
    }

    pub fn allows(&self, plan: &TroubleshootingPlan, index: usize) -> bool {
        self.matches(plan) && self.decisions.get(index).copied().unwrap_or(false)
    }
}

fn plan_digest(plan: &TroubleshootingPlan) -> Result<Vec<u8>, String> {
    let encoded = serde_json::to_vec(plan).map_err(|e| e.to_string())?;
    Ok(Sha256::digest(encoded).to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_or_different_context_requires_another_review() {
        let plan = TroubleshootingPlan::new(
            crate::PlanSource::UserQuestion("fixture".into()),
            "fixture".into(),
        );
        let mut review = PlanApprovals::review(&plan).unwrap();
        assert!(review.matches(&plan));
        review.expires_at = std::time::Instant::now();
        assert!(!review.matches(&plan));
        let mut review = PlanApprovals::review(&plan).unwrap();
        review.cwd = PathBuf::from("/different/workspace");
        assert!(!review.matches(&plan));
        let mut review = PlanApprovals::review(&plan).unwrap();
        review.boot_id = "different-host".into();
        assert!(!review.matches(&plan));
    }
}
