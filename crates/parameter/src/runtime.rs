//! Runtime-facing types for v2 parameter handling.

pub use crate::error::ParameterError;
pub use crate::values::{FieldValue, FieldValues, ModeValueRef};

/// Schema-bound validated values view.
///
/// Cannot be constructed outside the crate — only produced by
/// [`Schema::validate`](crate::schema::Schema::validate) or
/// [`Schema::validate_with_profile`](crate::schema::Schema::validate_with_profile).
#[derive(Debug, Clone)]
pub struct ValidatedValues {
    values: FieldValues,
}

impl ValidatedValues {
    /// Creates a validated wrapper from runtime values.
    ///
    /// Not publicly constructible — use [`Schema::validate`](crate::schema::Schema::validate).
    pub(crate) fn new(values: FieldValues) -> Self {
        Self { values }
    }

    /// Accesses the underlying runtime values.
    #[must_use]
    pub fn raw(&self) -> &FieldValues {
        &self.values
    }

    /// Consumes the wrapper and returns the raw values.
    #[must_use]
    pub fn into_inner(self) -> FieldValues {
        self.values
    }
}
