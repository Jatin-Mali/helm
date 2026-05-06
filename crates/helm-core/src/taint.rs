//! Taint markers for values derived from users, tools, or external sources.

use serde::{Deserialize, Serialize};

/// Trust marker associated with a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Taint {
    /// Data supplied directly by the user; lowest taint in HELM v0.1.
    User,
    /// Data produced by a named tool.
    Tool {
        /// Tool name that produced the data.
        name: String,
    },
    /// Data from a named external source; highest taint in HELM v0.1.
    External {
        /// External source label.
        source: String,
    },
}

/// A value paired with a taint marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tainted<T> {
    /// The wrapped value.
    pub value: T,
    /// The taint assigned to the value.
    pub taint: Taint,
}

impl Taint {
    /// Returns the higher taint of two inputs using External > Tool > User.
    pub fn escalate(&self, other: &Self) -> Self {
        if self.rank() >= other.rank() {
            self.clone()
        } else {
            other.clone()
        }
    }

    /// Returns the stable audit representation for this taint.
    pub fn label(&self) -> String {
        match self {
            Self::User => "user".to_owned(),
            Self::Tool { name } => format!("tool:{name}"),
            Self::External { source } => format!("external:{source}"),
        }
    }

    /// Returns true when the value came from web/browser/email/downloaded content.
    pub fn is_external(&self) -> bool {
        matches!(self, Self::External { .. })
    }

    fn rank(&self) -> u8 {
        match self {
            Self::User => 0,
            Self::Tool { .. } => 1,
            Self::External { .. } => 2,
        }
    }
}

impl<T> Tainted<T> {
    /// Wraps a value supplied by the user.
    pub fn user(value: T) -> Self {
        Self {
            value,
            taint: Taint::User,
        }
    }

    /// Wraps a value produced by a named tool.
    pub fn tool(value: T, name: impl Into<String>) -> Self {
        Self {
            value,
            taint: Taint::Tool { name: name.into() },
        }
    }

    /// Wraps a value from a named external source.
    pub fn external(value: T, source: impl Into<String>) -> Self {
        Self {
            value,
            taint: Taint::External {
                source: source.into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Taint, Tainted};

    fn tool() -> Taint {
        Taint::Tool {
            name: "shell".to_owned(),
        }
    }

    fn external() -> Taint {
        Taint::External {
            source: "web".to_owned(),
        }
    }

    #[test]
    fn escalate_picks_correctly_happy_path() {
        assert_eq!(Taint::User.escalate(&tool()), tool());
        assert_eq!(tool().escalate(&external()), external());
        assert_eq!(external().escalate(&Taint::User), external());
    }

    #[test]
    fn escalate_is_associative_edge_case() {
        let a = Taint::User;
        let b = tool();
        let c = external();

        assert_eq!(a.escalate(&b).escalate(&c), a.escalate(&b.escalate(&c)));
    }

    #[test]
    fn tainted_helpers_apply_expected_taint() {
        assert_eq!(Tainted::user(1).taint, Taint::User);
        assert_eq!(Tainted::tool(1, "shell").taint, tool());
        assert_eq!(Tainted::external(1, "web").taint, external());
    }

    #[test]
    fn equal_rank_keeps_left_error_path() {
        let left = Taint::Tool {
            name: "left".to_owned(),
        };
        let right = Taint::Tool {
            name: "right".to_owned(),
        };

        assert_eq!(left.escalate(&right), left);
    }
}
