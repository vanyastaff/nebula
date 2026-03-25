//! Notice severity for display-only parameter blocks.

use serde::{Deserialize, Serialize};

/// Severity level for a [`ParameterType::Notice`](crate::parameter_type::ParameterType::Notice).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeSeverity {
    /// Informational.
    #[default]
    Info,
    /// Warning.
    Warning,
    /// Success.
    Success,
    /// Danger / error.
    Danger,
}

impl NoticeSeverity {
    /// Returns `true` for the default variant ([`Info`](Self::Info)).
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Info)
    }
}
