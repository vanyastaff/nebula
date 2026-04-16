//! Validation mode — controls error accumulation behavior.

/// Controls error accumulation behavior in composite validators.
///
/// Determines whether a validator stops on the first error or collects
/// all errors before returning.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::foundation::ValidationMode;
///
/// // Default: collect all errors
/// assert_eq!(ValidationMode::default(), ValidationMode::CollectAll);
///
/// // Fail fast: stop on first error
/// let mode = ValidationMode::FailFast;
/// assert!(mode.is_fail_fast());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ValidationMode {
    /// Stop on the first validation error (short-circuit).
    ///
    /// Use when you only need to know whether validation passed,
    /// or when performance is critical and you don't need all errors.
    FailFast,

    /// Collect all validation errors before returning (default).
    ///
    /// Use when you want to report all problems at once (e.g., form validation).
    #[default]
    CollectAll,
}

impl ValidationMode {
    /// Returns `true` if this mode stops on the first error.
    #[inline]
    #[must_use]
    pub fn is_fail_fast(self) -> bool {
        matches!(self, Self::FailFast)
    }

    /// Returns `true` if this mode collects all errors.
    #[inline]
    #[must_use]
    pub fn is_collect_all(self) -> bool {
        matches!(self, Self::CollectAll)
    }
}
