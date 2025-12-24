//! AND combinator - logical conjunction of validators

use crate::core::{ValidationComplexity, ValidationError, Validator, ValidatorMetadata};

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

impl<L, R> Validator for And<L, R>
where
    L: Validator,
    R: Validator<Input = L::Input>,
{
    type Input = L::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        self.left.validate(input)?;
        self.right.validate(input)?;
        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        let left_meta = self.left.metadata();
        let right_meta = self.right.metadata();
        let complexity = std::cmp::max(left_meta.complexity, right_meta.complexity);
        let cacheable = left_meta.cacheable && right_meta.cacheable;

        ValidatorMetadata {
            name: format!("And({}, {})", left_meta.name, right_meta.name),
            description: Some(format!(
                "Both {} and {} must pass",
                left_meta.name, right_meta.name
            )),
            complexity,
            cacheable,
            estimated_time: None,
            tags: {
                let mut tags = left_meta.tags;
                tags.extend(right_meta.tags);
                tags.push("combinator".to_string());
                tags
            },
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

impl<L, R> And<L, R>
where
    L: Validator,
    R: Validator<Input = L::Input>,
{
    pub fn and<V>(self, other: V) -> And<Self, V>
    where
        V: Validator<Input = L::Input>,
    {
        And::new(self, other)
    }
}

pub fn and<L, R>(left: L, right: R) -> And<L, R>
where
    L: Validator,
    R: Validator<Input = L::Input>,
{
    And::new(left, right)
}

#[must_use]
pub fn and_all<V>(validators: Vec<V>) -> AndAll<V>
where
    V: Validator,
{
    AndAll { validators }
}

#[derive(Debug, Clone)]
pub struct AndAll<V> {
    validators: Vec<V>,
}

impl<V> Validator for AndAll<V>
where
    V: Validator,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate(input)?;
        }
        Ok(())
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
            name: format!("AndAll(count={})", self.validators.len()),
            description: Some(format!(
                "All {} validators must pass",
                self.validators.len()
            )),
            complexity,
            cacheable,
            estimated_time: None,
            tags,
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::ValidatorExt;

    struct MinLength {
        min: usize,
    }

    impl Validator for MinLength {
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

    impl Validator for MaxLength {
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
