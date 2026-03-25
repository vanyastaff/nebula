//! Runtime-facing types for v2 parameter handling.

pub use crate::error::ParameterError;
pub use crate::values::{ModeValueRef, ParameterValue, ParameterValues};

// Backward compat
#[deprecated(note = "renamed to ParameterValue")]
pub use crate::values::ParameterValue as FieldValue;
#[deprecated(note = "renamed to ParameterValues")]
pub use crate::values::ParameterValues as FieldValues;

/// Schema-bound validated values view.
///
/// Cannot be constructed outside the crate — only produced by
/// [`Schema::validate`](crate::collection::ParameterCollection::validate) or
/// [`Schema::validate_with_profile`](crate::collection::ParameterCollection::validate_with_profile).
#[derive(Debug, Clone)]
pub struct ValidatedValues {
    values: ParameterValues,
}

impl ValidatedValues {
    /// Creates a validated wrapper from runtime values.
    ///
    /// Not publicly constructible — use [`Schema::validate`](crate::collection::ParameterCollection::validate).
    pub(crate) fn new(values: ParameterValues) -> Self {
        Self { values }
    }

    /// Accesses the underlying runtime values.
    #[must_use]
    pub fn raw(&self) -> &ParameterValues {
        &self.values
    }

    /// Consumes the wrapper and returns the raw values.
    #[must_use]
    pub fn into_inner(self) -> ParameterValues {
        self.values
    }
}
