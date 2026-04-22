//! Visibility / required / expression policy enums.

use nebula_validator::Rule;
use serde::{Deserialize, Serialize};

/// When a field is visible in the UI.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisibilityMode {
    /// Field is always visible (default).
    #[default]
    Always,
    /// Never visible — replaces the removed `Field::Hidden`.
    Never,
    /// Visible only when rule evaluates true.
    When(Rule),
}

impl VisibilityMode {
    /// Returns true when mode is the default variant.
    #[must_use]
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Always)
    }
}

/// When a field is required.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequiredMode {
    /// Field is optional (default).
    #[default]
    Never,
    /// Field is always required.
    Always,
    /// Field is required only when rule evaluates to true.
    When(Rule),
}

impl RequiredMode {
    /// Returns true when mode is the default variant.
    #[must_use]
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Never)
    }
}

/// Whether the field accepts expression values (`{{ ... }}` or `{"$expr": "..."}`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpressionMode {
    /// Only literal values allowed.
    Forbidden,
    /// Both literal and expression values allowed (default).
    #[default]
    Allowed,
    /// Only expression values (e.g. Computed field).
    Required,
}

impl ExpressionMode {
    /// Returns true when mode is the default variant (`Allowed`).
    #[must_use]
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_default_is_always() {
        assert!(matches!(VisibilityMode::default(), VisibilityMode::Always));
        assert!(VisibilityMode::default().is_default());
    }

    #[test]
    fn required_default_is_never() {
        assert!(matches!(RequiredMode::default(), RequiredMode::Never));
        assert!(RequiredMode::default().is_default());
    }

    #[test]
    fn expression_default_is_allowed() {
        assert!(matches!(ExpressionMode::default(), ExpressionMode::Allowed));
    }

    #[test]
    fn visibility_never_hides_always() {
        assert!(!VisibilityMode::Never.is_default());
    }
}
