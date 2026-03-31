//! Unit tests for nebula-credential
//!
//! Tests individual components in isolation:
//! - Encryption and cryptographic operations
//! - Validation logic for credentials
//! - Error handling and error types

mod encryption_tests;
mod error_tests;
mod pending_lifecycle_tests;
mod resolve_snapshot_tests;
mod thundering_herd_tests;
mod validation_tests;
