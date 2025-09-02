//! Generic Validator trait for typed validation

use async_trait::async_trait;
use crate::core::{Valid, Invalid};
use crate::types::{ValidatorMetadata, ValidationComplexity, ValidatorId};
use std::fmt::Debug;

/// Generic validator trait for typed values
#[async_trait]
pub trait Validator<T>: Send + Sync + Debug {
    /// Output type after validation
    type Output;
    
    /// Validate a typed value
    async fn validate(&self, value: T) -> Result<Valid<Self::Output>, Invalid<Self::Output>>;
    
    /// Get validator metadata
    fn metadata(&self) -> ValidatorMetadata;
    
    /// Get validation complexity
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
    
    /// Transform the validator's output
    fn map<U, F>(self, f: F) -> MappedValidator<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output) -> U + Send + Sync,
    {
        MappedValidator {
            inner: self,
            mapper: f,
        }
    }
    
    /// Chain another validator
    fn and_then<V>(self, next: V) -> ChainedValidator<Self, V>
    where
        Self: Sized,
        V: Validator<Self::Output>,
    {
        ChainedValidator {
            first: self,
            second: next,
        }
    }
    
    /// Provide a fallback validator
    fn or_else<V>(self, fallback: V) -> FallbackValidator<Self, V>
    where
        Self: Sized,
        V: Validator<T, Output = Self::Output>,
    {
        FallbackValidator {
            primary: self,
            fallback,
        }
    }
}

/// Mapped validator that transforms output
#[derive(Debug)]
pub struct MappedValidator<V, F> {
    inner: V,
    mapper: F,
}

#[async_trait]
impl<T, V, F, U> Validator<T> for MappedValidator<V, F>
where
    V: Validator<T>,
    F: Fn(V::Output) -> U + Send + Sync,
    U: Send + Sync,
{
    type Output = U;
    
    async fn validate(&self, value: T) -> Result<Valid<Self::Output>, Invalid<Self::Output>> {
        match self.inner.validate(value).await {
            Ok(valid) => Ok(valid.map(&self.mapper)),
            Err(invalid) => Err(invalid.map_value(&self.mapper)),
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        self.inner.metadata()
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.inner.complexity()
    }
}

/// Chained validator that runs validators in sequence
#[derive(Debug)]
pub struct ChainedValidator<V1, V2> {
    first: V1,
    second: V2,
}

#[async_trait]
impl<T, V1, V2> Validator<T> for ChainedValidator<V1, V2>
where
    T: Send + Sync,
    V1: Validator<T>,
    V2: Validator<V1::Output>,
{
    type Output = V2::Output;
    
    async fn validate(&self, value: T) -> Result<Valid<Self::Output>, Invalid<Self::Output>> {
        match self.first.validate(value).await {
            Ok(valid) => self.second.validate(valid.into_value()).await,
            Err(invalid) => Err(Invalid::without_value(invalid.errors().to_vec())),
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            format!("{}→{}", self.first.metadata().id, self.second.metadata().id),
            format!("{} then {}", self.first.metadata().name, self.second.metadata().name),
            crate::types::ValidatorCategory::Logical,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.first.complexity().add(self.second.complexity())
    }
}

/// Fallback validator that tries a backup on failure
#[derive(Debug)]
pub struct FallbackValidator<V1, V2> {
    primary: V1,
    fallback: V2,
}

#[async_trait]
impl<T, V1, V2> Validator<T> for FallbackValidator<V1, V2>
where
    T: Clone + Send + Sync,
    V1: Validator<T>,
    V2: Validator<T, Output = V1::Output>,
{
    type Output = V1::Output;
    
    async fn validate(&self, value: T) -> Result<Valid<Self::Output>, Invalid<Self::Output>> {
        match self.primary.validate(value.clone()).await {
            Ok(valid) => Ok(valid),
            Err(_) => self.fallback.validate(value).await,
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            format!("{}||{}", self.primary.metadata().id, self.fallback.metadata().id),
            format!("{} or {}", self.primary.metadata().name, self.fallback.metadata().name),
            crate::types::ValidatorCategory::Logical,
        )
    }
}

/// Typed validator that works with specific types
pub trait TypedValidator<T>: Validator<T> {
    /// Get the type name this validator accepts
    fn type_name(&self) -> &'static str;
    
    /// Check if the validator accepts a value
    fn accepts_type(&self, value: &T) -> bool;
}

/// Trait for cloneable validators
pub trait ValidatorClone<T>: Validator<T> {
    /// Clone the validator into a box
    fn clone_box(&self) -> Box<dyn Validator<T, Output = Self::Output>>;
}