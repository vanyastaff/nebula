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
//! let mut values = ParameterValues::new();
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
//! - [`ParameterValues`] — Runtime key-value map with typed accessors
//! - [`providers::ProviderRegistry`] — Dynamic provider registry

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// ── Public modules ───────────────────────────────────────────────────────────
/// Error types for parameter operations.
pub mod error;
/// Runtime parameter value container and typed accessors.
pub mod values;
/// Option models shared by select-like fields.
pub mod option;
/// Dynamic provider contracts and registry.
pub mod providers;
/// Runtime wrappers around validated parameter values.
pub mod runtime;
/// Canonical v2 schema model: [`Schema`], [`UiElement`], [`Severity`], [`Group`].
pub mod schema;
/// Canonical v2 schema field variants.
pub mod field;
/// Shared field metadata.
pub mod metadata;
/// Declarative validation rules.
pub mod rules;
/// Declarative conditions for field visibility and required logic.
pub mod conditions;
/// Supporting spec types: [`ModeVariant`], [`FieldSpec`], [`PredicateExpr`], etc.
pub mod spec;
/// Static validation engine.
pub mod validate;
/// Default and mode normalization helpers.
pub mod normalize;
/// Validation output report.
pub mod report;
/// Validation profile selection.
pub mod profile;
/// Schema lint diagnostics.
pub mod lint;

// ── Top-level re-exports ─────────────────────────────────────────────────────
pub use conditions::Condition;
pub use error::ParameterError;
pub use field::Field;
pub use metadata::FieldMetadata;
pub use option::{OptionSource, SelectOption};
pub use profile::ValidationProfile;
pub use providers::{
    DynamicProviderEnvelope, DynamicRecordProvider, DynamicResponseKind, OptionProvider,
    ProviderError, ProviderRegistry, ProviderRequest,
};
pub use report::ValidationReport;
pub use rules::Rule;
pub use runtime::{ModeValueRef, ParameterValue, ParameterValues, ValidatedValues};
pub use schema::{Group, Schema, Severity, UiElement};
pub use spec::{
    DynamicFieldSpec, DynamicRecordMode, FieldSpec, ModeVariant, PredicateCombinator, PredicateExpr,
    PredicateGroup, PredicateOp, PredicateRule, UnknownFieldPolicy,
};

/// Common imports for working with parameters.
pub mod prelude {
    //! Common imports for parameter authors.

    pub use crate::conditions::Condition;
    pub use crate::error::ParameterError;
    pub use crate::field::Field;
    pub use crate::metadata::FieldMetadata;
    pub use crate::option::{OptionSource, SelectOption};
    pub use crate::profile::ValidationProfile;
    pub use crate::providers::{
        DynamicProviderEnvelope, DynamicRecordProvider, DynamicResponseKind, OptionProvider,
        ProviderError, ProviderRegistry, ProviderRequest,
    };
    pub use crate::report::ValidationReport;
    pub use crate::rules::Rule;
    pub use crate::runtime::{ModeValueRef, ParameterValue, ParameterValues, ValidatedValues};
    pub use crate::schema::{Group, Schema, Severity, UiElement};
    pub use crate::spec::{
        DynamicFieldSpec, DynamicRecordMode, FieldSpec, ModeVariant, PredicateCombinator,
        PredicateExpr, PredicateGroup, PredicateOp, PredicateRule, UnknownFieldPolicy,
    };
}

