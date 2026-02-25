//! Boolean validators
//!
//! This module provides validators for boolean values.
//!
//! # Validators
//!
//! - [`IsTrue`] - Validates that a boolean is `true`
//! - [`IsFalse`] - Validates that a boolean is `false`
//!
//! # Const Validators
//!
//! For zero-cost usage, const validators are available:
//! - [`IS_TRUE`] - Const instance of `IsTrue`
//! - [`IS_FALSE`] - Const instance of `IsFalse`
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // Validate that a value is true
//! let validator = is_true();
//! assert!(validator.validate(&true).is_ok());
//! assert!(validator.validate(&false).is_err());
//!
//! // Or use the const validator directly
//! assert!(IS_TRUE.validate(&true).is_ok());
//!
//! // Validate that a value is false
//! let validator = is_false();
//! assert!(validator.validate(&false).is_ok());
//! assert!(validator.validate(&true).is_err());
//! ```

use crate::foundation::ValidationError;

crate::validator! {
    /// Validates that a boolean value is `true`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::validators::is_true;
    /// use nebula_validator::foundation::Validate;
    ///
    /// let validator = is_true();
    /// assert!(validator.validate(&true).is_ok());
    /// assert!(validator.validate(&false).is_err());
    /// ```
    pub IsTrue for bool;
    rule(input) { *input }
    error(input) { ValidationError::new("is_true", "Value must be true") }
    fn is_true();
}

/// Const instance of [`IsTrue`] validator for zero-cost usage.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::IS_TRUE;
/// use nebula_validator::foundation::Validate;
///
/// assert!(IS_TRUE.validate(&true).is_ok());
/// ```
pub const IS_TRUE: IsTrue = IsTrue;

crate::validator! {
    /// Validates that a boolean value is `false`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::validators::is_false;
    /// use nebula_validator::foundation::Validate;
    ///
    /// let validator = is_false();
    /// assert!(validator.validate(&false).is_ok());
    /// assert!(validator.validate(&true).is_err());
    /// ```
    pub IsFalse for bool;
    rule(input) { !*input }
    error(input) { ValidationError::new("is_false", "Value must be false") }
    fn is_false();
}

/// Const instance of [`IsFalse`] validator for zero-cost usage.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::IS_FALSE;
/// use nebula_validator::foundation::Validate;
///
/// assert!(IS_FALSE.validate(&false).is_ok());
/// ```
pub const IS_FALSE: IsFalse = IsFalse;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;

    #[test]
    fn test_is_true() {
        assert!(is_true().validate(&true).is_ok());
        assert!(is_true().validate(&false).is_err());
    }

    #[test]
    fn test_is_false() {
        assert!(is_false().validate(&false).is_ok());
        assert!(is_false().validate(&true).is_err());
    }

    #[test]
    fn test_const_is_true() {
        assert!(IS_TRUE.validate(&true).is_ok());
        assert!(IS_TRUE.validate(&false).is_err());
    }

    #[test]
    fn test_const_is_false() {
        assert!(IS_FALSE.validate(&false).is_ok());
        assert!(IS_FALSE.validate(&true).is_err());
    }
}
