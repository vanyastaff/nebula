use nebula_validator::Rule;
use serde::{Deserialize, Serialize};

/// Visibility policy for a field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisibilityMode {
    /// Field is always visible.
    #[default]
    Always,
    /// Field is visible only when rule evaluates to true.
    When(Rule),
}

impl VisibilityMode {
    /// Returns true when mode is the default variant.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Always)
    }
}

/// Requiredness policy for a field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequiredMode {
    /// Field is optional.
    #[default]
    Never,
    /// Field is always required.
    Always,
    /// Field is required only when rule evaluates to true.
    When(Rule),
}

impl RequiredMode {
    /// Returns true when mode is the default variant.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Never)
    }
}

#[cfg(test)]
mod tests {
    use super::{RequiredMode, VisibilityMode};

    #[test]
    fn defaults_match_contract() {
        assert!(VisibilityMode::default().is_default());
        assert!(RequiredMode::default().is_default());
    }
}
