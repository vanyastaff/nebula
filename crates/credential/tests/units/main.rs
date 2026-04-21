//! Unit tests for nebula-credential
//!
//! Tests individual components in isolation:
//! - Encryption and cryptographic operations
//! - Validation logic for credentials
//! - Error handling and error types

mod error_tests;
// `pending_lifecycle_tests` moved to `crates/engine/tests/credential_pending_lifecycle_tests.rs`
// (P8 migration: runtime executor ownership moved to nebula-engine).
mod scheme_roundtrip_tests;
mod validation_tests;
