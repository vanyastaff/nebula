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

pub mod attrs;
pub mod codegen;
pub mod types;
pub mod validation;
