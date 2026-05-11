//! Sandbox support - bwrap availability check for future bubblewrap integration.

use which::which;

pub fn is_bubblewrap_available() -> bool {
    which("bwrap").is_ok()
}