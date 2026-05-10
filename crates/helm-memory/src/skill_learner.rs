//! Voyager-style skill learning — promotes successful procedures to named skills.

use crate::episodes::MemoryStore;
use crate::procedures::{Procedure, ProcedureError, ProcedureStep, ProcedureStore};
use crate::skills::{SkillError, SkillsManager};
use helm_core::ContentBlock;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SkillLearnerError {
    #[error("procedure: {0}")]
    Procedure(#[from] ProcedureError),
    #[error("skill: {0}")]
    Skill(#[from] SkillError),
    #[error("memory: {0}")]
    Memory(#[from] helm_core::MemoryError),
}

// ── SkillMatch ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SkillMatch {
    pub skill_id: String,
    pub skill_name: String,
    pub tool_sequence: Vec<String>,
    pub confidence: f32,
}

// ── SkillLearner ──────────────────────────────────────────────────────────────

pub struct SkillLearner {
    procedures: ProcedureStore,
    skills: SkillsManager,
    memory: Arc<MemoryStore>,
    promote_threshold: u32,
}

impl SkillLearner {
    pub fn new(
        procedures: ProcedureStore,
        skills: SkillsManager,
        memory: Arc<MemoryStore>,
    ) -> Self {
        Self {
            procedures,
            skills,
            memory,
            promote_threshold: 3,
        }
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

        Ok(LearnResult {
            procedure_id: proc_id,
            promoted_skill,
        })
    }

    /// Return procedures relevant to `goal`, ranked by success count.
    pub fn suggest(&self, goal: &str, limit: u32) -> Result<Vec<Procedure>, SkillLearnerError> {
        Ok(self.procedures.find_by_goal(goal, limit)?)
    }

    /// Extract skills from a completed episode by analyzing tool sequences.
    pub async fn extract_skills_from_episode(
        &self,
        episode_id: &str,
    ) -> Result<Vec<String>, SkillLearnerError> {
        let episode =
            self.memory.get_episode(episode_id).await?.ok_or_else(|| {
                helm_core::MemoryError::NotFound(format!("episode {}", episode_id))
            })?;
        let steps = self.memory.get_steps(episode_id).await?;

        // Extract tool names from steps by looking for ToolUse blocks
        let mut tool_sequence: Vec<String> = Vec::new();
        for step in &steps {
            for content_block in &step.content {
                if let ContentBlock::ToolUse { name, .. } = content_block {
                    tool_sequence.push(name.clone());
                }
            }
        }

        if tool_sequence.is_empty() {
            return Ok(Vec::new());
        }

        // Compute skill key from tool sequence
        let _skill_key = compute_skill_key(&tool_sequence);
        let skill_id = format!(
            "skill_{}",
            Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("unknown")
        );

        // Store the skill
        let _skill_name = format!(
            "skill_{}",
            tool_sequence.first().unwrap_or(&"unknown".to_string())
        );
        let content = render_skill_markdown(&episode.goal, &tool_sequence);

        let dir = self.skills.skills_dir();
        std::fs::create_dir_all(dir).map_err(SkillError::Io)?;
        std::fs::write(dir.join(format!("{skill_id}.md")), content).map_err(SkillError::Io)?;

        Ok(vec![skill_id])
    }

    /// Find skills that match a given goal by keyword matching.
    pub async fn find_matching_skills(
        &self,
        _goal: &str,
        min_confidence: f32,
    ) -> Result<Vec<SkillMatch>, SkillLearnerError> {
        let skills = self.skills.list()?;
        let mut matches: Vec<SkillMatch> = skills
            .iter()
            .filter_map(|skill| {
                let confidence = 0.7; // Fixed confidence for now
                if confidence >= min_confidence {
                    Some(SkillMatch {
                        skill_id: skill.id.clone(),
                        skill_name: skill.name.clone(),
                        tool_sequence: Vec::new(),
                        confidence,
                    })
                } else {
                    None
                }
            })
            .collect();

        matches.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(matches.into_iter().take(5).collect())
    }

    fn promote(&self, proc: &Procedure) -> Result<String, SkillLearnerError> {
        let name = slugify(&proc.goal_pattern);
        let skill_id = format!("learned-{name}");
        let tool_names: Vec<String> = proc.steps.iter().map(|step| step.tool.clone()).collect();
        let content = render_skill_markdown(&proc.goal_pattern, &tool_names);
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
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn compute_skill_key(tool_sequence: &[String]) -> String {
    let joined = tool_sequence.join("→");
    let mut hasher = Sha256::new();
    hasher.update(&joined);
    format!("{:x}", hasher.finalize())
        .chars()
        .take(16)
        .collect()
}

fn render_skill_markdown(goal: &str, tool_names: &[String]) -> String {
    let mut md = format!(
        "# {goal}\n\n**Description:** Learned from successful completions of: {goal}\n**Version:** 1.0\n\n## Tool Sequence\n\n"
    );
    for (i, tool) in tool_names.iter().enumerate() {
        md.push_str(&format!("{}. `{}`\n", i + 1, tool));
    }
    md
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use super::{SkillLearner, slugify};
    use crate::episodes::MemoryStore;
    use crate::procedures::{ProcedureStep, ProcedureStore};
    use crate::skills::SkillsManager;
    use std::sync::Arc;

    fn steps(tools: &[&str]) -> Vec<ProcedureStep> {
        tools
            .iter()
            .map(|t| ProcedureStep {
                tool: t.to_string(),
                input: json!({}),
            })
            .collect()
    }

    fn learner(threshold: u32) -> (SkillLearner, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let procs = ProcedureStore::open_in_memory().unwrap();
        let skills = SkillsManager::with_dir(dir.path().to_path_buf());
        let memory_path = dir.path().join("memory.db");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let memory = Arc::new(rt.block_on(MemoryStore::open(&memory_path)).unwrap());
        (
            SkillLearner::new(procs, skills, memory).with_promote_threshold(threshold),
            dir,
        )
    }

    #[test]
    fn learn_creates_procedure_happy_path() {
        let (l, _d) = learner(5);
        let r = l
            .learn("deploy nginx", &steps(&["shell", "service"]))
            .unwrap();
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
        assert_eq!(
            slugify("Deploy NGINX on Staging!"),
            "deploy-nginx-on-staging"
        );
        assert_eq!(slugify("  run tests  "), "run-tests");
    }
}
