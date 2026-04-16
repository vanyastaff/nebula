//! Unique identifiers for Nebula entities.
//!
//! All ID types use prefixed ULIDs (Stripe-style): `exe_01J9ABCDEF...`.
//! Convention: `FooId` = system-generated ULID, `FooKey` = author-defined string.

mod types;

// Re-export ULID parse error for consumers
pub use domain_key::UlidParseError;
pub use types::*;
