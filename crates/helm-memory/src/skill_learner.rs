//! Voyager-style skill learning — promotes successful procedures to named skills.

use crate::procedures::{Procedure, ProcedureError, ProcedureStep, ProcedureStore};
use crate::skills::{SkillError, SkillsManager};

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SkillLearnerError {
    #[error("procedure: {0}")]
    Procedure(#[from] ProcedureError),
    #[error("skill: {0}")]
    Skill(#[from] SkillError),
}

// ── SkillLearner ──────────────────────────────────────────────────────────────

pub struct SkillLearner {
    procedures: ProcedureStore,
    skills: SkillsManager,
    promote_threshold: u32,
}

impl SkillLearner {
    pub fn new(procedures: ProcedureStore, skills: SkillsManager) -> Self {
        Self { procedures, skills, promote_threshold: 3 }
    }

    pub fn with_promote_threshold(mut self, n: u32) -> Self {
        self.promote_threshold = n;
        self
    }

    /// Record a successful episode and learn from it.
    /// Finds or creates a matching procedure, increments success, and promotes
    /// to a named skill file once `promote_threshold` is reached.
    pub fn learn(
        &self,
        goal: &str,
        steps: &[ProcedureStep],
    ) -> Result<LearnResult, SkillLearnerError> {
        let existing = self.procedures.find_by_goal(goal, 1)?;
        let proc_id = if let Some(proc) = existing.into_iter().next() {
            self.procedures.record_success(&proc.id)?;
            proc.id
        } else {
            let id = self.procedures.insert(goal, steps)?;
            self.procedures.record_success(&id)?;
            id
        };

        let proc = self.procedures.get(&proc_id)?.unwrap();
        let promoted_skill = if proc.success_count >= self.promote_threshold {
            Some(self.promote(&proc)?)
        } else {
            None
        };

        Ok(LearnResult { procedure_id: proc_id, promoted_skill })
    }

    /// Return procedures relevant to `goal`, ranked by success count.
    pub fn suggest(&self, goal: &str, limit: u32) -> Result<Vec<Procedure>, SkillLearnerError> {
        Ok(self.procedures.find_by_goal(goal, limit)?)
    }

    fn promote(&self, proc: &Procedure) -> Result<String, SkillLearnerError> {
        let name = slugify(&proc.goal_pattern);
        let skill_id = format!("learned-{name}");
        let content = render_skill_markdown(&proc.goal_pattern, &proc.steps);
        let dir = self.skills.skills_dir();
        std::fs::create_dir_all(dir).map_err(SkillError::Io)?;
        std::fs::write(dir.join(format!("{skill_id}.md")), content).map_err(SkillError::Io)?;
        Ok(skill_id)
    }
}

// ── LearnResult ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct LearnResult {
    pub procedure_id: String,
    /// Set when the procedure's success count crossed `promote_threshold`.
    pub promoted_skill: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn render_skill_markdown(goal: &str, steps: &[ProcedureStep]) -> String {
    let mut md = format!(
        "# {goal}\n\n**Description:** Learned from successful completions of: {goal}\n**Version:** 1.0\n\n## Steps\n\n"
    );
    for (i, step) in steps.iter().enumerate() {
        md.push_str(&format!("{}. `{}` — `{}`\n", i + 1, step.tool, step.input));
    }
    md
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use super::{SkillLearner, slugify};
    use crate::procedures::{ProcedureStep, ProcedureStore};
    use crate::skills::SkillsManager;

    fn steps(tools: &[&str]) -> Vec<ProcedureStep> {
        tools.iter().map(|t| ProcedureStep { tool: t.to_string(), input: json!({}) }).collect()
    }

    fn learner(threshold: u32) -> (SkillLearner, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let procs = ProcedureStore::open_in_memory().unwrap();
        let skills = SkillsManager::with_dir(dir.path().to_path_buf());
        (SkillLearner::new(procs, skills).with_promote_threshold(threshold), dir)
    }

    #[test]
    fn learn_creates_procedure_happy_path() {
        let (l, _d) = learner(5);
        let r = l.learn("deploy nginx", &steps(&["shell", "service"])).unwrap();
        assert!(!r.procedure_id.is_empty());
        assert!(r.promoted_skill.is_none());
    }

    #[test]
    fn learn_increments_existing_procedure_happy_path() {
        let (l, _d) = learner(5);
        l.learn("run migrations", &steps(&["shell"])).unwrap();
        l.learn("run migrations", &steps(&["shell"])).unwrap();
        let suggestions = l.suggest("migrations", 10).unwrap();
        assert_eq!(suggestions[0].success_count, 2);
    }

    #[test]
    fn learn_promotes_at_threshold_happy_path() {
        let (l, d) = learner(2);
        l.learn("restart api", &steps(&["shell"])).unwrap();
        let r = l.learn("restart api", &steps(&["shell"])).unwrap();
        assert!(r.promoted_skill.is_some());
        let skill_id = r.promoted_skill.unwrap();
        assert!(d.path().join(format!("{skill_id}.md")).exists());
    }

    #[test]
    fn suggest_returns_relevant_procedures_happy_path() {
        let (l, _d) = learner(5);
        l.learn("deploy nginx", &steps(&["shell"])).unwrap();
        l.learn("deploy redis", &steps(&["shell"])).unwrap();
        l.learn("check disk", &steps(&["disk"])).unwrap();
        let s = l.suggest("deploy", 10).unwrap();
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn slugify_normalizes_correctly_edge_case() {
        assert_eq!(slugify("Deploy NGINX on Staging!"), "deploy-nginx-on-staging");
        assert_eq!(slugify("  run tests  "), "run-tests");
    }
}
