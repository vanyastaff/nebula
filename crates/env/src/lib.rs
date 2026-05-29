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
//! Enable the `testing` feature for [`testing::EnvGuard`], an RAII guard that
//! serializes env mutation behind a process-global lock and restores prior
//! values on drop.

mod error;
mod reader;

pub use error::EnvError;
pub use reader::{flag, flag_or, list, parse, parse_or, var, var_opt};

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests;
