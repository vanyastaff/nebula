//! Parameter schema system for Nebula workflow nodes.
//!
//! ## Quick Start
//!
//! ```ignore
//! use nebula_parameter::prelude::*;
//!
//! let params = ParameterCollection::new()
//!     .add(Parameter::string("api_key").label("API Key").required().secret())
//!     .add(Parameter::integer("timeout_ms").label("Timeout (ms)"));
//! ```
//!
//! ## Core Types
//!
//! - [`Parameter`] — Single parameter definition with fluent builder
//! - [`ParameterType`] — Type-specific configuration (19 variants)
//! - [`ParameterCollection`] — Ordered collection of parameters
//! - [`ParameterValues`] — Runtime key-value map with typed accessors
//! - [`Condition`] — Declarative predicates for field visibility/required logic

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// ── Public modules ──────────────────────────────────────────────────────────
/// Parameter collection container.
pub mod collection;
/// Declarative conditions for field visibility and required logic.
pub mod conditions;
/// Object display mode.
pub mod display_mode;
/// Error types for parameter operations.
pub mod error;
/// Typed field definitions for the Filter condition builder.
pub mod filter_field;
/// Schema lint diagnostics.
pub mod lint;
/// Inline loader types for select, dynamic, and filter fields.
pub mod loader;
/// Paginated loader result.
pub mod loader_result;
/// Default and mode normalization helpers.
pub mod normalize;
/// Notice severity levels.
pub mod notice;
/// Option models for select parameters.
pub mod option;
/// Parameter definition with fluent builder.
pub mod parameter;
/// Type-specific parameter configuration.
pub mod parameter_type;
/// Typed parameter path references.
pub mod path;
/// Validation profile selection.
pub mod profile;
/// Validation output report.
pub mod report;
/// Declarative validation rules.
pub mod rules;
/// Runtime wrappers around validated parameter values.
pub mod runtime;
/// Supporting spec types.
pub mod spec;
/// Declarative value transformers.
pub mod transformer;
/// Static validation engine.
pub mod validate;
/// Runtime parameter value container and typed accessors.
pub mod values;

// ── Top-level re-exports ────────────────────────────────────────────────────
pub use collection::ParameterCollection;
pub use conditions::Condition;
pub use display_mode::{ComputedReturn, DisplayMode};
pub use error::ParameterError;
pub use filter_field::{FilterField, FilterFieldType};
pub use loader::{FilterFieldLoader, LoaderContext, LoaderError, OptionLoader, RecordLoader};
pub use loader_result::LoaderResult;
pub use notice::NoticeSeverity;
pub use option::SelectOption;
pub use parameter::Parameter;
pub use parameter_type::ParameterType;
pub use path::ParameterPath;
pub use profile::ValidationProfile;
pub use report::ValidationReport;
pub use rules::Rule;
pub use runtime::ValidatedValues;
pub use spec::{
    FieldSpec, FieldSpecConvertError, FilterCombinator, FilterExpr, FilterGroup, FilterOp,
    FilterRule,
};
pub use transformer::Transformer;
pub use values::{ModeValueRef, ParameterValue, ParameterValues};

/// Common imports for working with parameters.
pub mod prelude {
    pub use crate::collection::ParameterCollection;
    pub use crate::conditions::Condition;
    pub use crate::display_mode::{ComputedReturn, DisplayMode};
    pub use crate::error::ParameterError;
    pub use crate::filter_field::{FilterField, FilterFieldType};
    pub use crate::loader::{
        FilterFieldLoader, LoaderContext, LoaderError, OptionLoader, RecordLoader,
    };
    pub use crate::loader_result::LoaderResult;
    pub use crate::notice::NoticeSeverity;
    pub use crate::option::SelectOption;
    pub use crate::parameter::Parameter;
    pub use crate::parameter_type::ParameterType;
    pub use crate::path::ParameterPath;
    pub use crate::profile::ValidationProfile;
    pub use crate::report::ValidationReport;
    pub use crate::rules::Rule;
    pub use crate::runtime::ValidatedValues;
    pub use crate::spec::{
        FieldSpec, FieldSpecConvertError, FilterCombinator, FilterExpr, FilterGroup, FilterOp,
        FilterRule,
    };
    pub use crate::transformer::Transformer;
    pub use crate::values::{ModeValueRef, ParameterValue, ParameterValues};
}
