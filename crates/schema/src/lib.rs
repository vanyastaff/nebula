//! Schema system for Nebula workflow surfaces.
//!
//! `nebula-schema` is the replacement for `nebula-parameter`.
//! It provides:
//! - typed schema field definitions
//! - shared validation rules via `nebula-validator::Rule`
//! - serde-friendly schema wire formats

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// [`nebula_validator::RuleContext`] adapters backed by [`FieldValues`].
pub(crate) mod context;
/// Error types for schema operations.
pub mod error;
/// Expression value wrapper (Task 14 adds lazy parse + OnceLock).
pub mod expression;
/// Typed field definitions and wrappers.
pub mod field;
/// UI hints for string input rendering.
pub mod input_hint;
/// Strongly typed field identifiers.
pub mod key;
/// Static schema lint diagnostics.
pub mod lint;
/// Runtime loader registry and async loader types.
pub mod loader;
/// Visibility/required mode configuration.
pub mod mode;
/// Select-option models.
pub mod option;
/// Typed references to schema fields.
pub mod path;
/// Legacy validation report models (kept for schema.rs; will be deleted in a later task).
#[doc(hidden)]
pub mod report;
/// Top-level schema aggregate.
pub mod schema;
/// Value transformer definitions.
pub mod transformer;
/// Validated schema proof-tokens (ValidSchema, FieldHandle, SchemaFlags).
pub mod validated;
/// Runtime value wrappers and wire-format helpers.
pub mod value;
/// Typed widget hints by field family.
pub mod widget;

pub use error::{SchemaError, Severity, ValidationError, ValidationErrorBuilder, ValidationReport};
pub use expression::{Expression, ExpressionAst, ExpressionContext};
pub use field::{
    BooleanField, CodeField, ComputedField, ComputedReturn, DynamicField, Field, FileField,
    ListField, ModeField, ModeVariant, NoticeField, NoticeSeverity, NumberField, ObjectField,
    SecretField, SelectField, StringField,
};
pub use input_hint::InputHint;
pub use key::FieldKey;
pub use lint::{LintDiagnostic, LintLevel, LintReport, lint_schema};
pub use loader::{
    Loader, LoaderContext, LoaderFuture, LoaderRegistry, LoaderResult, OptionLoader, RecordLoader,
};
pub use mode::{ExpressionMode, RequiredMode, VisibilityMode};
pub use nebula_schema_macros::field_key;
pub use nebula_validator::ExecutionMode;
pub use option::SelectOption;
pub use path::FieldPath;
pub use report::ValidationIssue;
pub use schema::{Schema, SchemaBuilder};
pub use transformer::Transformer;
pub use validated::{FieldHandle, ResolvedValues, SchemaFlags, ValidSchema, ValidValues};
pub use value::{EXPRESSION_KEY, FieldValue, FieldValues};
pub use widget::{
    BooleanWidget, CodeWidget, ListWidget, NumberWidget, ObjectWidget, SecretWidget, SelectWidget,
    StringWidget,
};

/// Common imports for schema definition code.
pub mod prelude {
    pub use nebula_validator::Rule;

    pub use crate::{
        BooleanField, BooleanWidget, CodeWidget, Field, FieldKey, ListWidget, LoaderContext,
        LoaderRegistry, NumberField, NumberWidget, ObjectField, ObjectWidget, RequiredMode, Schema,
        SchemaError, SecretField, SecretWidget, SelectField, SelectOption, SelectWidget,
        StringField, StringWidget, VisibilityMode,
    };
}
