//! Boolean validators

use crate::foundation::ValidationError;

crate::validator! {
    /// Validates that a boolean value is `true`.
    pub IsTrue for bool;
    rule(input) { *input }
    error(input) { ValidationError::new("is_true", "Value must be true") }
    fn is_true();
}

crate::validator! {
    /// Validates that a boolean value is `false`.
    pub IsFalse for bool;
    rule(input) { !*input }
    error(input) { ValidationError::new("is_false", "Value must be false") }
    fn is_false();
}

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
}
