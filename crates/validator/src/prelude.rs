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
//! let username = min_length(3).and(max_length(20)).and(alphanumeric());
//! let age = in_range(18, 100);
//! let tags = min_size::<String>(1).and(max_size::<String>(10));
//!
//! // New: use all_of for cleaner composition
//! let validator = all_of([min_length(3), max_length(20)]);
//!
//! // New: use AnyValidator for type erasure
//! let validators: Vec<AnyValidator<str>> = vec![
//!     AnyValidator::new(min_length(3)),
//!     AnyValidator::new(email()),
//! ];
//! ```

// ============================================================================
// FOUNDATION: Core traits, errors, type erasure
// ============================================================================

pub use crate::foundation::{
    AnyValidator, AsValidatable, ErrorSeverity, Validate, ValidateExt, ValidationError,
    ValidationErrors, ValidatorFor,
};

// ============================================================================
// VALIDATORS: All built-in validators
// ============================================================================

#[allow(clippy::wildcard_imports, ambiguous_glob_reexports)]
pub use crate::validators::*;

// ============================================================================
// COMBINATORS: Composition functions and types
// ============================================================================

pub use crate::combinators::{
    AllOf, And, AnyOf, Each, Field, FieldValidateExt, Lazy, Not, Optional, Or, Unless, When,
    WithCode, WithMessage, all_of, and, any_of, each, each_fail_fast, field, lazy, named_field,
    not, optional, or, unless, when, with_code, with_message,
};

// ============================================================================
// SERDE-GATED: JSON field validators
// ============================================================================

#[cfg(feature = "serde")]
pub use crate::combinators::{JsonField, json_field, json_field_optional};

// ============================================================================
// CACHING-GATED: Memoized validators
// ============================================================================

#[cfg(feature = "caching")]
pub use crate::combinators::{Cached, cached};
