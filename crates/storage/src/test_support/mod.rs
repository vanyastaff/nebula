//! Test infrastructure for `nebula-storage`.
//!
//! Provides ephemeral SQLite in-memory databases and fixture factories
//! for writing repository tests. Gated behind `#[cfg(test)]`.

mod fixtures;
mod harness;

pub use fixtures::*;
pub use harness::*;
