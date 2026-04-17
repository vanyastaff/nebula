//! UI hints for String fields (replaces v3 separate Date/Time/Color types).

use serde::{Deserialize, Serialize};

/// Semantic hint for rendering a string input.
///
/// Attach to a [`StringField`](crate::StringField) to give the UI a rendering
/// hint without changing validation semantics.
///
/// # Example
///
/// ```rust
/// use nebula_schema::{Field, InputHint, field_key};
///
/// let field = Field::string(field_key!("contact_email")).hint(InputHint::Email);
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputHint {
    /// Plain single-line text input.
    #[default]
    Text,
    /// Email address input.
    Email,
    /// URL input.
    Url,
    /// Masked password input.
    Password,
    /// Phone number input.
    Phone,
    /// IP address input.
    Ip,
    /// Regular expression editor.
    Regex,
    /// Markdown editor.
    Markdown,
    /// Cron expression input.
    Cron,
    /// Date picker (ISO 8601 date).
    Date,
    /// Date and time picker (ISO 8601 datetime).
    DateTime,
    /// Time picker (ISO 8601 time).
    Time,
    /// Color picker (hex or CSS color).
    Color,
    /// Duration input (ISO 8601 duration).
    Duration,
    /// UUID input.
    Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_text() {
        assert_eq!(InputHint::default(), InputHint::Text);
    }

    #[test]
    fn serde_uses_snake_case() {
        let json = serde_json::to_string(&InputHint::DateTime).unwrap();
        assert_eq!(json, "\"date_time\"");
    }
}
