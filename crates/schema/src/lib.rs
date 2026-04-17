//! `nebula-schema` — schema definition system for Nebula workflow surfaces.
//!
//! This crate provides:
//! - Typed field definitions and the `Field` enum.
//! - `Schema` builder with structural lint passes via `Schema::lint`.
//! - Schema-time validation via `ValidSchema::validate` returning a `ValidValues` proof-token.
//! - Runtime expression resolution via `ValidValues::resolve` returning a `ResolvedValues`
//!   proof-token.
//! - Strongly-typed error and path types.
//!
//! # Quick start
//!
//! ```rust
//! use nebula_schema::{Field, FieldValues, Schema, field_key};
//! use serde_json::json;
//!
//! let schema = Schema::builder()
//!     .add(Field::string(field_key!("name")).required())
//!     .add(Field::number(field_key!("age")))
//!     .build()
//!     .expect("schema is valid");
//!
//! let values = FieldValues::from_json(json!({"name": "Alice", "age": 30})).unwrap();
//! let valid = schema.validate(&values).expect("values are valid");
//!
//! assert_eq!(valid.warnings().len(), 0);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// [`nebula_validator::RuleContext`] adapters backed by [`FieldValues`].
pub(crate) mod context;
/// Error types for schema operations.
pub mod error;
/// Expression wrapper and [`ExpressionContext`] trait.
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
/// Common imports for schema-definition code.
pub mod prelude;
/// Top-level schema aggregate.
pub mod schema;
/// Value transformer definitions.
pub mod transformer;
/// Validated schema proof-tokens.
pub mod validated;
/// Runtime value wrappers and wire-format helpers.
pub mod value;
/// Typed widget hints by field family.
pub mod widget;

pub use error::{
    STANDARD_CODES, Severity, ValidationError, ValidationErrorBuilder, ValidationReport,
};
pub use expression::{Expression, ExpressionAst, ExpressionContext};
pub use field::{
    BooleanField, CodeField, ComputedField, ComputedReturn, DynamicField, Field, FileField,
    ListField, ModeField, ModeVariant, NoticeField, NoticeSeverity, NumberField, ObjectField,
    SecretField, SelectField, StringField,
};
pub use input_hint::InputHint;
pub use key::FieldKey;
pub use loader::{
    Loader, LoaderContext, LoaderFuture, LoaderRegistry, LoaderResult, OptionLoader, RecordLoader,
};
pub use mode::{ExpressionMode, RequiredMode, VisibilityMode};
pub use nebula_schema_macros::field_key;
pub use option::SelectOption;
pub use path::{FieldPath, PathSegment};
pub use schema::{Schema, SchemaBuilder};
pub use transformer::Transformer;
pub use validated::{FieldHandle, ResolvedValues, SchemaFlags, ValidSchema, ValidValues};
pub use value::{EXPRESSION_KEY, FieldValue, FieldValues};
pub use widget::{
    BooleanWidget, CodeWidget, ListWidget, NumberWidget, ObjectWidget, SecretWidget, SelectWidget,
    StringWidget,
};

/// Schema wire-format version emitted in serialized output (Phase 2+ plugins read this).
pub const SCHEMA_WIRE_VERSION: u16 = 1;
