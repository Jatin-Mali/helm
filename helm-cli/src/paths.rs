//! XDG Base Directory Specification compliance for HELM.
//!
//! Resolves all HELM paths according to XDG spec:
//! - `XDG_CONFIG_HOME` (or `~/.config`) for config files
//! - `XDG_DATA_HOME` (or `~/.local/share`) for data files
//! - `XDG_CACHE_HOME` (or `~/.cache`) for cache files
//!
//! Falls back gracefully when XDG vars are unset.

use std::path::PathBuf;

const HELM_DIR: &str = "helm";

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(HELM_DIR)
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(HELM_DIR)
}

#[allow(dead_code)]
pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(HELM_DIR)
}

pub fn config_file(name: &str) -> PathBuf {
    config_dir().join(name)
}

pub fn data_file(name: &str) -> PathBuf {
    data_dir().join(name)
}

#[allow(dead_code)]
pub fn cache_file(name: &str) -> PathBuf {
    cache_dir().join(name)
}

#[allow(dead_code)]
pub fn ensure_dir(path: &PathBuf) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

pub fn default_config_path() -> PathBuf {
    config_file("config.toml")
}

pub fn default_db_path() -> PathBuf {
    data_file("helm.db")
}

pub fn default_graph_path() -> PathBuf {
    data_file("graph.db")
}

pub fn default_snapshots_path() -> PathBuf {
    data_dir().join("snapshots")
}

pub fn default_log_path() -> PathBuf {
    data_dir().join("logs").join("helm.log")
}

#[allow(dead_code)]
pub fn default_secrets_path() -> PathBuf {
    config_file("secrets.toml")
}

#[allow(dead_code)]
pub fn default_skills_path() -> PathBuf {
    data_dir().join("skills")
}

#[allow(dead_code)]
pub fn default_hooks_path() -> PathBuf {
    config_file("hooks.toml")
}

#[allow(dead_code)]
pub fn default_keybindings_path() -> PathBuf {
    config_file("keybindings.json")
}

#[allow(dead_code)]
pub fn default_remotes_path() -> PathBuf {
    config_file("remotes.toml")
}

#[allow(dead_code)]
pub fn default_allowlist_path() -> PathBuf {
    config_file("allowlist.toml")
}

#[allow(dead_code)]
pub fn default_mcp_servers_path() -> PathBuf {
    config_file("mcp-servers.toml")
}
