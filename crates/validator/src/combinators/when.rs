//! WHEN combinator - conditional validation

use crate::core::{Validate, ValidationError, ValidatorMetadata};
use std::borrow::Cow;

/// Conditionally applies a validator based on a predicate.
#[derive(Debug, Clone, Copy)]
pub struct When<V, C> {
    pub(crate) validator: V,
    pub(crate) condition: C,
}

impl<V, C> When<V, C> {
    pub fn new(validator: V, condition: C) -> Self {
        Self {
            validator,
            condition,
        }
    }

    pub fn validator(&self) -> &V {
        &self.validator
    }

    pub fn condition(&self) -> &C {
        &self.condition
    }

    pub fn into_parts(self) -> (V, C) {
        (self.validator, self.condition)
    }
}

impl<V, C> Validate for When<V, C>
where
    V: Validate,
    C: Fn(&V::Input) -> bool,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if (self.condition)(input) {
            self.validator.validate(input)
        } else {
            Ok(())
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();

        ValidatorMetadata {
            name: format!("When({})", inner_meta.name).into(),
            description: Some(format!("Conditionally apply {}", inner_meta.name).into()),
            complexity: inner_meta.complexity,
            cacheable: false,
            estimated_time: inner_meta.estimated_time,
            tags: {
                let mut tags = inner_meta.tags;
                tags.push(Cow::Borrowed("combinator"));
                tags.push("conditional".into());
                tags
            },
            version: inner_meta.version,
            custom: inner_meta.custom,
        }
    }
}

pub fn when<V, C>(validator: V, condition: C) -> When<V, C>
where
    V: Validate,
    C: Fn(&V::Input) -> bool,
{
    When::new(validator, condition)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::ValidateExt;

    struct MinLength {
        min: usize,
    }

    impl Validate for MinLength {
        type Input = str;
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::min_length("", self.min, input.len()))
            }
        }
    }

    #[test]
    fn test_when_condition_true() {
        let validator = When::new(MinLength { min: 10 }, |s: &str| s.starts_with("check_"));
        assert!(validator.validate("check_hello").is_ok()); // 11 chars >= 10
        assert!(validator.validate("check_").is_err()); // 6 chars < 10
    }

    #[test]
    fn test_when_condition_false() {
        let validator = When::new(MinLength { min: 5 }, |s: &str| s.starts_with("check_"));
        assert!(validator.validate("hi").is_ok());
        assert!(validator.validate("").is_ok());
    }

    #[test]
    fn test_when_via_ext() {
        let validator = MinLength { min: 10 }.when(|s: &str| !s.is_empty());
        assert!(validator.validate("").is_ok());
        assert!(validator.validate("short").is_err());
        assert!(validator.validate("long_enough!").is_ok());
    }
}
