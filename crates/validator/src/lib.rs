//! # nebula-validator
//!
//! Validation rules engine for the Nebula workflow engine. Provides two complementary
//! surfaces: composable programmatic validators via the [`foundation::Validate`] trait,
//! and a JSON-serializable [`Rule`] enum for declarative schema-field constraints.
//!
//! **Role:** Validation Rules Engine + Declarative Rule. See `crates/validator/README.md`.
//!
//! **Canon:** §3.5 (schema system), §4.5 (proof-token pipeline).
//!
//! **Maturity:** `frontier` — programmatic validator API is stable; [`Rule`] wire format
//! will change in the pending sum-of-sums refactor (see
//! `docs/superpowers/specs/2026-04-17-nebula-validator-rule-refactor-design.md`).
//!
//! ## Core Types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`foundation::Validate`] | Core trait every validator implements |
//! | [`foundation::ValidateExt`] | Combinator methods (`.and()`, `.or()`, `.not()`) |
//! | [`proof::Validated`] | Proof-token certifying a value passed validation |
//! | [`foundation::ValidationError`] | Structured error (80 bytes, `Cow`-based) |
//! | [`Rule`] | Unified declarative rule (value, predicate, combinator, deferred) |
//! | [`ExecutionMode`] | Controls which categories run (`StaticOnly`, `Deferred`, `Full`) |
//! | [`ValidatorError`] | Crate-level operational error type |
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
//! // Proof token: validate once, carry the guarantee in the type system
//! let name: Validated<String> = min_length(3).validate_into("alice".to_string())?;
//! // fn process(name: Validated<String>) — compiler enforces the check happened
//! ```
//!
//! ## Non-goals
//!
//! Not a schema system (`nebula-schema`), not an expression evaluator (`nebula-expression`),
//! not a resilience pipeline (`nebula-resilience`).

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
pub use rule::{Rule, RuleContext};

// `regex` is re-exported so code emitted by `#[derive(Validator)]` can
// reference `nebula_validator::__private::regex` without requiring users to
// add a direct `regex` dependency.
#[doc(hidden)]
pub mod __private {
    pub use regex;
}
