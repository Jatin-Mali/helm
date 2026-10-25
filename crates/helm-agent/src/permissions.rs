//! Exact, expiring human decisions. Model inputs cannot create a decision.
use crate::ToolCall;
use helm_monitor::{PlanApprovals, TroubleshootingPlan};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

struct Review {
    call: ToolCall,
    plan: TroubleshootingPlan,
    approvals: PlanApprovals,
    decision: Option<bool>,
    description: String,
}
#[derive(Default)]
pub(crate) struct Permissions {
    pending: Mutex<HashMap<String, Review>>,
    consumed: Mutex<HashSet<String>>,
}
pub(crate) enum Decision {
    Required(String),
    Approved(Box<TroubleshootingPlan>),
}
impl Permissions {
    pub fn decide(&self, call: &ToolCall, approved: bool) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "permission lock poisoned")?;
        let review = pending
            .get_mut(&call.id)
            .ok_or("no pending review for this call")?;
        if review.call != *call || !review.approvals.matches(&review.plan) {
            return Err("call or approval context changed; review again".into());
        }
        review.approvals.decide(0, approved)?;
        review.decision = Some(approved);
        Ok(())
    }
    pub fn check(
        &self,
        call: &ToolCall,
        plan: TroubleshootingPlan,
        description: String,
    ) -> Result<Decision, String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "permission lock poisoned")?;
        let mut consumed = self
            .consumed
            .lock()
            .map_err(|_| "permission lock poisoned")?;
        if consumed.contains(&call.id) {
            return Err("this approval has already been consumed".into());
        }
        if let Some(review) = pending.get(&call.id) {
            if review.call != *call || !review.approvals.matches(&review.plan) {
                return Err("call or approval context changed; review again".into());
            }
            if review.decision.is_none() {
                return Ok(Decision::Required(review.description.clone()));
            }
            let review = pending.remove(&call.id).unwrap();
            consumed.insert(call.id.clone());
            if !review.approvals.allows(&review.plan, 0) {
                return Err("operator denied this exact operation".into());
            }
            return Ok(Decision::Approved(Box::new(review.plan)));
        }
        // Bound prompt floods, including consumed IDs; new authority requires a fresh session.
        if pending.len() + consumed.len() >= 512 {
            return Err("session approval limit reached".into());
        }
        let approvals = PlanApprovals::review(&plan)?;
        pending.insert(
            call.id.clone(),
            Review {
                call: call.clone(),
                plan,
                approvals,
                decision: None,
                description: description.clone(),
            },
        );
        Ok(Decision::Required(description))
    }
}
