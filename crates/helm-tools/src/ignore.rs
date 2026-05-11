//! `.helmignore` support (H7) — gitignore-style filter that restricts which
//! paths the agent's fs_read / fs_write / shell tools may touch.
//!
//! Patterns follow a simplified gitignore syntax:
//!   - Lines starting with `#` are comments.
//!   - Blank lines are ignored.
//!   - A leading `/` anchors the pattern to the workspace root.
//!   - A trailing `/` matches directories only.
//!   - `*` matches any run of characters within a single path segment.
//!   - `**` matches any number of segments.
//!   - Lines starting with `!` whitelist a previously-ignored path.
//!
//! Matching is greedy — the last matching pattern wins.

use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default)]
pub struct HelmIgnore {
    rules: Vec<Rule>,
    root: PathBuf,
}

#[derive(Debug, Clone)]
struct Rule {
    pattern: String,
    negate: bool,
    dir_only: bool,
    anchored: bool,
}

impl HelmIgnore {
    pub fn empty(root: PathBuf) -> Self {
        Self {
            rules: Vec::new(),
            root,
        }
    }

    /// Load from a workspace root. Looks for `<root>/.helmignore`.
    pub fn load(root: &Path) -> Self {
        let path = root.join(".helmignore");
        match fs::read_to_string(&path) {
            Ok(text) => Self::from_text(&text, root.to_path_buf()),
            Err(_) => Self::empty(root.to_path_buf()),
        }
    }

    pub fn from_text(text: &str, root: PathBuf) -> Self {
        let mut rules = Vec::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negate, body) = match line.strip_prefix('!') {
                Some(rest) => (true, rest.trim_start()),
                None => (false, line),
            };
            let dir_only = body.ends_with('/');
            let body = body.trim_end_matches('/');
            let anchored = body.starts_with('/');
            let pattern = body.trim_start_matches('/').to_owned();
            rules.push(Rule {
                pattern,
                negate,
                dir_only,
                anchored,
            });
        }
        Self { rules, root }
    }

    /// Returns true if the given path is ignored (i.e. tool access should be
    /// denied). Accepts absolute or relative paths; both are normalized
    /// against the workspace root.
    pub fn is_ignored<P: AsRef<Path>>(&self, path: P) -> bool {
        let p = path.as_ref();
        let rel = match p.strip_prefix(&self.root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => p.to_path_buf(),
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let mut state = false;
        for rule in &self.rules {
            if rule.dir_only && !rel.is_dir() && !rel_str.ends_with('/') {
                let segments: Vec<&str> = rel_str.split('/').collect();
                let mut any_match = false;
                for end in 1..segments.len() {
                    let sub = segments[..end].join("/");
                    if match_pattern(&rule.pattern, &sub, rule.anchored) {
                        any_match = true;
                        break;
                    }
                }
                if !any_match {
                    continue;
                }
            } else if !match_pattern(&rule.pattern, &rel_str, rule.anchored) {
                continue;
            }
            state = !rule.negate;
        }
        state
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

fn match_pattern(pattern: &str, path: &str, anchored: bool) -> bool {
    if anchored {
        glob_match(pattern, path)
    } else {
        if glob_match(pattern, path) {
            return true;
        }
        for (idx, _) in path.match_indices('/') {
            if glob_match(pattern, &path[idx + 1..]) {
                return true;
            }
        }
        false
    }
}

/// Tiny glob matcher supporting `*` (one segment) and `**` (any segments).
fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_inner(p: &[u8], t: &[u8]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    if p.starts_with(b"**/") {
        let rest = &p[3..];
        if glob_match_inner(rest, t) {
            return true;
        }
        for (idx, _) in t.iter().enumerate() {
            if t[idx] == b'/' && glob_match_inner(rest, &t[idx + 1..]) {
                return true;
            }
        }
        return false;
    }
    if p == b"**" {
        return true;
    }
    if p[0] == b'*' {
        let rest = &p[1..];
        for i in 0..=t.len() {
            if t[..i].contains(&b'/') {
                return false;
            }
            if glob_match_inner(rest, &t[i..]) {
                return true;
            }
        }
        return false;
    }
    if t.is_empty() {
        return false;
    }
    if p[0] == b'?' || p[0] == t[0] {
        return glob_match_inner(&p[1..], &t[1..]);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_literal() {
        let ig = HelmIgnore::from_text("secrets.txt\n", PathBuf::from("/repo"));
        assert!(ig.is_ignored("/repo/secrets.txt"));
        assert!(ig.is_ignored("/repo/dir/secrets.txt"));
        assert!(!ig.is_ignored("/repo/other.txt"));
    }

    #[test]
    fn matches_anchored() {
        let ig = HelmIgnore::from_text("/build\n", PathBuf::from("/repo"));
        assert!(ig.is_ignored("/repo/build"));
        assert!(!ig.is_ignored("/repo/src/build"));
    }

    #[test]
    fn matches_glob_star() {
        let ig = HelmIgnore::from_text("*.env\n", PathBuf::from("/repo"));
        assert!(ig.is_ignored("/repo/.env"));
        assert!(ig.is_ignored("/repo/local.env"));
        assert!(ig.is_ignored("/repo/sub/.env"));
    }

    #[test]
    fn matches_double_star() {
        let ig = HelmIgnore::from_text("node_modules/**\n", PathBuf::from("/repo"));
        assert!(ig.is_ignored("/repo/node_modules/foo"));
        assert!(ig.is_ignored("/repo/node_modules/foo/bar.js"));
    }

    #[test]
    fn negation_overrides_earlier_match() {
        let ig = HelmIgnore::from_text("*.log\n!keep.log\n", PathBuf::from("/repo"));
        assert!(ig.is_ignored("/repo/x.log"));
        assert!(!ig.is_ignored("/repo/keep.log"));
    }

    #[test]
    fn comments_and_blanks_skipped() {
        let ig = HelmIgnore::from_text("# comment\n\nsecret\n", PathBuf::from("/repo"));
        assert_eq!(ig.rule_count(), 1);
        assert!(ig.is_ignored("/repo/secret"));
    }
}
