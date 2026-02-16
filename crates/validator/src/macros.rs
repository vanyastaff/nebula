//! Macros for creating validators
//!
//! This module provides macros that simplify validator creation and usage.
//!
//! # Available Macros
//!
//! - `validator!` - Create simple validators with minimal boilerplate
//! - `validate!` - Inline validation expressions
//! - `validator_fn!` - Create validators from functions
//!
//! # Examples
//!
//! ## Creating a validator
//!
//! ```rust,ignore
//! use nebula_validator::macros::validator;
//!
//! validator! {
//!     pub struct MinLength {
//!         min: usize
//!     }
//!     impl {
//!         fn check(input: &str, min: usize) -> bool {
//!             input.len() >= min
//!         }
//!         fn error(min: usize) -> String {
//!             format!("Must be at least {} characters", min)
//!         }
//!         const DESCRIPTION: &str = "Minimum length validator";
//!     }
//! }
//! ```
//!
//! ## Inline validation
//!
//! ```rust,ignore
//! use nebula_validator::macros::validate;
//!
//! let result = validate!(input, |s: &str| {
//!     s.len() >= 5 && s.len() <= 20
//! }, "Length must be between 5 and 20");
//! ```

// ============================================================================
// VALIDATOR MACRO
// ============================================================================

/// Creates a validator with minimal boilerplate.
///
/// This macro generates a complete validator implementation including:
/// - Struct definition
/// - Validate trait implementation
/// - Error handling
///
/// # Syntax
///
/// ```ignore
/// validator! {
///     [pub] struct ValidatorName {
///         field1: Type1,
///         field2: Type2,
///     }
///     impl {
///         fn check(input: &InputType, field1: Type1, field2: Type2) -> bool {
///             // validation logic
///         }
///         fn error(field1: Type1, field2: Type2) -> String {
///             // error message
///         }
///         const DESCRIPTION: &str = "description";
///     }
/// }
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::macros::validator;
///
/// validator! {
///     pub struct MinLength {
///         min: usize
///     }
///     impl {
///         fn check(input: &str, min: usize) -> bool {
///             input.len() >= min
///         }
///         fn error(min: usize) -> String {
///             format!("Must be at least {} characters", min)
///         }
///         const DESCRIPTION: &str = "Validates minimum string length";
///     }
/// }
///
/// let validator = MinLength { min: 5 };
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("hi").is_err());
/// ```
#[macro_export]
macro_rules! validator {
    // Main pattern: struct with fields and impl
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident: $field_ty:ty
            ),* $(,)?
        }
        impl {
            fn check($input:ident: &$input_ty:ty $(, $param:ident: $param_ty:ty)*) -> bool $check_body:block
            fn error($($error_param:ident: $error_param_ty:ty),*) -> String $error_body:block
            $(const DESCRIPTION: &str = $desc:expr;)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $vis struct $name {
            $(
                $(#[$field_meta])*
                pub $field: $field_ty,
            )*
        }

        impl $crate::foundation::Validate for $name {
            type Input = $input_ty;

            fn validate(&self, $input: &Self::Input) -> Result<(), ValidationError> {
                if Self::check($input $(, self.$param)*) {
                    Ok(())
                } else {
                    Err($crate::foundation::ValidationError::new(
                        stringify!($name),
                        Self::error($(self.$error_param),*)
                    ))
                }
            }
        }

        impl $name {
            fn check($input: &$input_ty $(, $param: $param_ty)*) -> bool $check_body
            fn error($($error_param: $error_param_ty),*) -> String $error_body
        }
    };
}

// ============================================================================
// VALIDATE MACRO
// ============================================================================

/// Inline validation with custom predicate.
///
/// This macro provides a quick way to validate a value inline without
/// creating a full validator struct.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validate;
///
/// let input = "hello";
/// validate!(input, |s: &str| s.len() >= 5, "Too short")?;
///
/// // With custom error code
/// validate!(input, |s: &str| s.contains("@"), "email_invalid", "Must contain @")?;
/// ```
#[macro_export]
macro_rules! validate {
    // Simple validation with message
    ($value:expr, $predicate:expr, $message:expr) => {{
        let value = $value;
        if $predicate(&value) {
            Ok(())
        } else {
            Err($crate::foundation::ValidationError::new(
                "validation_failed",
                $message,
            ))
        }
    }};

    // Validation with custom error code
    ($value:expr, $predicate:expr, $code:expr, $message:expr) => {{
        let value = $value;
        if $predicate(&value) {
            Ok(())
        } else {
            Err($crate::foundation::ValidationError::new($code, $message))
        }
    }};
}

// ============================================================================
// VALIDATOR_FN MACRO
// ============================================================================

/// Creates a validator from a function.
///
/// This macro wraps a validation function into a validator struct.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validator_fn;
///
/// validator_fn!(is_even, |n: &i32| *n % 2 == 0, "Number must be even");
///
/// let validator = is_even();
/// assert!(validator.validate(&4).is_ok());
/// assert!(validator.validate(&3).is_err());
/// ```
#[macro_export]
macro_rules! validator_fn {
    (
        $name:ident,
        |$input:ident: &$input_ty:ty| $body:expr,
        $message:expr
    ) => {
        #[derive(Debug, Clone, Copy)]
        pub struct $name;

        impl $name {
            pub fn new() -> Self {
                Self
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $crate::foundation::Validate for $name {
            type Input = $input_ty;

            fn validate(&self, $input: &Self::Input) -> Result<(), ValidationError> {
                if $body {
                    Ok(())
                } else {
                    Err($crate::foundation::ValidationError::new(
                        stringify!($name),
                        $message,
                    ))
                }
            }
        }
    };
}

// ============================================================================
// VALIDATOR_CONST MACRO
// ============================================================================

/// Creates a const validator (zero-size type).
///
/// Useful for validators without configuration.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validator_const;
///
/// validator_const! {
///     NotEmpty,
///     |s: &str| !s.is_empty(),
///     "String must not be empty"
/// }
/// ```
#[macro_export]
macro_rules! validator_const {
    (
        $name:ident,
        |$input:ident: &$input_ty:ty| $body:expr,
        $message:expr
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name;

        impl $crate::foundation::Validate for $name {
            type Input = $input_ty;

            fn validate(&self, $input: &Self::Input) -> Result<(), ValidationError> {
                if $body {
                    Ok(())
                } else {
                    Err($crate::foundation::ValidationError::new(
                        stringify!($name),
                        $message,
                    ))
                }
            }
        }
    };
}

// ============================================================================
// COMPOSE MACRO
// ============================================================================

/// Composes multiple validators using AND logic.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::compose;
///
/// let validator = compose![
///     min_length(5),
///     max_length(20),
///     alphanumeric(),
/// ];
/// ```
#[macro_export]
macro_rules! compose {
    ($first:expr) => {
        $first
    };
    ($first:expr, $($rest:expr),+ $(,)?) => {
        $first$(.and($rest))+
    };
}

// ============================================================================
// ANY_OF MACRO
// ============================================================================

/// Composes multiple validators using OR logic.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::any_of;
///
/// let validator = any_of![
///     exact_length(5),
///     exact_length(10),
///     exact_length(15),
/// ];
/// ```
#[macro_export]
macro_rules! any_of {
    ($first:expr) => {
        $first
    };
    ($first:expr, $($rest:expr),+ $(,)?) => {
        $first$(.or($rest))+
    };
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{Validate, ValidationError};

    validator! {
        struct TestMinLength {
            min: usize
        }
        impl {
            fn check(input: &str, min: usize) -> bool {
                input.len() >= min
            }
            fn error(min: usize) -> String {
                format!("Must be at least {} characters", min)
            }
            const DESCRIPTION: &str = "Test min length validator";
        }
    }

    #[test]
    fn test_validator_macro() {
        let validator = TestMinLength { min: 5 };
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_validate_macro() {
        let input = "hello";
        let result = validate!(input, |s: &str| s.len() >= 5, "Too short");
        assert!(result.is_ok());

        let result = validate!(input, |s: &str| s.len() >= 10, "Too short");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_macro_with_code() {
        let input = "hello";
        let result = validate!(
            input,
            |s: &str| s.contains("@"),
            "email_invalid",
            "Must contain @"
        );
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.code, "email_invalid");
        }
    }

    validator_fn!(IsEven, |n: &i32| *n % 2 == 0, "Number must be even");

    #[test]
    fn test_validator_fn_macro() {
        let validator = IsEven::new();
        assert!(validator.validate(&4).is_ok());
        assert!(validator.validate(&3).is_err());
    }

    validator_const!(NotEmpty, |s: &str| !s.is_empty(), "Must not be empty");

    #[test]
    fn test_validator_const_macro() {
        let validator = NotEmpty;
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("").is_err());
    }

    #[test]
    fn test_compose_macro() {
        let validator = compose![TestMinLength { min: 5 }, TestMinLength { min: 3 }];
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_any_of_macro() {
        struct ExactLength {
            length: usize,
        }
        impl Validate for ExactLength {
            type Input = str;
            fn validate(&self, input: &str) -> Result<(), ValidationError> {
                if input.len() == self.length {
                    Ok(())
                } else {
                    Err(ValidationError::new("exact_length", "Wrong length"))
                }
            }
        }

        let validator = any_of![ExactLength { length: 5 }, ExactLength { length: 10 }];
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("helloworld").is_ok());
        assert!(validator.validate("hi").is_err());
    }
}
