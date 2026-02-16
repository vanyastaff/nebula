//! AND combinator - logical conjunction of validators

use crate::foundation::{Validate, ValidationError};

/// Combines two validators with logical AND.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct And<L, R> {
    pub(crate) left: L,
    pub(crate) right: R,
}

impl<L, R> And<L, R> {
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

impl<L, R> Validate for And<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    type Input = L::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        self.left.validate(input)?;
        self.right.validate(input)?;
        Ok(())
    }
}

impl<L, R> And<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    pub fn and<V>(self, other: V) -> And<Self, V>
    where
        V: Validate<Input = L::Input>,
    {
        And::new(self, other)
    }
}

pub fn and<L, R>(left: L, right: R) -> And<L, R>
where
    L: Validate,
    R: Validate<Input = L::Input>,
{
    And::new(left, right)
}

#[must_use]
pub fn and_all<V>(validators: Vec<V>) -> AndAll<V>
where
    V: Validate,
{
    AndAll { validators }
}

#[derive(Debug, Clone)]
pub struct AndAll<V> {
    validators: Vec<V>,
}

impl<V> Validate for AndAll<V>
where
    V: Validate,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate(input)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::traits::ValidateExt;

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

    struct MaxLength {
        max: usize,
    }

    impl Validate for MaxLength {
        type Input = str;
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() <= self.max {
                Ok(())
            } else {
                Err(ValidationError::max_length("", self.max, input.len()))
            }
        }
    }

    #[test]
    fn test_and_both_pass() {
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 });
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_and_left_fails() {
        let validator = And::new(MinLength { min: 5 }, MaxLength { max: 10 });
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_and_chain() {
        let validator = MinLength { min: 3 }
            .and(MaxLength { max: 10 })
            .and(MinLength { min: 5 });
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_and_all() {
        let validators = vec![
            MinLength { min: 3 },
            MinLength { min: 5 },
            MinLength { min: 7 },
        ];
        let combined = and_all(validators);
        assert!(combined.validate("helloworld").is_ok());
        assert!(combined.validate("hello").is_err());
    }
}
