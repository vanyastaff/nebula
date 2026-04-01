//! Shared support utilities for Nebula proc-macro crates.
//!
//! This crate provides attribute parsing, diagnostics, utility functions,
//! and validation codegen helpers used across all `nebula-*-macros` crates.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Attribute parsing utilities.
pub mod attrs;
/// Diagnostic helpers for compile errors.
pub mod diag;
/// General proc-macro utility functions.
pub mod utils;
/// Code generation helpers for validator/config derives.
pub mod validation_codegen;
