//! Custom user slash commands at `~/.helm/commands/<name>.md` (P4).
//!
//! Each file has optional YAML frontmatter and a markdown body that becomes
//! the task template. The first line of frontmatter `description:` is shown
//! in the slash picker.
//!
//! Example `~/.helm/commands/triage.md`:
//! ```markdown
//! ---
//! description: Quick incident triage helper
//! ---
//! Investigate any active alerts on this host. Pull recent journalctl
//! entries with `journalctl -p err -b | tail -100` and summarize the
//! top three concerns.
//! ```

#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomCommand {
    pub name: String,
    pub description: String,
    pub body: String,
}

pub fn commands_dir() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?;
    Ok(home.join(".helm").join("commands"))
}

pub fn load_all() -> Vec<CustomCommand> {
    let Ok(dir) = commands_dir() else {
        return Vec::new();
    };
    load_from(&dir)
}

pub fn load_from(dir: &Path) -> Vec<CustomCommand> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        if let Some(cmd) = load_one(&path) {
            out.push(cmd);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn load_one(path: &Path) -> Option<CustomCommand> {
    let name = path.file_stem()?.to_str()?.to_owned();
    let text = fs::read_to_string(path).ok()?;
    let (description, body) = split_frontmatter(&text);
    Some(CustomCommand {
        name,
        description: description.unwrap_or_else(|| "(custom command)".to_owned()),
        body: body.trim().to_owned(),
    })
}

fn split_frontmatter(text: &str) -> (Option<String>, String) {
    let trimmed = text.trim_start_matches('\u{feff}');
    if let Some(rest) = trimmed.strip_prefix("---\n")
        && let Some(end) = rest.find("\n---")
    {
        let header = &rest[..end];
        let body_start = end + "\n---".len();
        let body = &rest[body_start..];
        let body = body.trim_start_matches('\n').to_owned();
        let mut description: Option<String> = None;
        for line in header.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("description:") {
                description = Some(value.trim().trim_matches('"').trim_matches('\'').to_owned());
            }
        }
        return (description, body);
    }
    (None, text.to_owned())
}

/// Substitute `{{args}}` placeholder in the body with the provided argument
/// string. If no placeholder is present, append the args (if any) at the end.
pub fn expand(cmd: &CustomCommand, args: &str) -> String {
    let args = args.trim();
    if cmd.body.contains("{{args}}") {
        return cmd.body.replace("{{args}}", args);
    }
    if args.is_empty() {
        cmd.body.clone()
    } else {
        format!("{}\n\n{args}", cmd.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn split_frontmatter_extracts_description() {
        let text = "---\ndescription: A test command\n---\nDo something useful";
        let (desc, body) = split_frontmatter(text);
        assert_eq!(desc.as_deref(), Some("A test command"));
        assert_eq!(body.trim(), "Do something useful");
    }

    #[test]
    fn split_frontmatter_handles_no_frontmatter() {
        let (desc, body) = split_frontmatter("Just a body, no frontmatter");
        assert!(desc.is_none());
        assert_eq!(body, "Just a body, no frontmatter");
    }

    #[test]
    fn expand_substitutes_placeholder() {
        let cmd = CustomCommand {
            name: "ping".into(),
            description: "ping host".into(),
            body: "Ping host {{args}} now.".into(),
        };
        assert_eq!(expand(&cmd, "prod-1"), "Ping host prod-1 now.");
    }

    #[test]
    fn expand_appends_when_no_placeholder() {
        let cmd = CustomCommand {
            name: "x".into(),
            description: "".into(),
            body: "Base task.".into(),
        };
        assert_eq!(expand(&cmd, "extra"), "Base task.\n\nextra");
        assert_eq!(expand(&cmd, ""), "Base task.");
    }

    #[test]
    fn load_from_reads_md_files() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("hello.md"),
            "---\ndescription: hi\n---\nSay hello to {{args}}.",
        )
        .unwrap();
        fs::write(dir.path().join("ignore.txt"), "not a command").unwrap();
        let cmds = load_from(dir.path());
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "hello");
        assert_eq!(cmds[0].description, "hi");
    }
}
