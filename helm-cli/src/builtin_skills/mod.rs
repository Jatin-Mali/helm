//! Compile-time embedded builtin skill bundle (K1).

use helm_memory::{Prerequisites, Skill};
use std::path::PathBuf;

struct BuiltinSrc {
    toml: &'static str,
    md: &'static str,
}

const BUILTINS: &[BuiltinSrc] = &[
    BuiltinSrc {
        toml: include_str!("docker-restart/skill.toml"),
        md: include_str!("docker-restart/SKILL.md"),
    },
    BuiltinSrc {
        toml: include_str!("git-status/skill.toml"),
        md: include_str!("git-status/SKILL.md"),
    },
    BuiltinSrc {
        toml: include_str!("nginx-deploy/skill.toml"),
        md: include_str!("nginx-deploy/SKILL.md"),
    },
];

pub fn load_builtin_skills() -> Vec<Skill> {
    BUILTINS
        .iter()
        .filter_map(|src| {
            let formal = helm_memory::formal_skill::parse_skill_toml(src.toml).ok()?;
            Some(Skill {
                id: formal.skill.id.clone(),
                name: formal.skill.name.clone(),
                description: formal.skill.description.short(),
                version: formal.skill.resolved_version(),
                content: src.md.to_owned(),
                approved: true,
                source_path: PathBuf::from("builtin"),
                is_formal: true,
                prerequisites: Some(Prerequisites::default()),
            })
        })
        .collect()
}

/// Extract all bash/sh fenced code blocks from skill markdown content.
pub fn extract_bash_commands(content: &str) -> Vec<String> {
    let mut cmds = Vec::new();
    let mut in_bash = false;
    let mut current = String::new();
    for line in content.lines() {
        if !in_bash {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```bash") || trimmed.starts_with("```sh") {
                in_bash = true;
                current.clear();
            }
        } else if line.trim() == "```" {
            let cmd = current.trim().to_owned();
            if !cmd.is_empty() {
                cmds.push(cmd);
            }
            in_bash = false;
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_builtin_skills_returns_three() {
        let skills = load_builtin_skills();
        assert_eq!(skills.len(), 3, "expected 3 builtin skills");
        let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"docker-restart"));
        assert!(ids.contains(&"git-status"));
        assert!(ids.contains(&"nginx-deploy"));
    }

    #[test]
    fn extract_bash_commands_finds_fenced_blocks() {
        let md = "# Test\n```bash\necho hello\n```\nsome text\n```sh\nls -la\n```";
        let cmds = extract_bash_commands(md);
        assert_eq!(cmds, vec!["echo hello", "ls -la"]);
    }

    #[test]
    fn extract_bash_commands_skips_non_bash_fences() {
        let md = "```rust\nfn main() {}\n```\n```bash\necho ok\n```";
        let cmds = extract_bash_commands(md);
        assert_eq!(cmds, vec!["echo ok"]);
    }
}
