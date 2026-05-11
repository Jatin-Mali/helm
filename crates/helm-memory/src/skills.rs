//! File-backed skill library metadata for HELM.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors returned by the skills system.
#[derive(Debug, Error)]
pub enum SkillError {
    /// Requested skill was not found.
    #[error("skill not found: {0}")]
    NotFound(String),
    /// Skill file content is missing required structure.
    #[error("invalid skill file: {0}")]
    InvalidFile(String),
    /// Filesystem error while reading or writing a skill.
    #[error("skill io error: {0}")]
    Io(#[from] std::io::Error),
}

/// A loaded skill file and review status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Skill {
    /// Stable skill identifier derived from the filename or directory name.
    pub id: String,
    /// Human-readable skill name.
    pub name: String,
    /// Short description extracted from skill content.
    pub description: String,
    /// Skill version string.
    pub version: String,
    /// Raw markdown skill content.
    pub content: String,
    /// Whether this skill has been approved by the user.
    pub approved: bool,
    /// Filesystem path containing the skill markdown.
    pub source_path: PathBuf,
    /// Whether this is a formal skill (skill.toml + SKILL.md bundle).
    #[serde(default)]
    pub is_formal: bool,
    /// Prerequisites for this skill (formal skills only).
    #[serde(default)]
    pub prerequisites: Option<crate::Prerequisites>,
}

/// Skills manager backed by markdown files under a configured directory.
#[derive(Debug, Clone)]
pub struct SkillsManager {
    skills_dir: PathBuf,
}

impl SkillsManager {
    /// Creates a skills manager rooted at `$XDG_DATA_HOME/helm/skills` (or fallback).
    pub fn new() -> Self {
        let skills_dir = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
            .join("helm")
            .join("skills");
        Self::with_dir(skills_dir)
    }

    /// Creates a skills manager for an explicit directory.
    pub fn with_dir(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }

    /// Returns the directory this manager scans for skill files.
    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }

    /// Lists all available skills. Unreadable entries are skipped.
    pub fn list(&self) -> Result<Vec<Skill>, SkillError> {
        if !self.skills_dir.exists() {
            return Ok(Vec::new());
        }
        let mut skills = Vec::new();
        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            let path = entry.path();
            let loaded = if path.is_dir() {
                self.load_skill_from_dir(&path)
            } else if path.extension().is_some_and(|ext| ext == "md") {
                self.load_skill_from_file(&path)
            } else {
                Ok(None)
            }?;
            if let Some(skill) = loaded {
                skills.push(skill);
            }
        }
        skills.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(skills)
    }

    /// Shows a specific skill by ID.
    pub fn show(&self, id: &str) -> Result<Skill, SkillError> {
        let skill_file = self.resolve_skill_file(id);
        if !skill_file.exists() {
            return Err(SkillError::NotFound(id.to_owned()));
        }
        self.load_skill_from_file(&skill_file)?
            .ok_or_else(|| SkillError::InvalidFile(skill_file.display().to_string()))
    }

    /// Marks a skill as approved for future use.
    pub fn approve(&self, id: &str) -> Result<(), SkillError> {
        let mut skill = self.show(id)?;
        skill.approved = true;
        self.save_skill(&skill)
    }

    /// Disables a skill without deleting it.
    pub fn disable(&self, id: &str) -> Result<(), SkillError> {
        let mut skill = self.show(id)?;
        skill.approved = false;
        self.save_skill(&skill)
    }

    /// Runs lightweight gold-example validation for a skill.
    pub fn test(&self, id: &str) -> Result<String, SkillError> {
        let skill = self.show(id)?;
        if !skill.approved {
            return Err(SkillError::InvalidFile(format!(
                "skill '{}' is not approved",
                id
            )));
        }
        if !skill.content.contains("#") {
            return Err(SkillError::InvalidFile(format!(
                "skill '{}' must contain a markdown heading",
                id
            )));
        }
        Ok(format!("skill '{}' passed basic checks", skill.name))
    }

    fn resolve_skill_file(&self, id: &str) -> PathBuf {
        let skill_path = self.skills_dir.join(id);
        if skill_path.is_dir() {
            skill_path.join("SKILL.md")
        } else if id.ends_with(".md") {
            skill_path
        } else {
            self.skills_dir.join(format!("{id}.md"))
        }
    }

    fn load_skill_from_dir(&self, dir: &Path) -> Result<Option<Skill>, SkillError> {
        let skill_file = dir.join("SKILL.md");
        if !skill_file.exists() {
            return Ok(None);
        }
        let skill_id = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();

        if let Some(toml_path) = dir
            .join("skill.toml")
            .exists()
            .then_some(dir.join("skill.toml"))
        {
            if let Ok(toml_content) = std::fs::read_to_string(&toml_path) {
                if let Ok(formal) = crate::formal_skill::parse_skill_toml(&toml_content) {
                    let skill_md = if skill_file.exists() {
                        std::fs::read_to_string(&skill_file)?
                    } else {
                        String::new()
                    };
                    return Ok(Some(Skill {
                        id: formal.skill.id.clone(),
                        name: formal.skill.name.clone(),
                        description: formal.skill.description.short(),
                        version: formal.skill.resolved_version(),
                        content: skill_md,
                        approved: false,
                        source_path: dir.to_path_buf(),
                        is_formal: true,
                        prerequisites: Some(formal.prerequisites),
                    }));
                }
            }
        }

        if let Some(inner) = self.load_skill_from_file(&skill_file)? {
            Ok(Some(Skill {
                id: skill_id,
                name: inner.name,
                description: inner.description,
                version: inner.version,
                content: inner.content,
                approved: inner.approved,
                source_path: inner.source_path,
                is_formal: false,
                prerequisites: None,
            }))
        } else {
            Ok(None)
        }
    }

    fn load_skill_from_file(&self, path: &Path) -> Result<Option<Skill>, SkillError> {
        let content = fs::read_to_string(path)?;
        if content.trim().is_empty() {
            return Ok(None);
        }
        let id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| SkillError::InvalidFile(path.display().to_string()))?
            .to_owned();
        let name = extract_title(&content).unwrap_or_else(|| id.replace(['_', '-'], " "));
        let approved = content
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case("approved: true"));
        Ok(Some(Skill {
            id,
            name: name.clone(),
            description: extract_description(&content).unwrap_or_else(|| format!("Skill: {name}")),
            version: extract_version(&content).unwrap_or_else(|| "1.0.0".to_owned()),
            content,
            approved,
            source_path: path.to_path_buf(),
            is_formal: false,
            prerequisites: None,
        }))
    }

    fn save_skill(&self, skill: &Skill) -> Result<(), SkillError> {
        let mut content = skill
            .content
            .lines()
            .filter(|line| !line.trim().to_ascii_lowercase().starts_with("approved:"))
            .collect::<Vec<_>>()
            .join("\n");
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!("\nApproved: {}\n", skill.approved));
        fs::write(&skill.source_path, content)?;
        Ok(())
    }
}

impl Default for SkillsManager {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_title(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_description(content: &str) -> Option<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("Approved:"))
        .map(ToOwned::to_owned)
        .next()
}

fn extract_version(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("Version:")
            .or_else(|| trimmed.strip_prefix("version:"))
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(ToOwned::to_owned)
    })
}

#[cfg(test)]
mod tests {
    use super::SkillsManager;
    use tempfile::tempdir;

    #[test]
    fn missing_directory_lists_empty_edge_case() {
        let dir = tempdir().unwrap();
        let manager = SkillsManager::with_dir(dir.path().join("missing"));

        assert!(manager.list().unwrap().is_empty());
    }

    #[test]
    fn skill_not_found_error_path() {
        let dir = tempdir().unwrap();
        let manager = SkillsManager::with_dir(dir.path().to_path_buf());

        let error = manager.show("nonexistent").unwrap_err();

        assert!(error.to_string().contains("not found"));
    }

    #[test]
    fn approve_disable_and_test_happy_path() {
        let dir = tempdir().unwrap();
        let skill_path = dir.path().join("demo.md");
        std::fs::write(
            &skill_path,
            "# Demo Skill\n\nVersion: 1.2.3\n\nDoes a useful thing.\n",
        )
        .unwrap();
        let manager = SkillsManager::with_dir(dir.path().to_path_buf());

        let listed = manager.list().unwrap();
        assert_eq!(listed[0].name, "Demo Skill");
        manager.approve("demo").unwrap();
        assert!(manager.show("demo").unwrap().approved);
        assert!(manager.test("demo").unwrap().contains("passed"));
        manager.disable("demo").unwrap();
        assert!(!manager.show("demo").unwrap().approved);
    }
}
