//! Input hints for String parameters.
//!
//! Hints tell the UI which specialized input widget to render
//! (e.g., a date picker, color picker, URL input with validation).

use serde::{Deserialize, Serialize};

/// UI rendering hint for String parameters.
///
/// Does not change the underlying data type (always stored as String).
/// The UI uses the hint to render a specialized input widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InputHint {
    /// Default text input.
    #[default]
    Text,
    /// URL input with format validation hint.
    Url,
    /// Email input with format validation hint.
    Email,
    /// Date picker (YYYY-MM-DD).
    Date,
    /// Date and time picker (ISO 8601).
    DateTime,
    /// Time picker (HH:MM:SS).
    Time,
    /// Color picker (hex string).
    Color,
    /// Password input (masked).
    Password,
    /// Phone number input.
    Phone,
    /// IP address input.
    Ip,
}

impl InputHint {
    /// Returns true if this is the default hint (`Text`).
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Text)
    }
}
