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

// ValidationError (152 bytes) is the fundamental error type for all validators —
// boxing it would add indirection to every validation call for no practical benefit.
#![allow(clippy::result_large_err)]
// Deep combinator nesting (And<Or<Not<...>, ...>, ...>) produces complex types
// that are inherent to the type-safe combinator architecture.
#![allow(clippy::type_complexity)]

pub mod combinators;
pub mod foundation;
mod macros;
pub mod prelude;
pub mod validators;
