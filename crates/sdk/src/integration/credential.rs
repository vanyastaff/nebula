//! Credential contracts used by integration authors.
//!
//! [`BearerTokenCredential`] is the built-in bearer token a resource declares
//! as `CredentialSlot<BearerTokenCredential>`; its guard projects a
//! `SecretToken`.

pub use nebula_credential::{
    BearerTokenCredential, Credential, CredentialMetadata, ResolveResult, StaticResolveResult,
    TestFailureCode, TestResult,
};
