//! Filter field definitions for dynamic filtering UIs.

use serde::{Deserialize, Serialize};

use crate::option::SelectOption;

/// A field definition used in filter/search UIs.
///
/// Each `FilterField` describes one filterable dimension — for example
/// "Status", "Created Date", or "Priority" — along with its data type.
///
/// # Examples
///
/// ```
/// use nebula_parameter::filter_field::{FilterField, FilterFieldType};
/// use nebula_parameter::option::SelectOption;
///
/// let field = FilterField {
///     id: "priority".into(),
///     label: "Priority".into(),
///     field_type: FilterFieldType::Enum {
///         options: vec![
///             SelectOption::new(serde_json::json!("high"), "High"),
///             SelectOption::new(serde_json::json!("low"), "Low"),
///         ],
///     },
/// };
/// assert_eq!(field.id, "priority");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterField {
    /// Unique identifier for this filter field.
    pub id: String,

    /// Human-readable display label.
    pub label: String,

    /// The data type of this filter field.
    #[serde(default, skip_serializing_if = "FilterFieldType::is_default")]
    pub field_type: FilterFieldType,
}

/// The data type of a [`FilterField`].
///
/// Defaults to [`String`](FilterFieldType::String).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterFieldType {
    /// Free-text string filter.
    #[default]
    String,

    /// Numeric filter.
    Number,

    /// Boolean (true/false) filter.
    Boolean,

    /// Date-only filter (no time component).
    Date,

    /// Date-and-time filter.
    DateTime,

    /// Enumerated set of allowed values.
    Enum {
        /// The selectable options.
        options: Vec<SelectOption>,
    },
}

impl FilterFieldType {
    /// Returns `true` if this is the default variant ([`String`](Self::String)).
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::String)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_field_type_is_string() {
        assert_eq!(FilterFieldType::default(), FilterFieldType::String);
        assert!(FilterFieldType::String.is_default());
        assert!(!FilterFieldType::Number.is_default());
    }

    #[test]
    fn serde_round_trip_string_field() {
        let field = FilterField {
            id: "name".into(),
            label: "Name".into(),
            field_type: FilterFieldType::String,
        };

        let json = serde_json::to_string(&field).unwrap();
        // Default field_type should be omitted
        assert!(!json.contains("field_type"));

        let deserialized: FilterField = serde_json::from_str(&json).unwrap();
        assert_eq!(field, deserialized);
    }

    #[test]
    fn serde_round_trip_enum_field() {
        let field = FilterField {
            id: "status".into(),
            label: "Status".into(),
            field_type: FilterFieldType::Enum {
                options: vec![
                    SelectOption::new(serde_json::json!("open"), "Open"),
                    SelectOption::new(serde_json::json!("closed"), "Closed"),
                ],
            },
        };

        let json = serde_json::to_string(&field).unwrap();
        assert!(json.contains("\"field_type\""));

        let deserialized: FilterField = serde_json::from_str(&json).unwrap();
        assert_eq!(field, deserialized);
    }

    #[test]
    fn serde_round_trip_all_variants() {
        let variants = [
            FilterFieldType::String,
            FilterFieldType::Number,
            FilterFieldType::Boolean,
            FilterFieldType::Date,
            FilterFieldType::DateTime,
            FilterFieldType::Enum {
                options: vec![SelectOption::new(serde_json::json!(1), "One")],
            },
        ];

        for variant in &variants {
            let field = FilterField {
                id: "test".into(),
                label: "Test".into(),
                field_type: variant.clone(),
            };
            let json = serde_json::to_string(&field).unwrap();
            let deserialized: FilterField = serde_json::from_str(&json).unwrap();
            assert_eq!(field, deserialized);
        }
    }

    #[test]
    fn deserialize_without_field_type_defaults_to_string() {
        let json = r#"{"id":"q","label":"Query"}"#;
        let field: FilterField = serde_json::from_str(json).unwrap();
        assert_eq!(field.field_type, FilterFieldType::String);
    }
}
