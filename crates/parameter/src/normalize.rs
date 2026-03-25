//! Default and mode normalization helpers.
//!
//! This module will be fully rewritten in Task 10 to work with
//! [`Parameter`](crate::parameter::Parameter) and
//! [`ParameterCollection`](crate::collection::ParameterCollection).
//!
//! Currently provides a stub that returns the input unchanged.

use crate::parameter::Parameter;
use crate::values::ParameterValues;

/// Maximum recursion depth for nested normalization.
#[allow(dead_code)]
const MAX_NORMALIZE_DEPTH: u8 = 16;

/// Applies schema defaults to `values` for each parameter in `parameters`.
///
/// Existing user-provided values are preserved. Missing parameters are
/// materialized from `default` metadata and mode default variants.
///
/// **Stub**: returns a clone of `values`. Will be fully implemented in Task 10.
#[must_use]
pub fn normalize_parameters(
    _parameters: &[Parameter],
    values: &ParameterValues,
) -> ParameterValues {
    values.clone()
}
