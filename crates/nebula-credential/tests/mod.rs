//! Test organization for nebula-credential
//!
//! Structure:
//! - `units/` - Unit tests for individual components
//! - `providers/` - Tests for storage provider implementations
//! - `integration/` - End-to-end integration tests
//!
//! Legacy tests (not yet updated for Phase 2+ architecture):
//! - caching_tests, concurrency_tests, locking_tests
//! - manager_tests, registry_tests
//! These are commented out until CredentialManager/Registry are implemented

// Active test modules (Phase 2+)
mod integration;
mod providers;
mod units;

// Legacy test modules (Phase 1 - commented out until refactored)
// Uncomment when CredentialManager and CredentialRegistry are implemented
// pub mod caching_tests;
// pub mod concurrency_tests;
// pub mod locking_tests;
// pub mod manager_tests;
// pub mod registry_tests;
