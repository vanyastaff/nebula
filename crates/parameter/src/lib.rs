//! Parameter definition system for Nebula workflow nodes.
//!
//! This crate provides a type-safe schema layer for declaring workflow node inputs.
//! Parameters are JSON-serializable, support validation rules, conditional display,
//! and recursive container types (Object, List, Mode).
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_parameter::prelude::*;
//!
//! // Create parameters with builder pattern
//! let text = TextParameter::new("api_key", "API Key")
//!     .required()
//!     .sensitive()
//!     .min_length(10)
//!     .placeholder("Enter your API key");
//!
//! let number = NumberParameter::new("timeout", "Timeout (seconds)")
//!     .default_value(30.0)
//!     .range(1.0, 300.0);
//!
//! // Build a collection
//! let collection = ParameterCollection::new()
//!     .with(ParameterDef::Text(text))
//!     .with(ParameterDef::Number(number));
//!
//! // Validate runtime values
//! let mut values = ParameterValues::new();
//! values.set("api_key", "secret123456".into());
//! values.set("timeout", 30.0.into());
//!
//! collection.validate(&values).unwrap();
//! ```
//!
//! ## Core Types
//!
//! - [`ParameterDef`] — Tagged enum of all 19 parameter types
//! - [`ParameterCollection`] — Ordered schema with validation pipeline
//! - [`ParameterValues`] — Runtime key-value map with type accessors
//! - [`ValidationRule`] — Declarative constraint schema
//! - [`ParameterError`] — Comprehensive error type with codes and categories

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod collection;
pub mod common;
pub mod def;
pub mod display;
pub mod error;
pub mod kind;
#[macro_use]
pub mod macros;
pub mod metadata;
pub mod option;
pub mod subtype;
pub mod typed;
pub mod types;
pub mod validation;
pub mod values;

pub mod prelude {
    //! Common imports for working with parameters.

    pub use crate::collection::ParameterCollection;
    pub use crate::common::ParameterType;
    pub use crate::def::ParameterDef;
    pub use crate::display::{DisplayCondition, DisplayContext, DisplayRuleSet, ParameterDisplay};
    pub use crate::error::ParameterError;
    pub use crate::kind::{ParameterCapability, ParameterKind};
    pub use crate::metadata::ParameterMetadata;
    pub use crate::option::{OptionsSource, SelectOption};
    pub use crate::subtype::{BooleanSubtype, NumberSubtype, TextSubtype};
    pub use crate::validation::ValidationRule;
    pub use crate::values::ParameterValues;

    pub use crate::types::*;

    /// Typed trait-based parameter API prelude.
    pub mod typed {
        pub use crate::typed::prelude::*;
    }
}
