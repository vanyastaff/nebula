//! NOT combinator - logical negation of validators

use crate::core::{ValidationError, Validator, ValidatorMetadata};

/// Inverts a validator with logical NOT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Not<V> {
    pub(crate) inner: V,
}

impl<V> Not<V> {
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

impl<V> Validator for Not<V>
where
    V: Validator,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        match self.inner.validate(input) {
            Ok(()) => Err(ValidationError::new(
                "not_failed",
                "Validation should have failed but passed",
            )),
            Err(_) => Ok(()),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.inner.metadata();

        ValidatorMetadata {
            name: format!("Not({})", inner_meta.name),
            description: Some(format!("{} must NOT pass", inner_meta.name)),
            complexity: inner_meta.complexity,
            cacheable: inner_meta.cacheable,
            estimated_time: inner_meta.estimated_time,
            tags: {
                let mut tags = inner_meta.tags;
                tags.push("combinator".to_string());
                tags.push("negation".to_string());
                tags
            },
            version: inner_meta.version,
            custom: inner_meta.custom,
        }
    }
}

pub fn not<V>(validator: V) -> Not<V> {
    Not::new(validator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::ValidatorExt;

    struct Contains {
        substring: &'static str,
    }

    impl Validator for Contains {
        type Input = str;
        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.contains(self.substring) {
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
    fn test_not_inverts_success() {
        let validator = Not::new(Contains {
            substring: "forbidden",
        });
        assert!(validator.validate("this is forbidden").is_err());
    }

    #[test]
    fn test_not_inverts_failure() {
        let validator = Not::new(Contains {
            substring: "forbidden",
        });
        assert!(validator.validate("this is allowed").is_ok());
    }

    #[test]
    fn test_not_via_ext() {
        let validator = Contains { substring: "test" }.not();
        assert!(validator.validate("hello world").is_ok());
        assert!(validator.validate("test string").is_err());
    }

    #[test]
    fn test_double_negation() {
        let validator = Contains { substring: "test" }.not().not();
        assert!(validator.validate("test").is_ok());
        assert!(validator.validate("hello").is_err());
    }
}
