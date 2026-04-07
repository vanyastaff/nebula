//! Validation output report.

use crate::error::ParameterError;
use crate::runtime::ValidatedValues;
use crate::values::ParameterValues;

/// The result of running schema validation with a [`crate::profile::ValidationProfile`].
///
/// Hard errors must be resolved before the values can be accepted.
/// Warnings are informational and do not block acceptance.
#[derive(Debug, Clone)]
pub struct ValidationReport {
    /// Hard validation failures that block accepting the values.
    pub(crate) errors: Vec<ParameterError>,
    /// Non-blocking diagnostic notices (e.g. unknown fields under `Warn` profile).
    pub(crate) warnings: Vec<ParameterError>,
    /// The values that were validated (used by `into_validated`).
    values: ParameterValues,
}

impl ValidationReport {
    /// Creates a new report from validation results and the validated values.
    pub(crate) fn new(
        errors: Vec<ParameterError>,
        warnings: Vec<ParameterError>,
        values: ParameterValues,
    ) -> Self {
        Self {
            errors,
            warnings,
            values,
        }
    }

    /// Returns a slice of hard validation errors.
    #[must_use]
    pub fn errors(&self) -> &[ParameterError] {
        &self.errors
    }

    /// Returns a slice of non-blocking warnings.
    #[must_use]
    pub fn warnings(&self) -> &[ParameterError] {
        &self.warnings
    }

    /// Adds a hard error to the report.
    pub(crate) fn push_error(&mut self, error: ParameterError) {
        self.errors.push(error);
    }

    /// Adds a warning to the report.
    pub(crate) fn push_warning(&mut self, warning: ParameterError) {
        self.warnings.push(warning);
    }

    /// Returns `true` if the report contains no hard errors.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns `true` if the report contains at least one hard error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Returns `true` if the report contains at least one warning.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Converts the report into a `Result`, discarding warnings.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the report contains one or more hard errors.
    pub fn into_result(self) -> Result<(), Vec<ParameterError>> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors)
        }
    }

    /// Extracts [`ValidatedValues`] when the report has no hard errors.
    ///
    /// # Errors
    ///
    /// Returns the report unchanged when it contains hard errors.
    pub fn into_validated(self) -> Result<ValidatedValues, Self> {
        if self.errors.is_empty() {
            Ok(ValidatedValues::new(self.values))
        } else {
            Err(self)
        }
    }
}
