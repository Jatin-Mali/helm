//! Bubblewrap-backed sandbox resolution for `--sandbox`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use which::which;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSandbox {
    pub root_dir: PathBuf,
    pub bwrap_program: PathBuf,
}

pub fn resolve(enabled: bool, sandbox_dir: Option<&PathBuf>) -> Result<Option<ResolvedSandbox>> {
    if !enabled {
        return Ok(None);
    }

    let bwrap_program = which("bwrap").map_err(|error| {
        anyhow!(
            "--sandbox requires bubblewrap (`bwrap`) to be installed and available on PATH: {error}"
        )
    })?;

    let root_dir = match sandbox_dir {
        Some(path) => {
            std::fs::create_dir_all(path)
                .with_context(|| format!("creating sandbox dir {}", path.display()))?;
            path.clone()
        }
        None => std::env::current_dir().context("resolving current working directory")?,
    };
    let root_dir = root_dir
        .canonicalize()
        .with_context(|| format!("resolving sandbox root {}", root_dir.display()))?;

    if !root_dir.is_dir() {
        return Err(anyhow!(
            "sandbox root is not a directory: {}",
            root_dir.display()
        ));
    }

    Ok(Some(ResolvedSandbox {
        root_dir,
        bwrap_program,
    }))
}

impl ResolvedSandbox {
    pub fn display_root(&self) -> &Path {
        &self.root_dir
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::resolve;

    #[test]
    fn disabled_sandbox_returns_none() {
        assert!(resolve(false, None).unwrap().is_none());
    }

    #[test]
    fn enabled_sandbox_uses_explicit_root() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sandbox");
        let resolved = match resolve(true, Some(&root)) {
            Ok(Some(policy)) => policy,
            Ok(None) => panic!("sandbox unexpectedly disabled"),
            Err(error) if error.to_string().contains("bubblewrap") => {
                eprintln!("skipping sandbox resolve test: {error}");
                return;
            }
            Err(error) => panic!("sandbox resolution failed: {error}"),
        };

        assert_eq!(resolved.root_dir, root.canonicalize().unwrap());
    }
}
