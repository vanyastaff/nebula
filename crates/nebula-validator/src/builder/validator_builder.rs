//! Main validator builder for composing complex validators

use super::{Builder, BuilderResult, BuilderError};
use crate::traits::{Validatable, ValidatableExt};
use crate::validators::logical::{And, Or, Not};
use crate::validators::conditional::{When, Unless};
use serde_json::Value;
use std::marker::PhantomData;

/// Main validator builder
#[derive(Debug)]
pub struct ValidatorBuilder<T = Value> {
    validators: Vec<Box<dyn Validatable>>,
    mode: CompositionMode,
    metadata: BuilderMetadata,
    _phantom: PhantomData<T>,
}

/// How to compose multiple validators
#[derive(Debug, Clone, Copy)]
pub enum CompositionMode {
    /// All validators must pass (AND)
    All,
    /// At least one validator must pass (OR)
    Any,
    /// Exactly one validator must pass (XOR)
    One,
    /// Chain validators sequentially
    Sequential,
}

/// Builder metadata
#[derive(Debug, Default)]
struct BuilderMetadata {
    name: Option<String>,
    description: Option<String>,
    tags: Vec<String>,
}

impl<T> ValidatorBuilder<T> {
    /// Create a new validator builder
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
            mode: CompositionMode::All,
            metadata: BuilderMetadata::default(),
            _phantom: PhantomData,
        }
    }
    
    /// Set the composition mode
    pub fn mode(mut self, mode: CompositionMode) -> Self {
        self.mode = mode;
        self
    }
    
    /// Set the name
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.metadata.name = Some(name.into());
        self
    }
    
    /// Set the description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.metadata.description = Some(desc.into());
        self
    }
    
    /// Add a tag
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.metadata.tags.push(tag.into());
        self
    }
    
    /// Add a validator
    pub fn add<V: Validatable + 'static>(mut self, validator: V) -> Self {
        self.validators.push(Box::new(validator));
        self
    }
    
    /// Add a string validator
    pub fn string(self) -> super::StringValidatorBuilder {
        super::StringValidatorBuilder::from_parent(Box::new(self))
    }
    
    /// Add a numeric validator
    pub fn number(self) -> super::NumericValidatorBuilder {
        super::NumericValidatorBuilder::from_parent(Box::new(self))
    }
    
    /// Add an object validator
    pub fn object(self) -> super::ObjectValidatorBuilder {
        super::ObjectValidatorBuilder::from_parent(Box::new(self))
    }
    
    /// Add an array validator
    pub fn array(self) -> super::ArrayValidatorBuilder {
        super::ArrayValidatorBuilder::from_parent(Box::new(self))
    }
    
    /// Add a required validator
    pub fn required(mut self) -> Self {
        self.validators.push(Box::new(crate::validators::basic::Required::new()));
        self
    }
    
    /// Add an optional validator
    pub fn optional(mut self) -> Self {
        self.validators.push(Box::new(crate::validators::basic::Optional::new()));
        self
    }
    
    /// Add a conditional validator
    pub fn when<C, V>(mut self, condition: C, validator: V) -> Self
    where
        C: Validatable + 'static,
        V: Validatable + 'static,
    {
        self.validators.push(Box::new(When::new(condition, validator)));
        self
    }
    
    /// Add an unless validator
    pub fn unless<C, V>(mut self, condition: C, validator: V) -> Self
    where
        C: Validatable + 'static,
        V: Validatable + 'static,
    {
        self.validators.push(Box::new(Unless::new(condition, validator)));
        self
    }
    
    /// Add a custom validator
    pub fn custom<F>(mut self, name: impl Into<String>, f: F) -> Self
    where
        F: Fn(&Value) -> bool + Send + Sync + 'static,
    {
        self.validators.push(Box::new(
            crate::validators::basic::Predicate::new(name, f, "Custom validation failed")
        ));
        self
    }
    
    /// Add validation for a specific type
    pub fn validate_type(mut self, expected_type: ValueType) -> Self {
        self.validators.push(Box::new(
            crate::validators::basic::IsType::new(expected_type)
        ));
        self
    }
}

impl<T> Default for ValidatorBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder for ValidatorBuilder<Value> {
    type Output = Box<dyn Validatable>;
    
    fn build(self) -> BuilderResult<Self::Output> {
        if self.validators.is_empty() {
            return Err(BuilderError::InvalidConfiguration(
                "No validators added".to_string()
            ));
        }
        
        let validator: Box<dyn Validatable> = match self.mode {
            CompositionMode::All => {
                if self.validators.len() == 1 {
                    self.validators.into_iter().next().unwrap()
                } else {
                    Box::new(crate::validators::logical::All::new(self.validators))
                }
            },
            CompositionMode::Any => {
                if self.validators.len() == 1 {
                    self.validators.into_iter().next().unwrap()
                } else {
                    Box::new(crate::validators::logical::Any::new(self.validators))
                }
            },
            CompositionMode::One => {
                Box::new(crate::validators::logical::Xor::new(self.validators))
            },
            CompositionMode::Sequential => {
                Box::new(crate::validators::logical::Chain::new(self.validators))
            },
        };
        
        Ok(validator)
    }
}