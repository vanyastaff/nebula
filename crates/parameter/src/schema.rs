//! Canonical v2 parameter schema — RFC 0005 reference implementation.
//!
//! The schema is the top-level container for a node's parameter definition.
//! It holds an ordered list of [`Field`] definitions plus optional UI-only
//! and layout metadata.
//!
//! ## Validation
//!
//! Call [`Schema::validate`] for strict validation (unknown fields are errors).
//! Call [`Schema::validate_with_profile`] to control how unknown fields are treated.
//!
//! ## Normalization
//!
//! Call [`Schema::normalize_values`] to backfill defaults and mode variants
//! before presenting values to a user or persisting them.

use crate::field::Field;
use crate::profile::ValidationProfile;
use crate::report::ValidationReport;
use crate::rules::Rule;
use crate::runtime::ValidatedValues;
use crate::values::FieldValues;

/// Complete parameter schema for v2 authoring.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Schema {
    /// Ordered field definitions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    /// UI-only elements that never appear in runtime values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ui: Vec<UiElement>,
    /// Optional visual grouping metadata.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
}

impl Schema {
    /// Creates an empty schema.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a field to the schema.
    #[must_use]
    pub fn field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }

    /// Appends a UI-only element to the schema.
    #[must_use]
    pub fn ui(mut self, element: UiElement) -> Self {
        self.ui.push(element);
        self
    }

    /// Appends a field group definition.
    #[must_use]
    pub fn group(mut self, group: Group) -> Self {
        self.groups.push(group);
        self
    }

    /// Returns the number of value-bearing fields in the schema.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns `true` if the schema contains no fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Returns the field with the given id, if any.
    #[must_use]
    pub fn get_field(&self, id: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.meta().id == id)
    }

    /// Returns `true` if the schema contains a field with the given id.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.fields.iter().any(|f| f.meta().id == id)
    }

    /// Validates `values` against this schema using strict defaults.
    ///
    /// Unknown fields are treated as hard errors. Use [`validate_with_profile`]
    /// to relax this behaviour.
    ///
    /// On success, returns [`ValidatedValues`] — a proof that the values
    /// passed schema validation.
    ///
    /// # Errors
    ///
    /// Returns a non-empty list of [`crate::error::ParameterError`] on failure.
    ///
    /// [`validate_with_profile`]: Schema::validate_with_profile
    pub fn validate(
        &self,
        values: &FieldValues,
    ) -> Result<ValidatedValues, Vec<crate::error::ParameterError>> {
        crate::validate::validate_fields(&self.fields, values)
            .map(|()| ValidatedValues::new(values.clone()))
    }

    /// Validates `values` under the given [`ValidationProfile`].
    ///
    /// Returns a [`ValidationReport`] that separates hard errors from warnings.
    /// Call [`ValidationReport::into_validated`] to extract the validated values
    /// when no hard errors are present.
    #[must_use]
    pub fn validate_with_profile(
        &self,
        values: &FieldValues,
        profile: ValidationProfile,
    ) -> ValidationReport {
        crate::validate::validate_with_profile(&self.fields, values, profile)
    }

    /// Normalizes runtime values using schema defaults.
    ///
    /// Existing user-provided values are preserved. Missing fields are
    /// materialized from `default` metadata and mode default variants.
    #[must_use]
    pub fn normalize_values(&self, values: &FieldValues) -> FieldValues {
        crate::normalize::normalize_fields(&self.fields, values)
    }
}

/// Non-value schema element.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiElement {
    /// Informational message.
    Notice {
        /// Severity level.
        severity: Severity,
        /// Display text.
        text: String,
        /// Show only when the condition is true.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visible_when: Option<Rule>,
    },
    /// Runtime-driven action button.
    Button {
        /// Display label.
        label: String,
        /// Action key forwarded to the runtime.
        action: String,
        /// Enable only when the condition is true.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled_when: Option<Rule>,
    },
}

/// UI severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Informational state.
    Info,
    /// Warning state.
    Warning,
    /// Error state.
    Error,
}

/// Visual field grouping.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Group {
    /// Group title.
    pub label: String,
    /// Ordered field ids in the group.
    pub fields: Vec<String>,
    /// Whether the group is initially collapsed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::field::Field;
    use crate::metadata::FieldMetadata;
    use crate::option::{OptionSource, SelectOption};
    use crate::rules::Rule;
    use crate::spec::{DynamicFieldsMode, ModeVariant, UnknownFieldPolicy};
    use crate::values::FieldValues;

    #[test]
    fn file_serializes_max_size_key() {
        let field = Field::File {
            meta: FieldMetadata {
                id: "attachment".to_owned(),
                label: "Attachment".to_owned(),
                ..FieldMetadata::default()
            },
            accept: Some("application/pdf".to_owned()),
            max_size: Some(1_024),
            multiple: false,
        };

        let value = serde_json::to_value(&field).expect("field should serialize");
        assert_eq!(value.get("max_size"), Some(&json!(1_024)));
        assert!(value.get("max_size_bytes").is_none());
    }

    #[test]
    fn dynamic_fields_serializes_mode_key() {
        let field = Field::DynamicFields {
            meta: FieldMetadata {
                id: "row_data".to_owned(),
                label: "Row Data".to_owned(),
                ..FieldMetadata::default()
            },
            provider: "sheets.columns".to_owned(),
            depends_on: vec!["sheet_id".to_owned()],
            mode: DynamicFieldsMode::RequiredOnly,
            unknown_field_policy: UnknownFieldPolicy::WarnKeep,
            loader: None,
        };

        let value = serde_json::to_value(&field).expect("field should serialize");
        assert_eq!(value.get("mode"), Some(&json!("required_only")));
    }

    #[test]
    fn validate_reports_required_when_condition_holds() {
        let schema = Schema::new().field(Field::text("token").with_label("Token").required_when(
            Rule::Eq {
                field: "auth".to_owned(),
                value: json!("bearer"),
            },
        ));

        let mut values = FieldValues::new();
        values.set("auth", json!("bearer"));

        let result = schema.validate(&values);
        assert!(result.is_err());
    }

    #[test]
    fn validate_applies_static_select_membership() {
        let field = Field::Select {
            meta: FieldMetadata {
                id: "method".to_owned(),
                label: "Method".to_owned(),
                ..FieldMetadata::default()
            },
            source: OptionSource::Static {
                options: vec![
                    SelectOption::new(json!("GET"), "GET"),
                    SelectOption::new(json!("POST"), "POST"),
                ],
            },
            multiple: false,
            allow_custom: false,
            searchable: false,
            loader: None,
        };
        let schema = Schema::new().field(field);

        let mut values = FieldValues::new();
        values.set("method", json!("PATCH"));

        let result = schema.validate(&values);
        assert!(result.is_err());
    }

    #[test]
    fn validate_emits_structured_validation_issue() {
        let schema = Schema::new().field(Field::text("username").with_label("Username").with_rule(
            Rule::MinLength {
                min: 5,
                message: None,
            },
        ));

        let mut values = FieldValues::new();
        values.set("username", json!("abc"));

        let result = schema
            .validate(&values)
            .expect_err("value must fail min_length");
        assert!(matches!(
            &result[0],
            crate::error::ParameterError::ValidationIssue {
                key,
                code,
                reason,
                ..
            } if key == "username" && code == "min_length" && !reason.is_empty()
        ));
    }

    #[test]
    fn validate_reports_unknown_top_level_field() {
        let schema = Schema::new().field(Field::text("known").with_label("Known"));
        let mut values = FieldValues::new();
        values.set("known", json!("ok"));
        values.set("unexpected", json!(true));

        let errors = schema
            .validate(&values)
            .expect_err("unknown key should fail validation");
        assert!(errors.iter().any(|error| {
            matches!(
                error,
                crate::error::ParameterError::UnknownField { key } if key == "unexpected"
            )
        }));
    }

    #[test]
    fn validate_reports_unknown_nested_object_field() {
        let schema = Schema::new().field(Field::Object {
            meta: FieldMetadata {
                id: "auth".to_owned(),
                label: "Auth".to_owned(),
                ..FieldMetadata::default()
            },
            fields: vec![Field::text("token").with_label("Token")],
        });

        let mut values = FieldValues::new();
        values.set(
            "auth",
            json!({
                "token": "abc",
                "extra": "not allowed"
            }),
        );

        let errors = schema
            .validate(&values)
            .expect_err("unknown nested key should fail validation");
        assert!(errors.iter().any(|error| {
            matches!(
                error,
                crate::error::ParameterError::UnknownField { key } if key == "auth.extra"
            )
        }));
    }

    #[test]
    fn normalize_values_applies_top_level_defaults() {
        let schema = Schema::new().field(
            Field::text("region")
                .with_label("Region")
                .with_default(json!("us-east-1")),
        );
        let values = FieldValues::new();

        let normalized = schema.normalize_values(&values);
        assert_eq!(normalized.get("region"), Some(&json!("us-east-1")));
    }

    #[test]
    fn normalize_values_applies_mode_default_variant() {
        let schema = Schema::new().field(Field::Mode {
            meta: FieldMetadata {
                id: "auth".to_owned(),
                label: "Auth".to_owned(),
                ..FieldMetadata::default()
            },
            variants: vec![ModeVariant {
                key: "none".to_owned(),
                label: "None".to_owned(),
                description: None,
                content: Box::new(Field::text("token").with_label("Token")),
            }],
            default_variant: Some("none".to_owned()),
        });

        let values = FieldValues::new();
        let normalized = schema.normalize_values(&values);
        assert_eq!(normalized.get("auth"), Some(&json!({ "mode": "none" })));
    }
}
