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
//!
//! # Conditional fields
//!
//! Use `active_when` to express "field X only appears and is only required
//! when predicate P holds" — the shorthand avoids repeating the same
//! predicate in `visible_when` and `required_when`.
//!
//! ```rust
//! use nebula_schema::prelude::*;
//! use serde_json::json;
//!
//! let schema = Schema::builder()
//!     .add(
//!         Field::select(field_key!("auth_type"))
//!             .option("api_key", "API key")
//!             .option("oauth2", "OAuth2")
//!             .required(),
//!     )
//!     .add(
//!         Field::secret(field_key!("api_key")).active_when(Rule::predicate(
//!             Predicate::eq("auth_type", json!("api_key")).unwrap(),
//!         )),
//!     )
//!     .add(
//!         Field::string(field_key!("client_id")).active_when(Rule::predicate(
//!             Predicate::eq("auth_type", json!("oauth2")).unwrap(),
//!         )),
//!     )
//!     .build()
//!     .expect("schema is valid");
//! assert_eq!(schema.fields().len(), 3);
//! ```
//!
//! # Struct-level rules (`#[schema(...)]` on `#[derive(Schema)]`)
//!
//! Attach deferred wire hooks or cross-field [`Rule`]s on the
//! whole value object — not new field types for the UI. See
//! [`SchemaBuilder::root_rule`](crate::SchemaBuilder::root_rule) and
//! [`ValidSchema::validate`](crate::ValidSchema::validate).
//!
//! ```rust
//! use nebula_schema::{Field, FieldValues, Schema, Predicate, Rule, field_key};
//! use serde_json::json;
//!
//! let schema = Schema::builder()
//!     .add(Field::string(field_key!("tier")))
//!     .root_rule(Rule::predicate(Predicate::eq("tier", json!("pro")).unwrap()))
//!     .build()
//!     .unwrap();
//!
//! assert!(
//!     schema
//!         .validate(&FieldValues::from_json(json!({"tier": "free"})).unwrap())
//!         .is_err()
//! );
//! assert!(
//!     schema
//!         .validate(&FieldValues::from_json(json!({"tier": "pro"})).unwrap())
//!         .is_ok()
//! );
//! ```
//!
//! # `#[derive(Schema)]` and `HasSchema`
//!
//! ```rust
//! use nebula_schema::{FieldValues, HasSchema, Schema};
//! use serde::Deserialize;
//! use serde_json::json;
//!
//! #[derive(Schema, Deserialize)]
//! // `Rule::custom` is deferred; `ValidSchema::validate` does not execute the
//! // expression string (engine hook). This still type-checks the value tree.
//! #[schema(custom = "engine_deferred")]
//! struct Example {
//!     name: String,
//! }
//!
//! let schema = Example::schema();
//! let values = FieldValues::from_json(json!({"name": "n"})).unwrap();
//! assert!(schema.validate(&values).is_ok());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// Allow `nebula_schema::Foo` to resolve from inside the crate too (lib unit
// tests, examples_include, internal docs). The proc-macro `field_key!` emits
// absolute `nebula_schema::FieldKey::new(..)` paths so the same call site
// works from external crates, integration tests, doctests, and lib tests.
extern crate self as nebula_schema;

/// Typed-closure builder DSL (leaf aliases + Object/List/Group composite builders).
pub mod builder;
/// [`nebula_validator::RuleContext`] adapters backed by [`FieldValues`].
pub(crate) mod context;
/// Error types for schema operations.
pub mod error;
/// Expression wrapper and [`ExpressionContext`] trait.
pub mod expression;
/// Typed field definitions and wrappers.
pub mod field;
/// Traits linking Rust types to schema definitions.
pub mod has_schema;
/// UI hints for string input rendering.
pub mod input_hint;
/// JSON Schema export (`schemars` feature).
#[cfg(feature = "schemars")]
pub mod json_schema;
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
/// Secret value types, optional KDF, and `SecretWire`.
pub mod secret;
/// Value transformer definitions.
pub mod transformer;
/// Validated schema proof-tokens.
pub mod validated;
/// Runtime value wrappers and wire-format helpers.
pub mod value;
/// Typed widget hints by field family.
pub mod widget;

pub use builder::{
    BooleanBuilder, CodeBuilder, FieldCollector, GroupBuilder, ListBuilder, NumberBuilder,
    ObjectBuilder, SecretBuilder, SelectBuilder, StringBuilder,
};
pub use error::{
    STANDARD_CODES, Severity, ValidationError, ValidationErrorBuilder, ValidationReport,
};
pub use expression::{EvalFuture, Expression, ExpressionAst, ExpressionContext};
/// Discriminated field: one of several payload shapes (auth scheme, body kind, etc.).
///
/// # JSON wire format
///
/// A mode value is a JSON object with:
///
/// - **`mode`**: the variant’s string key.
/// - **`value`** (optional): the payload for that variant.
///
/// When `value` is present, it matches the `field` you pass to [`ModeField::variant`]. The
/// shape depends on that child field:
///
/// - If the child is an [`ObjectField`], `value` is a JSON object whose keys are the nested
///   fields (each key must be a valid [`FieldKey`] string).
/// - If the child is a [`ListField`], `value` is a **JSON array** of list items. There is no
///   outer wrapper key around the list: each array element is validated with the `item`
///   schema, and that schema’s outer `Field` key (if the item is an `ObjectField`) is not
///   re-emitted as an extra wrapper in the wire.
/// - For scalar leaf families, `value` is the JSON the field type expects, for example
///   [`StringField`], [`SecretField`], [`CodeField`], [`NumberField`], [`BooleanField`],
///   [`SelectField`], and [`FileField`] (per-field options, such as `multiple`, apply as
///   usual).
///
/// Integrators and UI authors should treat a JSON object with `"mode"` and optional `"value"`
/// keys as the standard envelope, with the `value` kind determined by the selected variant’s
/// payload field type.
///
/// # Empty / no-payload variants
///
/// Variants with no user-facing data can use [`ModeField::variant_empty`], which stores a
/// hidden string placeholder under a stable key ([`ModeField::EMPTY_PLACEHOLDER_KEY`]). When
/// `value` is omitted, validators still accept the input if the only payload fields are hidden
/// and non-required.
pub use field::ModeField;
pub use field::{
    BooleanField, CodeField, ComputedField, ComputedReturn, DynamicField, Field, FileField,
    ListField, ModeVariant, NoticeField, NoticeSeverity, NumberField, ObjectField, SecretField,
    SelectField, StringField,
};
pub use has_schema::{HasSchema, HasSelectOptions};
pub use input_hint::InputHint;
#[cfg(feature = "schemars")]
pub use json_schema::JsonSchemaExportError;
pub use key::FieldKey;
pub use loader::{
    Loader, LoaderContext, LoaderFuture, LoaderRegistry, LoaderResult, OptionLoader, RecordLoader,
};
pub use mode::{ExpressionMode, RequiredMode, VisibilityMode};
pub use nebula_schema_macros::{EnumSelect, Schema, field_key};
/// Re-exported for `#[derive(Schema)]` expansion and schema authors who build
/// [`Rule`] / [`Predicate`] without importing `nebula-validator` directly.
pub use nebula_validator::{Predicate, Rule};
pub use option::SelectOption;
pub use path::{FieldPath, PathSegment};
pub use schema::{Schema, SchemaBuilder};
pub use secret::{
    KdfError, KdfParams, SECRET_REDACTED, SecretBytes, SecretString, SecretValue, SecretWire,
};
pub use transformer::Transformer;
pub use validated::{
    FieldHandle, ResolvedLookup, ResolvedValues, SchemaFlags, ValidSchema, ValidValues,
};
pub use value::{EXPRESSION_KEY, FieldValue, FieldValues};
pub use widget::{
    BooleanWidget, CodeWidget, ListWidget, NumberWidget, ObjectWidget, SecretWidget, SelectWidget,
    StringWidget,
};

/// Schema wire-format version emitted in serialized output (Phase 2+ plugins read this).
pub const SCHEMA_WIRE_VERSION: u16 = 1;

#[doc(hidden)]
pub mod __private {
    //! Re-exports used by `nebula-schema-macros`-generated code.
    //!
    //! Not part of the stable API. Do not depend on these from user code.
    pub use tracing;
}
