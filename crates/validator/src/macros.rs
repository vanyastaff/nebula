//! Macros for creating validators with minimal boilerplate.
//!
//! # Available Macros
//!
//! - [`validator!`] — Create a complete validator (struct + Validate impl + factory fn)
//! - [`compose!`] — AND-chain multiple validators
//! - [`any_of!`] — OR-chain multiple validators
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::validator;
//! use nebula_validator::foundation::{Validate, ValidationError};
//!
//! // Unit validator (no fields)
//! validator! {
//!     pub NotEmpty for str;
//!     rule(input) { !input.is_empty() }
//!     error(input) { ValidationError::new("not_empty", "must not be empty") }
//!     fn not_empty();
//! }
//!
//! // Struct with fields
//! validator! {
//!     #[derive(Copy, PartialEq, Eq, Hash)]
//!     pub MinLength { min: usize } for str;
//!     rule(self, input) { input.len() >= self.min }
//!     error(self, input) { ValidationError::min_length("", self.min, input.len()) }
//!     fn min_length(min: usize);
//! }
//! ```

// ============================================================================
// VALIDATOR MACRO
// ============================================================================

/// Creates a complete validator: struct definition, `Validate` implementation,
/// constructor, and factory function.
///
/// `#[derive(Debug, Clone)]` is always applied. Add extra derives via `#[derive(...)]`.
///
/// # Variants
///
/// **Unit validator** (zero-sized, no fields):
/// ```rust,ignore
/// validator! {
///     pub NotEmpty for str;
///     rule(input) { !input.is_empty() }
///     error(input) { ValidationError::new("not_empty", "empty") }
///     fn not_empty();
/// }
/// ```
///
/// **Struct with fields** (auto `new` from all fields):
/// ```rust,ignore
/// validator! {
///     #[derive(Copy, PartialEq, Eq, Hash)]
///     pub MinLength { min: usize } for str;
///     rule(self, input) { input.len() >= self.min }
///     error(self, input) { ValidationError::min_length("", self.min, input.len()) }
///     fn min_length(min: usize);
/// }
/// ```
///
/// **Custom constructor** (overrides auto `new`):
/// ```rust,ignore
/// validator! {
///     pub LengthRange { min: usize, max: usize } for str;
///     rule(self, input) { let l = input.len(); l >= self.min && l <= self.max }
///     error(self, input) { ValidationError::new("range", "out of range") }
///     new(min: usize, max: usize) { Self { min, max } }
///     fn length_range(min: usize, max: usize);
/// }
/// ```
///
/// **Generic validator**:
/// ```rust,ignore
/// validator! {
///     #[derive(Copy, PartialEq, Eq, Hash)]
///     pub Min<T: PartialOrd + Display + Copy> { min: T } for T;
///     rule(self, input) { *input >= self.min }
///     error(self, input) { ValidationError::new("min", format!("must be >= {}", self.min)) }
///     fn min(value: T);
/// }
/// ```
#[macro_export]
macro_rules! validator {
    // ── Variant 1a: Unit validator (no fields) + factory fn ──────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident for $input:ty;
        rule($inp:ident) $rule:block
        error($einp:ident) $err:block
        fn $factory:ident();
    ) => {
        $crate::validator! {
            $(#[$meta])*
            $vis $name for $input;
            rule($inp) $rule
            error($einp) $err
        }

        #[must_use]
        $vis const fn $factory() -> $name { $name }
    };

    // ── Variant 1b: Unit validator (no fields), no factory ───────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident for $input:ty;
        rule($inp:ident) $rule:block
        error($einp:ident) $err:block
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis struct $name;

        impl $crate::foundation::Validate for $name {
            type Input = $input;

            #[allow(unused_variables)]
            fn validate(&self, $inp: &Self::Input) -> Result<(), $crate::foundation::ValidationError> {
                if $rule {
                    Ok(())
                } else {
                    let $einp = $inp;
                    Err($err)
                }
            }
        }
    };

    // ── Variant 3a: Struct with fields + custom new + factory fn ─────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self_:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        new($($narg:ident: $naty:ty),* $(,)?) $new_body:block
        fn $factory:ident($($farg:ident: $faty:ty),* $(,)?);
    ) => {
        $crate::validator! {
            $(#[$meta])*
            $vis $name { $($field: $fty),+ } for $input;
            rule($self_, $inp) $rule
            error($self2, $einp) $err
            new($($narg: $naty),*) $new_body
        }

        #[must_use]
        $vis fn $factory($($farg: $faty),*) -> $name {
            $name::new($($farg),*)
        }
    };

    // ── Variant 3b: Struct with fields + custom new, no factory ──────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self_:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        new($($narg:ident: $naty:ty),* $(,)?) $new_body:block
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        $vis struct $name {
            $(pub $field: $fty,)+
        }

        impl $name {
            #[must_use]
            pub fn new($($narg: $naty),*) -> Self $new_body
        }

        impl $crate::foundation::Validate for $name {
            type Input = $input;

            #[allow(unused_variables)]
            fn validate(&$self_, $inp: &Self::Input) -> Result<(), $crate::foundation::ValidationError> {
                if $rule {
                    Ok(())
                } else {
                    let $einp = $inp;
                    Err($err)
                }
            }
        }
    };

    // ── Variant 2a: Struct with fields + auto new + factory fn ───────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self_:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        fn $factory:ident($($farg:ident: $faty:ty),* $(,)?);
    ) => {
        $crate::validator! {
            $(#[$meta])*
            $vis $name { $($field: $fty),+ } for $input;
            rule($self_, $inp) $rule
            error($self2, $einp) $err
        }

        #[must_use]
        $vis fn $factory($($farg: $faty),*) -> $name {
            $name::new($($farg),*)
        }
    };

    // ── Variant 2b: Struct with fields + auto new, no factory ────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self_:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        $vis struct $name {
            $(pub $field: $fty,)+
        }

        impl $name {
            #[must_use]
            pub fn new($($field: $fty),+) -> Self {
                Self { $($field),+ }
            }
        }

        impl $crate::foundation::Validate for $name {
            type Input = $input;

            #[allow(unused_variables)]
            fn validate(&$self_, $inp: &Self::Input) -> Result<(), $crate::foundation::ValidationError> {
                if $rule {
                    Ok(())
                } else {
                    let $einp = $inp;
                    Err($err)
                }
            }
        }
    };

    // ── Variant 4a: Generic struct + auto new + factory fn ───────────────
    //
    // Supports a single generic type parameter with one or more trait bounds.
    // Bounds must be simple identifiers (use imports for paths).
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident<$gen:ident: $first_bound:ident $(+ $rest_bound:ident)*>
            { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self_:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        fn $factory:ident($($farg:ident: $faty:ty),* $(,)?);
    ) => {
        $crate::validator! {
            $(#[$meta])*
            $vis $name<$gen: $first_bound $(+ $rest_bound)*>
                { $($field: $fty),+ } for $input;
            rule($self_, $inp) $rule
            error($self2, $einp) $err
        }

        #[must_use]
        $vis fn $factory<$gen: $first_bound $(+ $rest_bound)*>($($farg: $faty),*) -> $name<$gen> {
            $name::new($($farg),*)
        }
    };

    // ── Variant 4b: Generic struct + auto new, no factory ────────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident<$gen:ident: $first_bound:ident $(+ $rest_bound:ident)*>
            { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self_:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        $vis struct $name<$gen> {
            $(pub $field: $fty,)+
        }

        impl<$gen: $first_bound $(+ $rest_bound)*> $name<$gen> {
            #[must_use]
            pub fn new($($field: $fty),+) -> Self {
                Self { $($field),+ }
            }
        }

        impl<$gen: $first_bound $(+ $rest_bound)*> $crate::foundation::Validate for $name<$gen> {
            type Input = $input;

            #[allow(unused_variables)]
            fn validate(&$self_, $inp: &Self::Input) -> Result<(), $crate::foundation::ValidationError> {
                if $rule {
                    Ok(())
                } else {
                    let $einp = $inp;
                    Err($err)
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
/// ```rust,ignore
/// let validator = compose![min_length(5), max_length(20), alphanumeric()];
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
/// ```rust,ignore
/// let validator = any_of![exact_length(5), exact_length(10)];
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
    use crate::foundation::{Validate, ValidationError};

    // Test 1: Unit validator (no fields)
    validator! {
        /// A test unit validator.
        TestNotEmpty for str;
        rule(input) { !input.is_empty() }
        error(input) { ValidationError::new("not_empty", "must not be empty") }
        fn test_not_empty();
    }

    #[test]
    fn test_unit_validator() {
        let v = TestNotEmpty;
        assert!(v.validate("hello").is_ok());
        assert!(v.validate("").is_err());
    }

    #[test]
    fn test_unit_factory() {
        let v = test_not_empty();
        assert!(v.validate("x").is_ok());
    }

    // Test 2: Struct with fields + auto new
    validator! {
        #[derive(Copy, PartialEq, Eq, Hash)]
        TestMinLen { min: usize } for str;
        rule(self, input) { input.len() >= self.min }
        error(self, input) {
            ValidationError::new("min_len", format!("need {} chars", self.min))
        }
        fn test_min_len(min: usize);
    }

    #[test]
    fn test_struct_validator() {
        let v = TestMinLen { min: 3 };
        assert!(v.validate("abc").is_ok());
        assert!(v.validate("ab").is_err());
    }

    #[test]
    fn test_struct_new() {
        let v = TestMinLen::new(5);
        assert!(v.validate("hello").is_ok());
        assert!(v.validate("hi").is_err());
    }

    #[test]
    fn test_struct_factory() {
        let v = test_min_len(5);
        assert!(v.validate("hello").is_ok());
        assert!(v.validate("hi").is_err());
    }

    // Test 3: Generic validator
    use std::fmt::Display;

    validator! {
        #[derive(Copy, PartialEq, Eq, Hash)]
        TestMin<T: PartialOrd + Display + Copy> { min: T } for T;
        rule(self, input) { *input >= self.min }
        error(self, input) {
            ValidationError::new("min", format!("must be >= {}", self.min))
        }
        fn test_min_val(value: T);
    }

    #[test]
    fn test_generic_validator() {
        let v = test_min_val(5_i32);
        assert!(v.validate(&5).is_ok());
        assert!(v.validate(&4).is_err());
    }

    #[test]
    fn test_generic_validator_f64() {
        let v = TestMin::new(1.5_f64);
        assert!(v.validate(&2.0).is_ok());
        assert!(v.validate(&1.0).is_err());
    }

    // Test 4: Custom constructor
    validator! {
        #[derive(Copy, PartialEq, Eq, Hash)]
        TestRange { lo: usize, hi: usize } for usize;
        rule(self, input) { *input >= self.lo && *input <= self.hi }
        error(self, input) {
            ValidationError::new("range", format!("{} not in {}..{}", input, self.lo, self.hi))
        }
        new(lo: usize, hi: usize) { Self { lo, hi } }
        fn test_range(lo: usize, hi: usize);
    }

    #[test]
    fn test_custom_new() {
        let v = test_range(1, 10);
        assert!(v.validate(&5).is_ok());
        assert!(v.validate(&0).is_err());
        assert!(v.validate(&11).is_err());
    }

    // Test 5: Unit validator without factory fn
    validator! {
        TestAlwaysOk for str;
        rule(input) { true }
        error(input) { ValidationError::new("unreachable", "unreachable") }
    }

    #[test]
    fn test_unit_without_factory() {
        let v = TestAlwaysOk;
        assert!(v.validate("anything").is_ok());
    }

    // Test 6: Struct without factory fn
    validator! {
        TestMax { max: usize } for usize;
        rule(self, input) { *input <= self.max }
        error(self, input) {
            ValidationError::new("max", format!("must be <= {}", self.max))
        }
    }

    #[test]
    fn test_struct_without_factory() {
        let v = TestMax::new(10);
        assert!(v.validate(&10).is_ok());
        assert!(v.validate(&11).is_err());
    }

    // Test 7: compose! and any_of! still work
    #[test]
    fn test_compose_still_works() {
        use crate::foundation::ValidateExt;
        let v = compose![TestMinLen { min: 3 }, TestMinLen { min: 1 }];
        assert!(v.validate("abc").is_ok());
        assert!(v.validate("ab").is_err());
    }

    #[test]
    fn test_any_of_still_works() {
        use crate::foundation::ValidateExt;
        let v = any_of![TestMinLen { min: 100 }, TestMinLen { min: 1 }];
        assert!(v.validate("x").is_ok());
    }

    // Test 8: Error messages are correct
    #[test]
    fn test_error_message_content() {
        let v = TestMinLen { min: 5 };
        let err = v.validate("hi").unwrap_err();
        assert_eq!(err.code, "min_len");
        assert_eq!(err.message, "need 5 chars");
    }

    #[test]
    fn test_unit_error_message_content() {
        let v = TestNotEmpty;
        let err = v.validate("").unwrap_err();
        assert_eq!(err.code, "not_empty");
        assert_eq!(err.message, "must not be empty");
    }

    // Test 9: Custom new body is respected
    #[test]
    fn test_custom_new_body() {
        let v = TestRange::new(3, 7);
        assert_eq!(v.lo, 3);
        assert_eq!(v.hi, 7);
    }
}
