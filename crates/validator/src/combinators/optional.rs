//! OPTIONAL combinator - validates Option types

use crate::foundation::{Validate, ValidationError};

/// Makes a validator work with Option types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Optional<V> {
    pub(crate) inner: V,
}

impl<V> Optional<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &V {
        &self.inner
    }

    pub fn into_inner(self) -> V {
        self.inner
    }
}

impl<V, T> Validate for Optional<V>
where
    V: Validate<Input = T>,
{
    type Input = Option<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        match input {
            None => Ok(()),
            Some(value) => self.inner.validate(value),
        }
    }
}

pub fn optional<V>(validator: V) -> Optional<V> {
    Optional::new(validator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;

    struct MinLength {
        min: usize,
    }

    impl Validate for MinLength {
        type Input = String;
        fn validate(&self, input: &String) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::min_length("", self.min, input.len()))
            }
        }
    }

    #[test]
    fn test_optional_none() {
        let validator = Optional::new(MinLength { min: 5 });
        let input: Option<String> = None;
        assert!(validator.validate(&input).is_ok());
    }

    #[test]
    fn test_optional_some_valid() {
        let validator = Optional::new(MinLength { min: 5 });
        let input = Some("hello".to_string());
        assert!(validator.validate(&input).is_ok());
    }

    #[test]
    fn test_optional_some_invalid() {
        let validator = Optional::new(MinLength { min: 5 });
        let input = Some("hi".to_string());
        assert!(validator.validate(&input).is_err());
    }

    #[test]
    fn test_optional_helper() {
        let validator = optional(MinLength { min: 5 });
        let none: Option<String> = None;
        let some_valid = Some("hello".to_string());
        let some_invalid = Some("hi".to_string());

        assert!(validator.validate(&none).is_ok());
        assert!(validator.validate(&some_valid).is_ok());
        assert!(validator.validate(&some_invalid).is_err());
    }
}
