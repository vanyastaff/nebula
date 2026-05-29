//! # nebula-env
//!
//! Typed, cross-cutting environment-variable reader for the Nebula workspace.
//!
//! One parsing contract shared by every crate that reads the environment,
//! replacing the per-crate ad-hoc `std::env::var(...).unwrap_or_default().parse()`
//! patterns and the divergent bool/int helpers previously duplicated in
//! `nebula-api` and `nebula-log`. Consumers map [`EnvError`] into their own
//! typed error at the boundary; `nebula-env` itself takes no runtime
//! dependencies beyond `std` + `thiserror`.
//!
//! ## Reading
//!
//! - [`var`] / [`var_opt`] — required / optional string.
//! - [`parse`] / [`parse_or`] — any [`FromStr`](core::str::FromStr) type.
//! - [`flag`] / [`flag_or`] — boolean (`true/1/yes/on` vs `false/0/no/off`).
//! - [`list`] — whitespace/comma-delimited values, empties dropped.
//!
//! ## Testing
//!
//! Enable the `testing` feature for `testing::EnvGuard`, an RAII guard that
//! serializes env mutation behind a process-global lock and restores prior
//! values on drop. (Not an intra-doc link: the module is feature-gated, so it
//! is absent from the default doc build.)
//!
//! ## Examples
//!
//! ```
//! // Unset variables read as absent / fall back to the default — no panics.
//! assert!(nebula_env::var("NEBULA_ENV_DOCTEST_UNSET").is_err());
//! assert_eq!(nebula_env::var_opt("NEBULA_ENV_DOCTEST_UNSET"), Ok(None));
//! assert_eq!(nebula_env::parse_or("NEBULA_ENV_DOCTEST_UNSET", 4u32), Ok(4));
//! assert_eq!(nebula_env::flag_or("NEBULA_ENV_DOCTEST_UNSET", true), Ok(true));
//! assert!(nebula_env::list("NEBULA_ENV_DOCTEST_UNSET").is_empty());
//! ```

// The reader core is unsafe-free; the only `unsafe` lives behind the
// feature-gated `testing` module (env mutation under edition 2024).
#![cfg_attr(not(any(test, feature = "testing")), forbid(unsafe_code))]
#![warn(missing_docs)]

mod error;
mod reader;

pub use error::EnvError;
pub use reader::{flag, flag_or, list, parse, parse_or, var, var_opt};

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests;
