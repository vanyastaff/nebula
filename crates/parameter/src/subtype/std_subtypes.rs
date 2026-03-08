//! Standard subtype implementations using trait-based system.
//!
//! This module provides concrete implementations of common subtypes
//! that can be used with generic parameter types.

use super::traits::{BooleanSubtype, IntegerSubtype, NumberSubtype, TextSubtype};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ═══════════════════════════════════════════════════════════════════════════
// Helper macro for subtype serialization
// ═══════════════════════════════════════════════════════════════════════════

macro_rules! impl_subtype_serde {
    ($name:ident, $str_name:expr) => {
        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str($str_name)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                if s == $str_name {
                    Ok($name)
                } else {
                    Err(serde::de::Error::custom(format!(
                        "expected '{}', got '{}'",
                        $str_name, s
                    )))
                }
            }
        }
    };
}

// ═══════════════════════════════════════════════════════════════════════════
// Text Subtypes
// ═══════════════════════════════════════════════════════════════════════════

/// Plain text with no special formatting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Plain;

impl_subtype_serde!(Plain, "plain");

impl TextSubtype for Plain {
    fn name() -> &'static str {
        "plain"
    }
    fn description() -> &'static str {
        "Plain text"
    }
}

/// Email address.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Email;

impl_subtype_serde!(Email, "email");

impl TextSubtype for Email {
    fn name() -> &'static str {
        "email"
    }
    fn description() -> &'static str {
        "Email address"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^[^\s@]+@[^\s@]+\.[^\s@]+$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("user@example.com")
    }
}

/// URL address.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Url;

impl_subtype_serde!(Url, "url");

impl TextSubtype for Url {
    fn name() -> &'static str {
        "url"
    }
    fn description() -> &'static str {
        "URL address"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^https?://[^\s/$.?#].[^\s]*$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("https://example.com")
    }
}

/// Password or secret value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Password;

impl_subtype_serde!(Password, "password");

impl TextSubtype for Password {
    fn name() -> &'static str {
        "password"
    }
    fn description() -> &'static str {
        "Password or secret"
    }
    fn is_sensitive() -> bool {
        true
    }
}

/// JSON string.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Json;

impl_subtype_serde!(Json, "json");

impl TextSubtype for Json {
    fn name() -> &'static str {
        "json"
    }
    fn description() -> &'static str {
        "JSON string"
    }
    fn is_code() -> bool {
        true
    }
    fn is_multiline() -> bool {
        true
    }
}

/// UUID identifier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Uuid;

impl_subtype_serde!(Uuid, "uuid");

impl TextSubtype for Uuid {
    fn name() -> &'static str {
        "uuid"
    }
    fn description() -> &'static str {
        "UUID"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("550e8400-e29b-41d4-a716-446655440000")
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Code Subtypes (as Text)
// ═══════════════════════════════════════════════════════════════════════════

/// JavaScript code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct JavaScript;

impl_subtype_serde!(JavaScript, "javascript");

impl TextSubtype for JavaScript {
    fn name() -> &'static str {
        "javascript"
    }
    fn description() -> &'static str {
        "JavaScript code"
    }
    fn is_code() -> bool {
        true
    }
    fn is_multiline() -> bool {
        true
    }
}

/// Python code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Python;

impl_subtype_serde!(Python, "python");

impl TextSubtype for Python {
    fn name() -> &'static str {
        "python"
    }
    fn description() -> &'static str {
        "Python code"
    }
    fn is_code() -> bool {
        true
    }
    fn is_multiline() -> bool {
        true
    }
}

/// Rust code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Rust;

impl_subtype_serde!(Rust, "rust");

impl TextSubtype for Rust {
    fn name() -> &'static str {
        "rust"
    }
    fn description() -> &'static str {
        "Rust code"
    }
    fn is_code() -> bool {
        true
    }
    fn is_multiline() -> bool {
        true
    }
}

/// SQL query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Sql;

impl_subtype_serde!(Sql, "sql");

impl TextSubtype for Sql {
    fn name() -> &'static str {
        "sql"
    }
    fn description() -> &'static str {
        "SQL query"
    }
    fn is_code() -> bool {
        true
    }
    fn is_multiline() -> bool {
        true
    }
}

/// YAML configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Yaml;

impl_subtype_serde!(Yaml, "yaml");

impl TextSubtype for Yaml {
    fn name() -> &'static str {
        "yaml"
    }
    fn description() -> &'static str {
        "YAML configuration"
    }
    fn is_code() -> bool {
        true
    }
    fn is_multiline() -> bool {
        true
    }
}

/// Shell script.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Shell;

impl_subtype_serde!(Shell, "shell");

impl TextSubtype for Shell {
    fn name() -> &'static str {
        "shell"
    }
    fn description() -> &'static str {
        "Shell script"
    }
    fn is_code() -> bool {
        true
    }
    fn is_multiline() -> bool {
        true
    }
}

/// Markdown text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Markdown;

impl_subtype_serde!(Markdown, "markdown");

impl TextSubtype for Markdown {
    fn name() -> &'static str {
        "markdown"
    }
    fn description() -> &'static str {
        "Markdown text"
    }
    fn is_multiline() -> bool {
        true
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Color Subtypes (as Text)
// ═══════════════════════════════════════════════════════════════════════════

/// Hex color code (#RRGGBB).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct HexColor;

impl_subtype_serde!(HexColor, "hex_color");

impl TextSubtype for HexColor {
    fn name() -> &'static str {
        "hex_color"
    }
    fn description() -> &'static str {
        "Hex color (#RRGGBB)"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^#[0-9A-Fa-f]{6}$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("#FF5733")
    }
}

/// RGB color (rgb(r, g, b)).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct RgbColor;

impl_subtype_serde!(RgbColor, "rgb_color");

impl TextSubtype for RgbColor {
    fn name() -> &'static str {
        "rgb_color"
    }
    fn description() -> &'static str {
        "RGB color"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^rgb\(\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}\s*\)$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("rgb(255, 87, 51)")
    }
}

/// HSL color (hsl(h, s%, l%)).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct HslColor;

impl_subtype_serde!(HslColor, "hsl_color");

impl TextSubtype for HslColor {
    fn name() -> &'static str {
        "hsl_color"
    }
    fn description() -> &'static str {
        "HSL color"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^hsl\(\s*\d{1,3}\s*,\s*\d{1,3}%\s*,\s*\d{1,3}%\s*\)$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("hsl(9, 100%, 60%)")
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Date/Time Subtypes (as Text)
// ═══════════════════════════════════════════════════════════════════════════

/// ISO 8601 date (YYYY-MM-DD).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct IsoDate;

impl_subtype_serde!(IsoDate, "iso_date");

impl TextSubtype for IsoDate {
    fn name() -> &'static str {
        "iso_date"
    }
    fn description() -> &'static str {
        "ISO 8601 date (YYYY-MM-DD)"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^\d{4}-\d{2}-\d{2}$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("2026-03-06")
    }
}

/// ISO 8601 datetime (YYYY-MM-DDTHH:MM:SS).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct IsoDateTime;

impl_subtype_serde!(IsoDateTime, "iso_datetime");

impl TextSubtype for IsoDateTime {
    fn name() -> &'static str {
        "iso_datetime"
    }
    fn description() -> &'static str {
        "ISO 8601 datetime"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("2026-03-06T14:30:00Z")
    }
}

/// Time (HH:MM or HH:MM:SS).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Time;

impl_subtype_serde!(Time, "time");

impl TextSubtype for Time {
    fn name() -> &'static str {
        "time"
    }
    fn description() -> &'static str {
        "Time (HH:MM:SS)"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^\d{2}:\d{2}(:\d{2})?$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("14:30:00")
    }
}

/// Birthday (MM-DD or YYYY-MM-DD).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Birthday;

impl_subtype_serde!(Birthday, "birthday");

impl TextSubtype for Birthday {
    fn name() -> &'static str {
        "birthday"
    }
    fn description() -> &'static str {
        "Birthday date"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^\d{4}-\d{2}-\d{2}$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("1990-01-15")
    }
}

/// Expiry date (MM/YY or MM/YYYY).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ExpiryDate;

impl_subtype_serde!(ExpiryDate, "expiry_date");

impl TextSubtype for ExpiryDate {
    fn name() -> &'static str {
        "expiry_date"
    }
    fn description() -> &'static str {
        "Expiry date (MM/YY)"
    }
    fn pattern() -> Option<&'static str> {
        Some(r"^\d{2}/\d{2,4}$")
    }
    fn placeholder() -> Option<&'static str> {
        Some("12/26")
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Boolean Subtypes
// ═══════════════════════════════════════════════════════════════════════════

/// Generic boolean toggle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Toggle;

impl_subtype_serde!(Toggle, "toggle");

impl BooleanSubtype for Toggle {
    fn name() -> &'static str {
        "toggle"
    }

    fn description() -> &'static str {
        "Boolean toggle"
    }

    fn default_value() -> Option<bool> {
        Some(false)
    }
}

/// Feature flag switch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FeatureFlag;

impl_subtype_serde!(FeatureFlag, "feature_flag");

impl BooleanSubtype for FeatureFlag {
    fn name() -> &'static str {
        "feature_flag"
    }

    fn description() -> &'static str {
        "Feature flag"
    }

    fn label() -> Option<&'static str> {
        Some("Enable feature")
    }

    fn help_text() -> Option<&'static str> {
        Some("Controls rollout of this feature")
    }

    fn default_value() -> Option<bool> {
        Some(false)
    }
}

/// User consent checkbox.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Consent;

impl_subtype_serde!(Consent, "consent");

impl BooleanSubtype for Consent {
    fn name() -> &'static str {
        "consent"
    }

    fn description() -> &'static str {
        "Consent confirmation"
    }

    fn label() -> Option<&'static str> {
        Some("I agree")
    }

    fn help_text() -> Option<&'static str> {
        Some("Required to continue")
    }

    fn default_value() -> Option<bool> {
        Some(false)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Number Subtypes
// ═══════════════════════════════════════════════════════════════════════════

/// Generic number with no constraints.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GenericNumber;

impl_subtype_serde!(GenericNumber, "number");

impl NumberSubtype for GenericNumber {
    type Value = f64;

    fn name() -> &'static str {
        "number"
    }
    fn description() -> &'static str {
        "Plain number"
    }
}

/// Network port number (1-65535).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Port;

impl_subtype_serde!(Port, "port");

impl NumberSubtype for Port {
    type Value = u16;

    fn name() -> &'static str {
        "port"
    }
    fn description() -> &'static str {
        "Port number"
    }
    fn default_range() -> Option<(Self::Value, Self::Value)> {
        Some((1, 65535))
    }

    fn default_step() -> Option<Self::Value> {
        Some(1)
    }
}

impl IntegerSubtype for Port {}

/// Percentage (0-100).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Percentage;

impl_subtype_serde!(Percentage, "percentage");

impl NumberSubtype for Percentage {
    type Value = f64;

    fn name() -> &'static str {
        "percentage"
    }
    fn description() -> &'static str {
        "Percentage"
    }
    fn default_range() -> Option<(Self::Value, Self::Value)> {
        Some((0.0, 100.0))
    }
    fn is_percentage() -> bool {
        true
    }
}

/// Factor or multiplier (0-1).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Factor;

impl_subtype_serde!(Factor, "factor");

impl NumberSubtype for Factor {
    type Value = f64;

    fn name() -> &'static str {
        "factor"
    }
    fn description() -> &'static str {
        "Factor or multiplier"
    }
    fn default_range() -> Option<(Self::Value, Self::Value)> {
        Some((0.0, 1.0))
    }
}

/// Unix timestamp in seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Timestamp;

impl_subtype_serde!(Timestamp, "timestamp");

impl NumberSubtype for Timestamp {
    type Value = i64;

    fn name() -> &'static str {
        "timestamp"
    }
    fn description() -> &'static str {
        "Unix timestamp (seconds)"
    }

    fn default_step() -> Option<Self::Value> {
        Some(1)
    }
}

impl IntegerSubtype for Timestamp {}

/// Distance measurement.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Distance;

impl_subtype_serde!(Distance, "distance");

impl NumberSubtype for Distance {
    type Value = f64;

    fn name() -> &'static str {
        "distance"
    }
    fn description() -> &'static str {
        "Distance measurement"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_subtype() {
        assert_eq!(Email::name(), "email");
        assert_eq!(Email::description(), "Email address");
        assert!(Email::pattern().is_some());
        assert!(!Email::is_sensitive());
    }

    #[test]
    fn test_password_subtype() {
        assert_eq!(Password::name(), "password");
        assert!(Password::is_sensitive());
    }

    #[test]
    fn test_boolean_subtypes() {
        assert_eq!(Toggle::name(), "toggle");
        assert_eq!(FeatureFlag::name(), "feature_flag");
        assert_eq!(Consent::name(), "consent");
        assert_eq!(Consent::default_value(), Some(false));
    }

    #[test]
    fn test_port_subtype() {
        assert_eq!(Port::name(), "port");
        assert_eq!(Port::default_range(), Some((1, 65535)));
    }

    #[test]
    fn test_percentage_subtype() {
        assert_eq!(Percentage::name(), "percentage");
        assert!(Percentage::is_percentage());
        assert_eq!(Percentage::default_range(), Some((0.0, 100.0)));
    }

    #[test]
    fn test_serde_text_subtype() {
        let email = Email;
        let json = serde_json::to_string(&email).unwrap();
        assert_eq!(json, r#""email""#);

        let deserialized: Email = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, email);
    }

    #[test]
    fn test_serde_number_subtype() {
        let port = Port;
        let json = serde_json::to_string(&port).unwrap();
        assert_eq!(json, r#""port""#);

        let deserialized: Port = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, port);
    }
}
