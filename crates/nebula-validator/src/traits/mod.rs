//! Core traits for the nebula-validator framework
//! 
//! This module defines the fundamental traits that power the validation system:
//! - `Validatable` - Main trait for validators that work with `serde_json::Value`
//! - `Validator<T>` - Generic validator trait for typed values
//! - `AsyncValidator<T>` - Optimized async validator without boxing
//! - `ValidatableExt` - Extension trait with combinators
//! - `ContextAwareValidator` - Validators that use context
//! - `StateAwareValidator` - Validators with state

mod validatable;
mod validator;
mod async_validator;
mod combinators;
mod context_aware;
mod state_aware;
mod with_validator;

// Re-export all traits
pub use validatable::{Validatable, ValidatableClone};
pub use validator::{Validator, TypedValidator, ValidatorClone};
pub use async_validator::{AsyncValidator, AsyncValidatorExt};
pub use combinators::{ValidatableExt, CombinatorExt};
pub use context_aware::{ContextAwareValidator, ContextValidator};
pub use state_aware::{StateAwareValidator, StatefulValidator};
pub use with_validator::{
    WithValidator, WithValidatorExt, ValidatedType, 
    SelfValidating, ValidatedBuilder, LazyValidator,
    ConditionalValidator, AutoValidated,
};

// Common trait bounds
pub trait ValidatorBase: Send + Sync + std::fmt::Debug {}
impl<T> ValidatorBase for T where T: Send + Sync + std::fmt::Debug {}

// Prelude for convenient imports
pub mod prelude {
    pub use super::{
        Validatable, Validator, AsyncValidator,
        ValidatableExt, ContextAwareValidator, StateAwareValidator,
    };
}