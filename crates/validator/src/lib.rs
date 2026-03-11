//! # nebula-validator
//!
//! A composable, type-safe validation framework for the Nebula workflow engine.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // Compose validators with .and() / .or() / .not()
//! let username = min_length(3).and(max_length(20)).and(alphanumeric());
//! assert!(username.validate("alice").is_ok());
//!
//! // Proof tokens: validate once, carry proof through the system
//! let name: Validated<String> = min_length(3).validate_into("alice".to_string())?;
//! println!("Validated name: {}", name.as_ref());
//! ```
//!
//! ## Core Types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`Validate<T>`](foundation::Validate) | Core trait every validator implements |
//! | [`ValidateExt<T>`](foundation::ValidateExt) | Combinator methods (`.and()`, `.or()`, `.not()`) |
//! | [`Validated<T>`](proof::Validated) | Proof token certifying a value passed validation |
//! | [`ValidationError`](foundation::ValidationError) | Structured error (80 bytes, `Cow`-based) |
//! | [`AnyValidator<T>`](foundation::AnyValidator) | Type-erased validator for dynamic dispatch |
//! | [`ValidatorError`] | Crate-level operational error type |
//!
//! ## Creating Validators
//!
//! Use the [`validator!`] macro for zero-boilerplate validators,
//! or implement [`Validate`](foundation::Validate) manually for complex cases.
//!
//! ## Built-in Validators
//!
//! - **String**: [`MinLength`](validators::MinLength), [`MaxLength`](validators::MaxLength),
//!   [`NotEmpty`](validators::NotEmpty), [`Contains`](validators::Contains),
//!   [`Alphanumeric`](validators::Alphanumeric)
//! - **Numeric**: [`Min`](validators::Min), [`Max`](validators::Max),
//!   [`InRange`](validators::InRange)
//! - **Collection**: [`MinSize`](validators::MinSize), [`MaxSize`](validators::MaxSize)
//! - **Boolean**: [`IsTrue`](validators::IsTrue), [`IsFalse`](validators::IsFalse)
//! - **Nullable**: [`Required`](validators::Required)
//! - **Network**: [`Ipv4`](validators::Ipv4), [`Hostname`](validators::Hostname)
//! - **Temporal**: [`DateTime`](validators::DateTime), [`Uuid`](validators::Uuid)

#![forbid(unsafe_code)]
#![warn(missing_docs)]
// ValidationError (<= 80 bytes) is the fundamental error type for all validators —
// boxing it would add indirection to every validation call for no practical benefit.
#![allow(clippy::result_large_err)]
// Deep combinator nesting (And<Or<Not<...>, ...>, ...>) produces complex types
// that are inherent to the type-safe combinator architecture.
#![allow(clippy::type_complexity)]

// ── Public modules ──────────────────────────────────────────────────────────
/// Combinator types for composing validators.
pub mod combinators;
/// Field value provider trait for context-aware evaluation.
pub mod context;
/// Validation engine for declarative rules.
pub mod engine;
/// Crate-level operational error type.
pub mod error;
/// Core traits, errors, and type-erased validators.
pub mod foundation;
/// Single-import convenience module.
pub mod prelude;
/// Proof tokens that certify a value has been validated.
pub mod proof;
/// Unified declarative rule enum.
pub mod rule;
/// Built-in validators for common scenarios.
pub mod validators;

// ── Private modules ──────────────────────────────────────────────────────────
mod macros;

// ── Re-exports ───────────────────────────────────────────────────────────────
pub use context::FieldValueProvider;
pub use engine::{ExecutionMode, validate_rules};
pub use error::ValidatorError;
pub use proof::Validated;
pub use rule::Rule;
