//! Validator combinators for composition
//!
//! This module provides combinator types that allow composing validators
//! in powerful ways. Combinators follow functional programming principles
//! and enable building complex validation logic from simple building blocks.
//!
//! # Overview
//!
//! Combinators transform or combine validators:
//!
//! - **Logical**: `And`, `Or`, `Not` - boolean logic
//! - **Transformational**: `Map` - transform outputs
//! - **Conditional**: `When` - conditional validation
//! - **Optional**: `Optional`, `RequiredSome` - nullable handling
//! - **Performance**: `Cached` - memoization
//!
//! # Examples
//!
//! ## Logical Combinators
//!
//! ```rust
//! use nebula_validator::prelude::*;
//!
//! // AND: both must pass
//! let validator = min_length(5).and(max_length(20));
//!
//! // OR: at least one must pass
//! let validator = exact_length(5).or(exact_length(10));
//!
//! // NOT: must not pass
//! let validator = contains("test").not();
//! ```
//!
//! ## Transformational Combinators
//!
//! ```rust
//! // MAP: transform output
//! let validator = min_length(5).map(|_| "Valid!");
//! assert_eq!(validator.validate("hello").unwrap(), "Valid!");
//! ```
//!
//! ## Conditional Validation
//!
//! ```rust
//! // WHEN: only validate if condition met
//! let validator = min_length(10).when(|s| s.starts_with("long_"));
//!
//! assert!(validator.validate("short").is_ok());  // skipped
//! assert!(validator.validate("long_enough").is_ok());  // validated
//! ```
//!
//! ## Optional Values
//!
//! ```rust
//! // OPTIONAL: None is always valid
//! let validator = min_length(5).optional();
//!
//! assert!(validator.validate(&None).is_ok());
//! assert!(validator.validate(&Some("hello")).is_ok());
//! ```
//!
//! ## Performance Optimization
//!
//! ```rust
//! // CACHED: memoize results
//! let validator = expensive_validation().cached();
//!
//! validator.validate("test")?;  // First call: slow
//! validator.validate("test")?;  // Second call: instant!
//! ```
//!
//! # Composition Patterns
//!
//! Combinators can be chained to create complex validation logic:
//!
//! ```rust
//! let email_validator = not_null()
//!     .and(string())
//!     .and(contains("@"))
//!     .and(regex(r"^[\w\.-]+@[\w\.-]+\.\w+$"))
//!     .when(|s| !s.is_empty())
//!     .optional();
//! ```

// Module declarations
pub mod and;
pub mod cached;
pub mod error;
pub mod field;
pub mod map;
pub mod nested;
pub mod not;
pub mod optimizer;
pub mod optional;
pub mod or;
pub mod when;

// Re-export all combinator types
pub use and::{And, AndAll, and, and_all};
pub use cached::{CacheStats, Cached, cached};
pub use error::CombinatorError;
pub use field::{Field, FieldError, FieldValidatorExt, MultiField, field, named_field};
pub use map::{Map, MapWithInput, map, map_to, map_unit, map_with_input};
pub use nested::{
    CollectionNested, NestedValidator, OptionalNested, Validatable, collection_nested,
    custom_nested, nested_validator, optional_nested,
};
pub use not::{Not, not};
pub use optimizer::{
    OptimizationReport, OptimizationStrategy, ValidatorChainOptimizer, ValidatorOrdering,
    ValidatorStats,
};
pub use optional::{Nullable, Optional, RequiredSome, nullable, optional, required_some};
pub use or::{Or, OrAny, OrAnyError, or, or_any};
pub use when::{When, unless, when, when_not_empty, when_some};

#[cfg(feature = "lru")]
pub use cached::{LruCached, lru_cached};

// ============================================================================
// PRELUDE
// ============================================================================

/// Common combinator imports.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::combinators::prelude::*;
///
/// let validator = min_length(5)
///     .and(max_length(20))
///     .or(exact_length(0));
/// ```
pub mod prelude {
    pub use super::{
        And, AndAll, Cached, Field, FieldValidatorExt, Map, Not, Optional, Or, OrAny, When, and,
        and_all, cached, field, map, map_to, named_field, not, nullable, optional, or, or_any,
        required_some, unless, when, when_not_empty,
    };

    #[cfg(feature = "lru")]
    pub use super::{LruCached, lru_cached};
}

// ============================================================================
// COMBINATOR LAWS
// ============================================================================

/// Tests that verify algebraic laws for combinators.
///
/// These tests ensure that combinators behave correctly according to
/// mathematical laws (associativity, commutativity, etc.).
#[cfg(test)]
mod laws {
    use super::*;
    use crate::core::{TypedValidator, ValidationError};

    struct AlwaysValid;
    impl TypedValidator for AlwaysValid {
        type Input = str;
        type Output = ();
        type Error = ValidationError;
        fn validate(&self, _: &str) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    struct AlwaysFails;
    impl TypedValidator for AlwaysFails {
        type Input = str;
        type Output = ();
        type Error = ValidationError;
        fn validate(&self, _: &str) -> Result<(), ValidationError> {
            Err(ValidationError::new("fail", "Always fails"))
        }
    }

    #[test]
    fn test_and_associativity() {
        // (a AND b) AND c === a AND (b AND c)
        let left = And::new(And::new(AlwaysValid, AlwaysValid), AlwaysValid);
        let right = And::new(AlwaysValid, And::new(AlwaysValid, AlwaysValid));

        assert_eq!(
            left.validate("test").is_ok(),
            right.validate("test").is_ok()
        );
    }

    #[test]
    fn test_or_associativity() {
        // (a OR b) OR c === a OR (b OR c)
        let left = Or::new(Or::new(AlwaysFails, AlwaysValid), AlwaysFails);
        let right = Or::new(AlwaysFails, Or::new(AlwaysValid, AlwaysFails));

        assert_eq!(
            left.validate("test").is_ok(),
            right.validate("test").is_ok()
        );
    }

    #[test]
    fn test_and_or_distributivity() {
        // a AND (b OR c) should behave predictably
        let and_or = And::new(AlwaysValid, Or::new(AlwaysFails, AlwaysValid));
        assert!(and_or.validate("test").is_ok());

        let and_or_fail = And::new(AlwaysFails, Or::new(AlwaysValid, AlwaysValid));
        assert!(and_or_fail.validate("test").is_err());
    }

    #[test]
    fn test_double_negation() {
        // NOT(NOT(a)) === a
        let validator = AlwaysValid;
        let double_not = Not::new(Not::new(validator));

        assert_eq!(
            AlwaysValid.validate("test").is_ok(),
            double_not.validate("test").is_ok()
        );
    }

    #[test]
    fn test_de_morgan_and() {
        // NOT(a AND b) should fail when (a AND b) passes
        let and_validator = And::new(AlwaysValid, AlwaysValid);
        let not_and = Not::new(and_validator);

        assert!(And::new(AlwaysValid, AlwaysValid).validate("test").is_ok());
        assert!(not_and.validate("test").is_err());
    }

    #[test]
    fn test_de_morgan_or() {
        // NOT(a OR b) should fail when (a OR b) passes
        let or_validator = Or::new(AlwaysFails, AlwaysValid);
        let not_or = Not::new(or_validator);

        assert!(Or::new(AlwaysFails, AlwaysValid).validate("test").is_ok());
        assert!(not_or.validate("test").is_err());
    }
}

// ============================================================================
// INTEGRATION TESTS
// ============================================================================

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::core::{TypedValidator, ValidationError, ValidatorExt};

    struct MinLength {
        min: usize,
    }

    impl TypedValidator for MinLength {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::min_length("", self.min, input.len()))
            }
        }
    }

    struct MaxLength {
        max: usize,
    }

    impl TypedValidator for MaxLength {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() <= self.max {
                Ok(())
            } else {
                Err(ValidationError::max_length("", self.max, input.len()))
            }
        }
    }

    #[test]
    fn test_complex_composition() {
        // Build a complex validator using multiple combinators
        let validator = MinLength { min: 5 }
            .and(MaxLength { max: 20 })
            .when(|s: &str| !s.is_empty())
            .optional();

        // Test various cases
        assert!(validator.validate(&None).is_ok()); // None is valid
        assert!(validator.validate(&Some("")).is_ok()); // Empty skipped by when
        assert!(validator.validate(&Some("hello")).is_ok()); // Valid length
        assert!(validator.validate(&Some("hi")).is_err()); // Too short
        assert!(
            validator
                .validate(&Some("verylongstringthatistoolong"))
                .is_err()
        ); // Too long
    }

    #[test]
    fn test_or_with_different_validators() {
        let validator = MinLength { min: 10 }.or(MaxLength { max: 3 });

        // Should pass if either: >= 10 chars OR <= 3 chars
        assert!(validator.validate("verylongstring").is_ok()); // >= 10
        assert!(validator.validate("hi").is_ok()); // <= 3
        assert!(validator.validate("medium").is_err()); // Neither
    }

    #[test]
    fn test_not_with_and() {
        // NOT(min AND max) = strings that are too short OR too long
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 }).not();

        assert!(validator.validate("hi").is_ok()); // Too short, so NOT passes
        assert!(validator.validate("verylongstring").is_ok()); // Too long, so NOT passes
        assert!(validator.validate("hello").is_err()); // Just right, so NOT fails
    }

    #[test]
    fn test_map_with_and() {
        let validator = MinLength { min: 5 }
            .and(MaxLength { max: 10 })
            .map(|_| "Valid!");

        assert_eq!(validator.validate("hello").unwrap(), "Valid!");
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_cached_with_complex_validator() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        struct Counting {
            counter: Arc<AtomicUsize>,
        }

        impl TypedValidator for Counting {
            type Input = str;
            type Output = ();
            type Error = ValidationError;

            fn validate(&self, _: &str) -> Result<(), ValidationError> {
                self.counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let validator = Counting {
            counter: call_count_clone,
        }
        .and(MinLength { min: 5 })
        .cached();

        validator.validate("hello").unwrap();
        validator.validate("hello").unwrap();
        validator.validate("hello").unwrap();

        // Should only call inner validator once due to caching
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }
}

// ============================================================================
// DOCUMENTATION TESTS
// ============================================================================

#[cfg(test)]
mod doc_tests {
    //! These tests verify that documentation examples compile and work.

    use super::*;
    use crate::core::{TypedValidator, ValidationError, ValidatorExt};

    struct Contains {
        substring: String,
    }

    impl TypedValidator for Contains {
        type Input = str;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.contains(&self.substring) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "contains",
                    format!("Must contain '{}'", self.substring),
                ))
            }
        }
    }

    #[test]
    fn test_readme_example() {
        struct MinLength {
            min: usize,
        }
        struct MaxLength {
            max: usize,
        }

        impl TypedValidator for MinLength {
            type Input = str;
            type Output = ();
            type Error = ValidationError;
            fn validate(&self, input: &str) -> Result<(), ValidationError> {
                if input.len() >= self.min {
                    Ok(())
                } else {
                    Err(ValidationError::min_length("", self.min, input.len()))
                }
            }
        }

        impl TypedValidator for MaxLength {
            type Input = str;
            type Output = ();
            type Error = ValidationError;
            fn validate(&self, input: &str) -> Result<(), ValidationError> {
                if input.len() <= self.max {
                    Ok(())
                } else {
                    Err(ValidationError::max_length("", self.max, input.len()))
                }
            }
        }

        let validator = MinLength { min: 5 }.and(MaxLength { max: 20 });

        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
        assert!(validator.validate("verylongstringthatistoolong").is_err());
    }
}
