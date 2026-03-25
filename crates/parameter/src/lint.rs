//! Schema-time lint diagnostics.
//!
//! Provides a static lint pass over a [`ParameterCollection`] that detects
//! structural problems independent of any runtime values: duplicate parameter
//! ids, contradictory rules, dangling references, and integrity violations.
//!
//! This module will be fully rewritten in Task 11 to work with
//! [`Parameter`](crate::parameter::Parameter) and
//! [`ParameterCollection`](crate::collection::ParameterCollection).

use crate::collection::ParameterCollection;

/// A single lint finding emitted by [`lint_collection`].
#[derive(Debug, Clone, PartialEq)]
pub struct LintDiagnostic {
    /// Dot-separated path to the offending parameter or rule.
    pub path: String,
    /// Lint severity.
    pub level: LintLevel,
    /// Human-readable description of the problem.
    pub message: String,
}

/// Severity of a [`LintDiagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    /// Non-blocking advisory notice.
    Warning,
    /// Structural error that should be fixed before deployment.
    Error,
}

/// Runs the static lint pass over `collection` and returns all diagnostics found.
///
/// This is a schema-time check -- it does not require runtime values.
///
/// **Stub**: returns an empty list. Will be fully implemented in Task 11.
#[must_use]
pub fn lint_collection(_collection: &ParameterCollection) -> Vec<LintDiagnostic> {
    Vec::new()
}
