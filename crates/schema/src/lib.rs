//! Schema system for Nebula workflow surfaces.
//!
//! `nebula-schema` is the replacement for `nebula-parameter`.
//! It provides:
//! - typed schema field definitions
//! - shared validation rules via `nebula-validator::Rule`
//! - serde-friendly schema wire formats

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Error types for schema operations.
pub mod error;
/// Typed field definitions and wrappers.
pub mod field;
/// Strongly typed field identifiers.
pub mod key;
/// Visibility/required mode configuration.
pub mod mode;
/// Select-option models.
pub mod option;
/// Typed references to schema fields.
pub mod path;
/// Validation report models.
pub mod report;
/// Top-level schema aggregate.
pub mod schema;
/// Value transformer definitions.
pub mod transformer;
/// Runtime value wrappers and wire-format helpers.
pub mod value;
/// Typed widget hints by field family.
pub mod widget;

pub use error::SchemaError;
pub use field::{
    BooleanField, CodeField, ColorField, ComputedField, ComputedReturn, DateField, DateTimeField,
    DynamicField, Field, FileField, HiddenField, ListField, ModeField, ModeVariant, NoticeField,
    NoticeSeverity, NumberField, ObjectField, SecretField, SelectField, StringField, TimeField,
};
pub use key::FieldKey;
pub use mode::{RequiredMode, VisibilityMode};
pub use nebula_validator::ExecutionMode;
pub use option::SelectOption;
pub use path::FieldPath;
pub use report::{ValidationIssue, ValidationReport};
pub use schema::Schema;
pub use transformer::Transformer;
pub use value::{EXPRESSION_KEY, FieldValue, FieldValues};
pub use widget::{
    BooleanWidget, CodeWidget, ListWidget, NumberWidget, ObjectWidget, SecretWidget, SelectWidget,
    StringWidget,
};

/// Common imports for schema definition code.
pub mod prelude {
    pub use nebula_validator::Rule;

    pub use crate::{
        BooleanField, BooleanWidget, CodeWidget, Field, FieldKey, ListWidget, NumberField,
        NumberWidget, ObjectField, ObjectWidget, RequiredMode, Schema, SchemaError, SecretField,
        SecretWidget, SelectField, SelectOption, SelectWidget, StringField, StringWidget,
        VisibilityMode,
    };
}
