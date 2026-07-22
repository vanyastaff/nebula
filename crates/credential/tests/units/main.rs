//! Unit tests for nebula-credential
//!
//! Tests individual components in isolation:
//! - Encryption and cryptographic operations
//! - Validation logic for credentials
//! - Error handling and error types

mod error_tests;
// `pending_lifecycle_tests` moved to `crates/engine/tests/credential_pending_lifecycle_tests.rs`
// as a cross-crate engine-bridge integration test; the runtime executor now
// lives in `nebula-credential` per ADR-0092.
mod scheme_roundtrip_tests;
mod validation_tests;
