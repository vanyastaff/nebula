//! Static validation engine for parameter schemas.
//!
//! This module will be fully rewritten in Task 9 to work with
//! [`Parameter`](crate::parameter::Parameter) and
//! [`ParameterCollection`](crate::collection::ParameterCollection).
//!
//! Currently provides stub implementations that compile and pass through.

use crate::error::ParameterError;
use crate::parameter::Parameter;
use crate::profile::ValidationProfile;
use crate::report::ValidationReport;
use crate::values::ParameterValues;

/// Validates `parameters` against `values` using strict defaults.
///
/// # Errors
///
/// Returns a non-empty [`Vec`] of [`ParameterError`] when any parameter fails.
pub fn validate_parameters(
    parameters: &[Parameter],
    values: &ParameterValues,
) -> Result<(), Vec<ParameterError>> {
    let report = validate_with_profile(parameters, values, ValidationProfile::Strict);
    if report.errors.is_empty() {
        Ok(())
    } else {
        Err(report.errors)
    }
}

/// Validates `parameters` against `values` under the given [`ValidationProfile`].
///
/// Returns a [`ValidationReport`] that separates hard errors from warnings.
///
/// **Stub**: returns an empty report. Will be fully implemented in Task 9.
#[must_use]
pub fn validate_with_profile(
    _parameters: &[Parameter],
    _values: &ParameterValues,
    _profile: ValidationProfile,
) -> ValidationReport {
    ValidationReport::default()
}
