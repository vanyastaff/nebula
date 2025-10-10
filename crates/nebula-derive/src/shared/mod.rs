//! Shared infrastructure for all derive macros
//!
//! This module provides reusable components for:
//! - `#[derive(Validator)]`
//! - `#[derive(Action)]` (future)
//! - `#[derive(Resource)]` (future)
//! - `#[derive(Parameter)]` (future)
//!
//! # Modules
//!
//! - [`attrs`] - Attribute parsing utilities
//! - [`codegen`] - Code generation utilities
//! - [`types`] - Universal type system and detection
//! - [`validation`] - Input validation helpers
//!
//! # Note on Dead Code
//!
//! Many functions in these modules are not yet used but are part of the planned
//! infrastructure for future derive macros (Parameter, Action, Resource).
//! They are kept to maintain a complete API surface.

// Allow dead code in shared infrastructure - will be used by future derive macros
#![allow(dead_code)]

pub(crate) mod attrs;
pub(crate) mod codegen;
pub(crate) mod types;
pub(crate) mod validation;
