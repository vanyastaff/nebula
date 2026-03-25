//! Object display mode controlling UI presentation and normalization behavior.

use serde::{Deserialize, Serialize};

/// Controls how an Object parameter renders its sub-parameters.
///
/// Affects both UI presentation and normalization behavior:
/// - `Inline` / `Collapsed`: all sub-parameter defaults are backfilled.
/// - `PickFields` / `Sections`: only explicitly added fields appear in values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    /// All sub-parameters rendered inline, always visible.
    #[default]
    Inline,
    /// Collapsible section with expand/collapse toggle.
    Collapsed,
    /// "Add Field" dropdown. Only added fields present in values.
    PickFields,
    /// Like PickFields, but dropdown grouped by `Parameter.group`.
    Sections,
}

impl DisplayMode {
    /// Whether this is the default display mode.
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Inline)
    }

    /// Whether this mode uses pick-style field selection.
    #[must_use]
    pub fn is_pick_mode(&self) -> bool {
        matches!(self, Self::PickFields | Self::Sections)
    }
}

/// Return type for computed parameter fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputedReturn {
    /// Computed field returns a string.
    String,
    /// Computed field returns a number.
    Number,
    /// Computed field returns a boolean.
    Boolean,
}
