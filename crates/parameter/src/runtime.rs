//! Runtime-facing types for v2 parameter handling.

pub use crate::error::ParameterError;
pub use crate::values::{FieldValue, FieldValues, ModeValueRef};

/// Schema-bound validated values view.
#[derive(Debug, Clone)]
pub struct ValidatedValues {
    values: FieldValues,
}

impl ValidatedValues {
    /// Creates a validated wrapper from runtime values.
    #[must_use]
    pub fn new(values: FieldValues) -> Self {
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
