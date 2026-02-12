//! OR combinator - logical disjunction of validators

use crate::core::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};
use std::borrow::Cow;

/// Combines two validators with logical OR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Or<L, R> {
    pub(crate) left: L,
    pub(crate) right: R,
}

impl<L, R> Or<L, R> {
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }

    pub fn left(&self) -> &L {
        &self.left
    }

    pub fn right(&self) -> &R {
        &self.right
    }

    pub fn into_parts(self) -> (L, R) {
        (self.left, self.right)
    }
}

impl<L, R> Validate for Or<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    type Input = L::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        match self.left.validate(input) {
            Ok(()) => Ok(()),
            Err(left_error) => match self.right.validate(input) {
                Ok(()) => Ok(()),
                Err(right_error) => Err(ValidationError::new(
                    "or_failed",
                    format!(
                        "Both validators failed: ({}) OR ({})",
                        left_error.message, right_error.message
                    ),
                )),
            },
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        let left_meta = self.left.metadata();
        let right_meta = self.right.metadata();
        let complexity = std::cmp::max(left_meta.complexity, right_meta.complexity);
        let cacheable = left_meta.cacheable && right_meta.cacheable;

        ValidatorMetadata {
            name: format!("Or({}, {})", left_meta.name, right_meta.name).into(),
            description: Some(
                format!("Either {} or {} must pass", left_meta.name, right_meta.name).into(),
            ),
            complexity,
            cacheable,
            estimated_time: None,
            tags: {
                let mut tags = left_meta.tags;
                tags.extend(right_meta.tags);
                tags.push(Cow::Borrowed("combinator"));
                tags
            },
            version: None,
            custom: Vec::new(),
        }
    }
}

impl<L, R> Or<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    pub fn or<V>(self, other: V) -> Or<Self, V>
    where
        V: Validate<Input = L::Input>,
    {
        Or::new(self, other)
    }
}

pub fn or<L, R>(left: L, right: R) -> Or<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    Or::new(left, right)
}

#[must_use]
pub fn or_any<V>(validators: Vec<V>) -> OrAny<V>
where
    V: Validate,
{
    OrAny { validators }
}

#[derive(Debug, Clone)]
pub struct OrAny<V> {
    validators: Vec<V>,
}

impl<V> Validate for OrAny<V>
where
    V: Validate,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let mut errors = Vec::new();

        for validator in &self.validators {
            match validator.validate(input) {
                Ok(()) => return Ok(()),
                Err(e) => errors.push(e.message.clone()),
            }
        }

        Err(ValidationError::new(
            "or_any_failed",
            format!(
                "All {} validators failed: {}",
                errors.len(),
                errors.join(", ")
            ),
        ))
    }

    fn metadata(&self) -> ValidatorMetadata {
        let mut complexity = ValidationComplexity::Constant;
        let mut cacheable = true;
        let mut tags = Vec::new();

        for validator in &self.validators {
            let meta = validator.metadata();
            complexity = std::cmp::max(complexity, meta.complexity);
            cacheable = cacheable && meta.cacheable;
            tags.extend(meta.tags);
        }

        ValidatorMetadata {
            name: format!("OrAny(count={})", self.validators.len()).into(),
            description: Some(
                format!(
                    "At least one of {} validators must pass",
                    self.validators.len()
                )
                .into(),
            ),
            complexity,
            cacheable,
            estimated_time: None,
            tags,
            version: None,
            custom: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::ValidateExt;

    struct ExactLength {
        length: usize,
    }

    impl Validate for ExactLength {
        type Input = str;
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() == self.length {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "exact_length",
                    format!("Expected length {}", self.length),
                ))
            }
        }
    }

    #[test]
    fn test_or_left_passes() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_or_right_passes() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        assert!(validator.validate("helloworld").is_ok());
    }

    #[test]
    fn test_or_both_fail() {
        let validator = Or::new(ExactLength { length: 5 }, ExactLength { length: 10 });
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_or_chain() {
        let validator = ExactLength { length: 3 }
            .or(ExactLength { length: 5 })
            .or(ExactLength { length: 7 });
        assert!(validator.validate("abc").is_ok());
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_or_any() {
        let validators = vec![
            ExactLength { length: 3 },
            ExactLength { length: 5 },
            ExactLength { length: 7 },
        ];
        let combined = or_any(validators);
        assert!(combined.validate("abc").is_ok());
        assert!(combined.validate("hello").is_ok());
        assert!(combined.validate("hi").is_err());
    }
}
