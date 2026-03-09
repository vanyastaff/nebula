//! Parameter schema system for Nebula workflow nodes.
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_parameter::schema::{Field, Schema};
//! use nebula_parameter::ParameterValues;
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
//! - [`schema::Schema`] — Canonical v2 parameter schema
//! - [`schema::Field`] — Canonical v2 schema field (all 16 variants)
//! - [`ParameterValues`] — Runtime key-value map with typed accessors
//! - [`providers::DynamicProviderEnvelope`] — Shared dynamic provider contract

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Error types for parameter operations.
pub mod error;
/// Runtime parameter value container and typed accessors.
pub mod values;

/// Option models shared by select-like fields.
pub mod option;
/// Dynamic provider contracts.
pub mod providers;
/// Runtime wrappers around validated parameter values.
pub mod runtime;
/// Canonical v2 schema model.
pub mod schema;

pub use providers::{
    DynamicProviderEnvelope, DynamicRecordProvider, DynamicResponseKind, OptionProvider,
    ProviderError, ProviderRegistry, ProviderRequest,
};
pub use runtime::{ModeValueRef, ParameterError, ParameterValue, ParameterValues, ValidatedValues};
pub use schema::{
    Condition, DynamicFieldSpec, DynamicRecordMode, Field, FieldMetadata, FieldSpec, Group,
    ModeVariant, OptionSource, PredicateCombinator, PredicateExpr, PredicateGroup, PredicateOp,
    PredicateRule, Rule, Schema, Severity, UiElement, UnknownFieldPolicy,
};

/// Common imports for working with parameters.
pub mod prelude {
    //! Common imports for parameter authors.

    pub use crate::providers::{
        DynamicProviderEnvelope, DynamicRecordProvider, DynamicResponseKind, OptionProvider,
        ProviderError, ProviderRegistry, ProviderRequest,
    };
    pub use crate::runtime::{
        ModeValueRef, ParameterError, ParameterValue, ParameterValues, ValidatedValues,
    };
    pub use crate::schema::{
        Condition, DynamicFieldSpec, DynamicRecordMode, Field, FieldMetadata, FieldSpec, Group,
        ModeVariant, OptionSource, PredicateCombinator, PredicateExpr, PredicateGroup, PredicateOp,
        PredicateRule, Rule, Schema, Severity, UiElement, UnknownFieldPolicy,
    };
}
