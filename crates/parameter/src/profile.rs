//! Validation profile selection.
//!
//! Controls how unknown fields are treated during schema validation.

/// Controls the strictness of schema validation, specifically regarding
/// unknown fields present in the input values map.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ValidationProfile {
    /// Unknown fields are hard errors (default).
    ///
    /// Any key in the values map that is not defined by the schema
    /// causes validation to fail.
    #[default]
    Strict,
    /// Unknown fields produce warnings, not errors.
    ///
    /// Validation succeeds but the [`crate::report::ValidationReport`]
    /// carries a warning for each unrecognised key.
    Warn,
    /// Unknown fields are silently ignored.
    ///
    /// The values map may contain arbitrary extra keys without any diagnostic.
    Permissive,
}
