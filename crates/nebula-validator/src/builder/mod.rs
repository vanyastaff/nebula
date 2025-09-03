//! Builder API for creating validators with fluent interface
//! 
//! This module provides builders for creating complex validators
//! with a clean, chainable API.

mod validator_builder;

// Note: string/numeric/object/array builders are not present in this repo snapshot.
// Keep only the validator_builder exports to avoid missing module errors.

// Re-export available builders
pub use validator_builder::ValidatorBuilder;

// Prelude for convenient imports
pub mod prelude {
    pub use super::ValidatorBuilder;
}

/// Common builder traits
pub trait Builder: Sized {
    /// The output type when building
    type Output;
    
    /// Build the validator
    fn build(self) -> BuilderResult<Self::Output>;
}

/// Chainable builder trait
pub trait ChainableBuilder: Builder {
    /// Chain with another builder
    fn chain<B: Builder>(self, other: B) -> ChainedBuilder<Self, B> {
        ChainedBuilder {
            first: self,
            second: other,
        }
    }
}

/// Chained builder for combining multiple builders
#[derive(Debug)]
pub struct ChainedBuilder<A, B> {
    first: A,
    second: B,
}

impl<A, B> Builder for ChainedBuilder<A, B>
where
    A: Builder,
    B: Builder,
{
    type Output = (A::Output, B::Output);
    
    fn build(self) -> BuilderResult<Self::Output> {
        let first = self.first.build()?;
        let second = self.second.build()?;
        Ok((first, second))
    }
}

/// Builder error type
pub type BuilderResult<T> = Result<T, BuilderError>;

/// Builder errors
#[derive(Debug, thiserror::Error)]
pub enum BuilderError {
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
    
    #[error("Missing required field: {0}")]
    MissingRequired(String),
    
    #[error("Conflicting rules: {0}")]
    ConflictingRules(String),
    
    #[error("Regex compilation failed: {0}")]
    RegexError(#[from] regex::Error),
    
    #[error("Builder error: {0}")]
    Other(String),
}