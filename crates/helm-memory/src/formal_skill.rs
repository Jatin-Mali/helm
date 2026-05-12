//! Formal skill definition with skill.toml and auto-composite registration.
//!
//! A skill is either:
//! - A single `.md` file: informal, freeform markdown (backward-compatible)
//! - A `skill.toml` + `SKILL.md` bundle: formal, structured, auto-registered as composite tool
//!
//! Formal skill structure:
//!   skills/skill-id/
//!     skill.toml    — metadata, prerequisites, tool_sequence definition
//!     SKILL.md      — detailed instructions, examples, references
//!     script/       — optional helper scripts

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FormalSkillError {
    #[error("skill not found: {0}")]
    NotFound(String),
    #[error("invalid skill.toml: {0}")]
    InvalidToml(String),
    #[error("skill {0} is not formal (no skill.toml)")]
    NotFormal(String),
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormalSkill {
    pub skill: SkillMeta,
    #[serde(default)]
    pub prerequisites: Prerequisites,
    #[serde(default)]
    pub metadata: SkillMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    pub description: SkillDescription,
}

impl SkillMeta {
    pub fn resolved_version(&self) -> String {
        self.version.clone().unwrap_or_else(|| "1.0.0".to_owned())
    }

    pub fn resolved_author(&self) -> String {
        self.author.clone().unwrap_or_else(|| "unknown".to_owned())
    }

    pub fn resolved_license(&self) -> String {
        self.license
            .clone()
            .unwrap_or_else(|| "Apache-2.0".to_owned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SkillDescription {
    Short(String),
    Full { short: String, long: Option<String> },
}

impl SkillDescription {
    pub fn short(&self) -> String {
        match self {
            SkillDescription::Short(s) => s.clone(),
            SkillDescription::Full { short, .. } => short.clone(),
        }
    }

    pub fn long(&self) -> Option<String> {
        match self {
            SkillDescription::Short(_) => None,
            SkillDescription::Full { long, .. } => long.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Eq, PartialEq)]
pub struct Prerequisites {
    #[serde(default)]
    pub env_vars: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Eq, PartialEq)]
pub struct SkillMetadata {
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
}

impl FormalSkill {
    pub fn load(dir: &Path) -> Result<Self, FormalSkillError> {
        let skill_toml_path = dir.join("skill.toml");
        if !skill_toml_path.exists() {
            return Err(FormalSkillError::NotFormal(
                dir.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_owned(),
            ));
        }
        let content = fs::read_to_string(&skill_toml_path)?;
        let parsed: FormalSkill =
            toml::from_str(&content).map_err(|e| FormalSkillError::InvalidToml(e.to_string()))?;
        Ok(parsed)
    }

    pub fn is_formal(dir: &Path) -> bool {
        dir.join("skill.toml").exists()
    }

    pub fn skill_dir(id: &str) -> PathBuf {
        PathBuf::from(id)
    }

    pub fn skill_toml_path(id: &str) -> PathBuf {
        Self::skill_dir(id).join("skill.toml")
    }

    pub fn skill_md_path(id: &str) -> PathBuf {
        Self::skill_dir(id).join("SKILL.md")
    }

    pub fn tool_sequence(&self) -> Vec<String> {
        self.prerequisites.tools.clone()
    }
}

pub fn parse_skill_toml(content: &str) -> Result<FormalSkill, FormalSkillError> {
    toml::from_str(content).map_err(|e| FormalSkillError::InvalidToml(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_skill_toml() {
        let toml = r#"
[skill]
id = "test-skill"
name = "Test Skill"
description = "A test skill"

[prerequisites]
env_vars = ["API_KEY"]
"#;
        let skill = parse_skill_toml(toml).unwrap();
        assert_eq!(skill.skill.id, "test-skill");
        assert_eq!(skill.skill.name, "Test Skill");
        assert_eq!(skill.prerequisites.env_vars, vec!["API_KEY"]);
    }

    #[test]
    fn parse_full_skill_toml() {
        let toml = r#"
[skill]
id = "deploy-nginx"
name = "Deploy NGINX"
version = "1.0.0"
author = "community"
license = "MIT"

[skill.description]
short = "Deploy nginx via shell and service"
long = "Full description here"

[prerequisites]
env_vars = ["NGINX_API_KEY"]
capabilities = ["ShellExec", "SystemService"]

[metadata]
tags = ["nginx", "web", "deployment"]
homepage = "https://example.com"
"#;
        let skill = parse_skill_toml(toml).unwrap();
        assert_eq!(skill.skill.version.as_deref(), Some("1.0.0"));
        assert_eq!(skill.skill.author.as_deref(), Some("community"));
        assert_eq!(
            skill.skill.description.short(),
            "Deploy nginx via shell and service"
        );
        assert_eq!(
            skill.prerequisites.capabilities,
            vec!["ShellExec", "SystemService"]
        );
        assert_eq!(skill.metadata.tags, vec!["nginx", "web", "deployment"]);
    }
}
