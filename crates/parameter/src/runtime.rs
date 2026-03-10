//! Runtime-facing types for v2 parameter handling.

pub use crate::error::ParameterError;
pub use crate::values::{ModeValueRef, FieldValue, ParameterValues};

/// Schema-bound validated values view.
#[derive(Debug, Clone)]
pub struct ValidatedValues {
    values: ParameterValues,
}

impl ValidatedValues {
    /// Creates a validated wrapper from runtime values.
    #[must_use]
    pub fn new(values: ParameterValues) -> Self {
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
