//! # nebula-validator
//!
//! A composable, type-safe validation framework for the Nebula workflow engine.
//!
//! This crate provides two complementary validation approaches:
//!
//! - **Programmatic validators** — composable Rust types using the
//!   [`Validate`](foundation::Validate) trait with `.and()`, `.or()`, `.not()` combinators.
//! - **Declarative rules** — a unified [`Rule`] enum that can be serialized to/from JSON and
//!   evaluated at runtime for value validation, context predicates, and logical combinators.
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
//! | [`Rule`] | Unified declarative rule (value, predicate, combinator) |
//! | [`ExecutionMode`] | Controls which rule categories run (`StaticOnly`, `Deferred`, `Full`) |
//! | [`ValidatorError`] | Crate-level operational error type |
//!
//! ## Declarative Rules
//!
//! The [`Rule`] enum is the single source of truth for declarative validation.
//! Rules are JSON-serializable and cover four categories:
//!
//! ```rust
//! use nebula_validator::{ExecutionMode, Rule, validate_rules};
//! use serde_json::json;
//!
//! // Value validation — checks a single JSON value
//! let rule = Rule::min_length(3);
//! assert!(
//!     validate_rules(
//!         &json!("alice"),
//!         std::slice::from_ref(&rule),
//!         ExecutionMode::StaticOnly
//!     )
//!     .is_ok()
//! );
//!
//! // Logical combinator — compose rules
//! let rule = Rule::all([Rule::min_length(3), Rule::max_length(20)]);
//! assert!(
//!     validate_rules(
//!         &json!("hello"),
//!         std::slice::from_ref(&rule),
//!         ExecutionMode::StaticOnly
//!     )
//!     .is_ok()
//! );
//!
//! // Engine — batch-validate with execution mode
//! let rules = vec![Rule::min_length(3), Rule::pattern("^[a-z]+$")];
//! assert!(validate_rules(&json!("alice"), &rules, ExecutionMode::StaticOnly).is_ok());
//! ```
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
//!
//! ## Module Overview
//!
//! | Module | Contents |
//! |--------|----------|
//! | [`foundation`] | Core traits, errors, type-erased validators |
//! | [`validators`] | Built-in validator implementations |
//! | [`combinators`] | Composition types (`.and()`, `.or()`, [`.not()`](combinators::not())) |
//! | [`rule`] | Unified [`Rule`] enum for declarative validation |
//! | [`engine`] | [`validate_rules`] batch evaluation with [`ExecutionMode`] |
//! | [`proof`] | [`Validated<T>`](proof::Validated) proof tokens |
//! | [`error`] | Crate-level [`ValidatorError`] |

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
pub use engine::{ExecutionMode, validate_rules};
pub use error::ValidatorError;
#[cfg(feature = "derive")]
pub use nebula_validator_macros::Validator;
pub use proof::Validated;
pub use rule::{
    DeferredRule, Logic, Predicate, PredicateContext, Rule, RuleContext, RuleKind, ValueRule,
};

// `regex` is re-exported so code emitted by `#[derive(Validator)]` can
// reference `nebula_validator::__private::regex` without requiring users to
// add a direct `regex` dependency.
#[doc(hidden)]
pub mod __private {
    pub use regex;
}
