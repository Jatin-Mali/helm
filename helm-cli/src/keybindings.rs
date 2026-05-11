//! User-remappable TUI keybindings (F24) loaded from
//! `~/.helm/keybindings.json`. Format mirrors Claude Code's: a JSON map of
//! action name -> key combo string.
//!
//! Example:
//! ```json
//! {
//!   "send": "Enter",
//!   "newline": "Shift+Enter",
//!   "quit": "Ctrl+D",
//!   "palette": "Ctrl+P",
//!   "tool_sidebar": "Ctrl+T",
//!   "history": "Ctrl+H",
//!   "clear": "Ctrl+L",
//!   "cancel": "Ctrl+C",
//!   "help": "?",
//!   "external_editor": "Ctrl+G",
//!   "verbose_toggle": "Ctrl+O"
//! }
//! ```

#![allow(dead_code)]

use std::{collections::HashMap, fs, path::PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use serde::Deserialize;

fn xdg_config_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("helm")
}

pub fn config_path() -> Result<PathBuf> {
    Ok(xdg_config_dir().join("keybindings.json"))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Bind {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

#[derive(Debug, Clone)]
pub struct KeyMap {
    pub map: HashMap<String, Bind>,
}

impl KeyMap {
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn load() -> Self {
        let Ok(path) = config_path() else {
            return Self::empty();
        };
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::empty();
        };
        let parsed: HashMap<String, String> = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(error) => {
                tracing::warn!(target: "helm::keybinds", "malformed keybindings: {error}");
                return Self::empty();
            }
        };
        let mut map = HashMap::new();
        for (action, combo) in parsed {
            if let Some(bind) = parse_combo(&combo) {
                map.insert(action, bind);
            } else {
                tracing::warn!(target: "helm::keybinds", "ignoring unknown combo `{combo}` for {action}");
            }
        }
        Self { map }
    }

    pub fn get(&self, action: &str) -> Option<&Bind> {
        self.map.get(action)
    }

    pub fn matches(&self, action: &str, code: KeyCode, mods: KeyModifiers) -> bool {
        self.map
            .get(action)
            .map(|b| b.code == code && b.mods == mods)
            .unwrap_or(false)
    }
}

pub fn parse_combo(s: &str) -> Option<Bind> {
    let mut mods = KeyModifiers::NONE;
    let parts: Vec<&str> = s.split('+').collect();
    let key = parts.last()?;
    for part in &parts[..parts.len().saturating_sub(1)] {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "shift" => mods |= KeyModifiers::SHIFT,
            "alt" | "meta" => mods |= KeyModifiers::ALT,
            _ => return None,
        }
    }
    let code = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "space" => KeyCode::Char(' '),
        "?" => KeyCode::Char('?'),
        other if other.len() == 1 => KeyCode::Char(other.chars().next().unwrap()),
        other if other.starts_with('f') && other.len() <= 3 => {
            let n: u8 = other[1..].parse().ok()?;
            KeyCode::F(n)
        }
        _ => return None,
    };
    Some(Bind { code, mods })
}

#[derive(Debug, Default, Deserialize)]
struct _Raw {
    #[serde(flatten)]
    _map: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_keys() {
        assert_eq!(
            parse_combo("Enter").unwrap(),
            Bind {
                code: KeyCode::Enter,
                mods: KeyModifiers::NONE
            }
        );
        assert_eq!(
            parse_combo("Ctrl+T").unwrap(),
            Bind {
                code: KeyCode::Char('t'),
                mods: KeyModifiers::CONTROL
            }
        );
        assert_eq!(
            parse_combo("Shift+Enter").unwrap(),
            Bind {
                code: KeyCode::Enter,
                mods: KeyModifiers::SHIFT
            }
        );
        assert_eq!(
            parse_combo("F5").unwrap(),
            Bind {
                code: KeyCode::F(5),
                mods: KeyModifiers::NONE
            }
        );
    }

    #[test]
    fn parse_rejects_unknown() {
        assert!(parse_combo("Hyper+Q").is_none());
        assert!(parse_combo("notakey").is_none());
    }

    #[test]
    fn load_from_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("kb.json");
        fs::write(&p, r#"{"quit":"Ctrl+D","palette":"Ctrl+P","help":"?"}"#).unwrap();
        let km = KeyMap::load_from(&p);
        assert!(km.matches("quit", KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(km.matches("palette", KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(km.matches("help", KeyCode::Char('?'), KeyModifiers::NONE));
    }
}
