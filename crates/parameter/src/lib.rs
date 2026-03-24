//! Parameter schema system for Nebula workflow nodes.
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_parameter::prelude::*;
//!
//! let schema = Schema::new()
//!     .field(Field::text("api_key").with_label("API Key").required().secret())
//!     .field(Field::integer("timeout_ms").with_label("Timeout (ms)"));
//!
//! let mut values = FieldValues::new();
//! values.set("api_key", "secret123456".into());
//! values.set("timeout_ms", 30_000.into());
//!
//! assert_eq!(schema.fields.len(), 2);
//! ```
//!
//! ## Core Types
//!
//! - [`Schema`] — Canonical v2 parameter schema
//! - [`Field`] — Canonical v2 schema field (all 16 variants)
//! - [`FieldValues`] — Runtime key-value map with typed accessors
//! - [`loader::OptionLoader`] / [`loader::RecordLoader`] — Inline async loaders

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// ── Public modules ───────────────────────────────────────────────────────────
/// Declarative conditions for field visibility and required logic.
pub mod conditions;
/// Error types for parameter operations.
pub mod error;
/// Canonical v2 schema field variants.
pub mod field;
/// Schema lint diagnostics.
pub mod lint;
/// Inline loader types for select and dynamic-record fields.
pub mod loader;
/// Shared field metadata.
pub mod metadata;
/// Default and mode normalization helpers.
pub mod normalize;
/// Option models shared by select-like fields.
pub mod option;
/// Validation profile selection.
pub mod profile;
/// Validation output report.
pub mod report;
/// Declarative validation rules.
pub mod rules;
/// Runtime wrappers around validated parameter values.
pub mod runtime;
/// Canonical v2 schema model: [`Schema`], [`UiElement`], [`Severity`], [`Group`].
pub mod schema;
/// Supporting spec types: [`ModeVariant`], [`FieldSpec`], `PredicateExpr`, etc.
pub mod spec;
/// Static validation engine.
pub mod validate;
/// Runtime parameter value container and typed accessors.
pub mod values;

// ── Top-level re-exports ─────────────────────────────────────────────────────
pub use conditions::Condition;
pub use error::ParameterError;
pub use field::Field;
pub use loader::{LoaderCtx, LoaderError, OptionLoader, RecordLoader};
pub use metadata::FieldMetadata;
pub use option::{OptionSource, SelectOption};
pub use profile::ValidationProfile;
pub use report::ValidationReport;
pub use rules::Rule;
pub use runtime::{FieldValue, FieldValues, ModeValueRef, ValidatedValues};
pub use schema::{Group, Schema, Severity, UiElement};
pub use spec::{
    DynamicFieldsMode, FieldSpec, FieldSpecConvertError, FilterCombinator, FilterExpr, FilterGroup,
    FilterOp, FilterRule, ModeVariant, UnknownFieldPolicy,
};

pub mod prelude {
    //! Common imports for working with parameters.

    pub use crate::conditions::Condition;
    pub use crate::error::ParameterError;
    pub use crate::field::Field;
    pub use crate::loader::{LoaderCtx, LoaderError, OptionLoader, RecordLoader};
    pub use crate::metadata::FieldMetadata;
    pub use crate::option::{OptionSource, SelectOption};
    pub use crate::profile::ValidationProfile;
    pub use crate::report::ValidationReport;
    pub use crate::rules::Rule;
    pub use crate::runtime::{FieldValue, FieldValues, ModeValueRef, ValidatedValues};
    pub use crate::schema::{Group, Schema, Severity, UiElement};
    pub use crate::spec::{
        DynamicFieldsMode, FieldSpec, FieldSpecConvertError, FilterCombinator, FilterExpr,
        FilterGroup, FilterOp, FilterRule, ModeVariant, UnknownFieldPolicy,
    };
}
