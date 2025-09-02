//! Optimized async validator trait without boxing overhead

use std::future::Future;
use std::pin::Pin;
use crate::core::{Valid, Invalid};
use crate::types::{ValidatorMetadata, ValidationComplexity};

/// Async validator trait with associated future type to avoid boxing
pub trait AsyncValidator<T>: Send + Sync {
    /// Output type after validation
    type Output: Send;
    
    /// Future type for validation
    type Future: Future<Output = Result<Valid<Self::Output>, Invalid<Self::Output>>> + Send;
    
    /// Validate asynchronously without boxing
    fn validate_async(&self, value: T) -> Self::Future;
    
    /// Get validator metadata
    fn metadata(&self) -> ValidatorMetadata;
    
    /// Get validation complexity
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
    
    /// Check if cacheable
    fn is_cacheable(&self) -> bool {
        true
    }
}

/// Extension trait for AsyncValidator
pub trait AsyncValidatorExt<T>: AsyncValidator<T> {
    /// Convert to a boxed future validator
    fn boxed(self) -> BoxedAsyncValidator<T, Self::Output>
    where
        Self: Sized + 'static,
    {
        BoxedAsyncValidator::new(self)
    }
    
    /// Map the output
    fn map<U, F>(self, f: F) -> MappedAsyncValidator<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output) -> U + Send + Sync,
        U: Send,
    {
        MappedAsyncValidator {
            inner: self,
            mapper: f,
        }
    }
    
    /// Run validators in parallel
    fn parallel<V>(self, other: V) -> ParallelValidator<Self, V>
    where
        Self: Sized,
        V: AsyncValidator<T>,
    {
        ParallelValidator {
            first: self,
            second: other,
        }
    }
}

impl<T, V> AsyncValidatorExt<T> for V where V: AsyncValidator<T> {}

/// Boxed async validator for type erasure
pub struct BoxedAsyncValidator<T, O> {
    inner: Box<
        dyn Fn(T) -> Pin<Box<dyn Future<Output = Result<Valid<O>, Invalid<O>>> + Send>>
            + Send
            + Sync,
    >,
    metadata: ValidatorMetadata,
    complexity: ValidationComplexity,
}

impl<T, O> BoxedAsyncValidator<T, O> {
    /// Create a new boxed validator
    pub fn new<V>(validator: V) -> Self
    where
        V: AsyncValidator<T, Output = O> + 'static,
    {
        let metadata = validator.metadata();
        let complexity = validator.complexity();
        
        Self {
            inner: Box::new(move |value| Box::pin(validator.validate_async(value))),
            metadata,
            complexity,
        }
    }
}

impl<T, O> AsyncValidator<T> for BoxedAsyncValidator<T, O>
where
    T: Send + 'static,
    O: Send + 'static,
{
    type Output = O;
    type Future = Pin<Box<dyn Future<Output = Result<Valid<O>, Invalid<O>>> + Send>>;
    
    fn validate_async(&self, value: T) -> Self::Future {
        (self.inner)(value)
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        self.metadata.clone()
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.complexity
    }
}

/// Mapped async validator
pub struct MappedAsyncValidator<V, F> {
    inner: V,
    mapper: F,
}

impl<T, V, F, U> AsyncValidator<T> for MappedAsyncValidator<V, F>
where
    V: AsyncValidator<T>,
    F: Fn(V::Output) -> U + Send + Sync,
    U: Send,
{
    type Output = U;
    type Future = impl Future<Output = Result<Valid<U>, Invalid<U>>> + Send;
    
    fn validate_async(&self, value: T) -> Self::Future {
        async move {
            match self.inner.validate_async(value).await {
                Ok(valid) => Ok(valid.map(&self.mapper)),
                Err(invalid) => Err(invalid.map_value(&self.mapper)),
            }
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        self.inner.metadata()
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.inner.complexity()
    }
}

/// Parallel validator that runs two validators concurrently
pub struct ParallelValidator<V1, V2> {
    first: V1,
    second: V2,
}

impl<T, V1, V2> AsyncValidator<T> for ParallelValidator<V1, V2>
where
    T: Clone + Send,
    V1: AsyncValidator<T>,
    V2: AsyncValidator<T>,
{
    type Output = (V1::Output, V2::Output);
    type Future = impl Future<Output = Result<Valid<Self::Output>, Invalid<Self::Output>>> + Send;
    
    fn validate_async(&self, value: T) -> Self::Future {
        async move {
            let (result1, result2) = tokio::join!(
                self.first.validate_async(value.clone()),
                self.second.validate_async(value)
            );
            
            match (result1, result2) {
                (Ok(valid1), Ok(valid2)) => {
                    Ok(Valid::new(
                        (valid1.into_value(), valid2.into_value()),
                        valid1.proof().merge(valid2.proof().clone()),
                    ))
                },
                (Err(invalid1), Err(invalid2)) => {
                    let mut errors = invalid1.errors().to_vec();
                    errors.extend(invalid2.errors().to_vec());
                    Err(Invalid::without_value(errors))
                },
                (Err(invalid), _) | (_, Err(invalid)) => {
                    Err(Invalid::without_value(invalid.errors().to_vec()))
                },
            }
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            format!("{}||{}", self.first.metadata().id, self.second.metadata().id),
            "Parallel validation",
            crate::types::ValidatorCategory::Logical,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.first.complexity().combine(self.second.complexity())
    }
}