//! Unit tests for nebula-credential
//!
//! Tests individual components in isolation:
//! - Encryption and cryptographic operations
//! - Validation logic for credentials
//! - Error handling and error types

mod error_tests;
// `pending_lifecycle_tests` moved to `crates/storage/tests/credential_pending_lifecycle.rs`
// (ADR-0029 §4 / ADR-0032 — InMemoryPendingStore lives in nebula-storage).
mod scheme_roundtrip_tests;
mod validation_tests;
