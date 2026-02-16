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
//! ```

// ============================================================================
// FOUNDATION: Core traits, errors, metadata
// ============================================================================

pub use crate::foundation::{
    AsValidatable, ErrorSeverity, Validate, ValidateExt, ValidationComplexity, ValidationError,
    ValidationErrors, ValidatorMetadata,
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
    And, Each, Field, FieldValidateExt, Lazy, Not, Optional, Or, Unless, When, WithCode,
    WithMessage, and, each, each_fail_fast, field, lazy, named_field, not, optional, or, unless,
    when, with_code, with_message,
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
