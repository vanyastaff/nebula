//! Validation output report.

use crate::error::ParameterError;

/// The result of running schema validation with a [`crate::profile::ValidationProfile`].
///
/// Hard errors must be resolved before the values can be accepted.
/// Warnings are informational and do not block acceptance.
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    /// Hard validation failures that block accepting the values.
    pub errors: Vec<ParameterError>,
    /// Non-blocking diagnostic notices (e.g. unknown fields under `Warn` profile).
    pub warnings: Vec<ParameterError>,
}

impl ValidationReport {
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
}
