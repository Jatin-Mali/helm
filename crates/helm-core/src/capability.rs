//! Capability names, grant scopes, and permission decisions shared by HELM.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// Machine-control capability required by a HELM tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// Read files or directories.
    FsRead,
    /// Create, overwrite, or append files.
    FsWrite,
    /// Delete files or directories.
    FsDelete,
    /// Execute one binary with literal arguments, without shell expansion.
    ShellExec,
    /// Execute a command through a shell with expansion, pipes, and redirection.
    ShellShell,
    /// Inspect or modify system services.
    SystemService,
    /// Install, remove, or update packages.
    PkgInstall,
    /// Control a browser instance.
    BrowserControl,
    /// Make outbound network requests.
    NetworkOut,
    /// Run privileged operations through sudo.
    Sudo,
}

impl Capability {
    /// Returns the stable dotted capability string used in CLI and audit rows.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FsRead => "fs.read",
            Self::FsWrite => "fs.write",
            Self::FsDelete => "fs.delete",
            Self::ShellExec => "shell.exec",
            Self::ShellShell => "shell.shell",
            Self::SystemService => "system.service",
            Self::PkgInstall => "pkg.install",
            Self::BrowserControl => "browser.control",
            Self::NetworkOut => "network.out",
            Self::Sudo => "sudo",
        }
    }

    /// Returns whether this capability requires an explicit grant by default.
    pub fn requires_grant_by_default(self) -> bool {
        matches!(
            self,
            Self::FsDelete
                | Self::ShellExec
                | Self::ShellShell
                | Self::SystemService
                | Self::PkgInstall
                | Self::BrowserControl
                | Self::Sudo
        )
    }

    /// Returns whether external-tainted context needs a fresh grant.
    pub fn requires_fresh_grant_for_external_taint(self) -> bool {
        matches!(
            self,
            Self::ShellExec
                | Self::ShellShell
                | Self::Sudo
                | Self::FsDelete
                | Self::PkgInstall
                | Self::SystemService
        )
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Capability {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "fs.read" => Ok(Self::FsRead),
            "fs.write" => Ok(Self::FsWrite),
            "fs.delete" => Ok(Self::FsDelete),
            "shell.exec" => Ok(Self::ShellExec),
            "shell.run" | "shell.shell" => Ok(Self::ShellShell),
            "system.service" => Ok(Self::SystemService),
            "pkg.install" => Ok(Self::PkgInstall),
            "browser.control" => Ok(Self::BrowserControl),
            "network.out" => Ok(Self::NetworkOut),
            "sudo" => Ok(Self::Sudo),
            _ => Err(format!("unknown capability: {value}")),
        }
    }
}

/// Lifetime assigned to a granted capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrantScope {
    /// Valid for one capability check and consumed after use.
    Once,
    /// Valid for the current local session.
    Session,
    /// Valid for fifteen minutes after grant time.
    FifteenMinutes,
    /// Valid until revoked.
    Always,
}

impl GrantScope {
    /// Returns the stable CLI/database representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Session => "session",
            Self::FifteenMinutes => "15m",
            Self::Always => "always",
        }
    }

    /// Returns whether this grant represents fresh user confirmation.
    pub fn is_fresh(self) -> bool {
        matches!(self, Self::Once | Self::Session)
    }
}

impl fmt::Display for GrantScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for GrantScope {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "once" => Ok(Self::Once),
            "session" => Ok(Self::Session),
            "15m" => Ok(Self::FifteenMinutes),
            "always" => Ok(Self::Always),
            _ => Err(format!("unknown grant scope: {value}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{Capability, GrantScope};

    #[test]
    fn capability_round_trips_happy_path() {
        let capability = Capability::from_str("shell.shell").unwrap();

        assert_eq!(capability, Capability::ShellShell);
        assert_eq!(capability.to_string(), "shell.shell");
    }

    #[test]
    fn shell_capabilities_parse_independently_edge_case() {
        assert_eq!(
            Capability::from_str("shell.exec").unwrap(),
            Capability::ShellExec
        );
        assert_eq!(
            Capability::from_str("shell.shell").unwrap(),
            Capability::ShellShell
        );
        assert_eq!(
            Capability::from_str("shell.run").unwrap(),
            Capability::ShellShell
        );
    }

    #[test]
    fn unknown_capability_errors_error_path() {
        let error = Capability::from_str("root.everything").unwrap_err();

        assert!(error.contains("unknown capability"));
    }

    #[test]
    fn grant_scope_freshness_edge_case() {
        assert!(GrantScope::Once.is_fresh());
        assert!(GrantScope::Session.is_fresh());
        assert!(!GrantScope::FifteenMinutes.is_fresh());
        assert!(!GrantScope::Always.is_fresh());
    }
}
