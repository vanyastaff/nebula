//! MAP combinator - previously transformed validation output (now deprecated)

use crate::core::{ValidationError, Validator, ValidatorMetadata};

/// Maps the output of a successful validation.
///
/// **Deprecated**: Since all validators now return `Result<(), ValidationError>`,
/// this combinator simply delegates to the inner validator.
#[derive(Debug, Clone, Copy)]
pub struct Map<V, F> {
    pub(crate) validator: V,
    #[allow(dead_code)]
    pub(crate) mapper: F,
}

impl<V, F> Map<V, F> {
    pub fn new(validator: V, mapper: F) -> Self {
        Self { validator, mapper }
    }

    pub fn validator(&self) -> &V {
        &self.validator
    }

    pub fn mapper(&self) -> &F {
        &self.mapper
    }

    pub fn into_parts(self) -> (V, F) {
        (self.validator, self.mapper)
    }
}

impl<V, F> Validator for Map<V, F>
where
    V: Validator,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        self.validator.validate(input)
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.validator.metadata();

        ValidatorMetadata {
            name: format!("Map({})", inner_meta.name),
            description: inner_meta.description,
            complexity: inner_meta.complexity,
            cacheable: inner_meta.cacheable,
            estimated_time: inner_meta.estimated_time,
            tags: inner_meta.tags,
            version: inner_meta.version,
            custom: inner_meta.custom,
        }
    }
}

pub fn map<V, F, O>(validator: V, mapper: F) -> Map<V, F>
where
    V: Validator,
    F: Fn(()) -> O,
{
    Map::new(validator, mapper)
}

pub fn map_to<V, O: Clone>(validator: V, value: O) -> Map<V, impl Fn(()) -> O> {
    Map::new(validator, move |_| value.clone())
}

pub fn map_unit<V>(validator: V) -> Map<V, impl Fn(())>
where
    V: Validator,
{
    Map::new(validator, |_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysValid;
    impl Validator for AlwaysValid {
        type Input = str;
        fn validate(&self, _: &str) -> Result<(), ValidationError> {
            Ok(())
        }
    }

    struct AlwaysFails;
    impl Validator for AlwaysFails {
        type Input = str;
        fn validate(&self, _: &str) -> Result<(), ValidationError> {
            Err(ValidationError::new("fail", "Always fails"))
        }
    }

    #[test]
    fn test_map_delegates_success() {
        let validator = Map::new(AlwaysValid, |_: ()| "mapped");
        assert!(validator.validate("test").is_ok());
    }

    #[test]
    fn test_map_delegates_failure() {
        let validator = Map::new(AlwaysFails, |_: ()| "mapped");
        assert!(validator.validate("test").is_err());
    }
}
