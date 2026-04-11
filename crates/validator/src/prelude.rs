//! Prelude module for convenient imports.
//!
//! Provides a single `use nebula_validator::prelude::*;` import that brings
//! in all commonly needed traits, types, validators, and combinators.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // Extension method style - read left-to-right
//! "hello".validate_with(&min_length(3))?;
//! 42.validate_with(&min(10))?;
//!
//! // Direct method style - traditional
//! min_length(3).validate("hello")?;
//!
//! // Composition with combinators
//! let validator = min_length(3).and(max_length(20));
//! "hello".validate_with(&validator)?;
//! ```

#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use crate::validators::*;
pub use crate::{
    combinators::{
        AllOf, AnyOf, CollectJsonFields, CollectionNested, Each, Field, JsonField, MultiField,
        NestedValidate, OptionalNested, SelfValidating, all_of, and, any_of, collect_json_fields,
        collection_nested, each, field, json_field, json_field_optional, named_field,
        nested_validator, not, optional_nested, or,
    },
    engine::{ExecutionMode, validate_rules},
    error::ValidatorError,
    foundation::{
        And, AnyValidator, AsValidatable, ErrorSeverity, FieldPath, Not, Or, Validatable, Validate,
        ValidateExt, ValidationError, ValidationErrors, ValidationMode, When,
    },
    proof::Validated,
    rule::Rule,
};
