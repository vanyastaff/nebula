//! Shared support utilities for Nebula proc-macro crates.
//!
//! This crate provides attribute parsing, diagnostics, utility functions,
//! and validation codegen helpers used across all `nebula-*-macros` crates.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Attribute parsing utilities.
pub mod attrs;
/// `CredentialRef<dyn X>` rewrite per ADR-0035 4.3 + Tech Spec 2.7.
pub mod credential_ref;
/// Diagnostic helpers for compile errors.
pub mod diag;
/// General proc-macro utility functions.
pub mod utils;
/// Code generation helpers for validator/config derives.
pub mod validation_codegen;
