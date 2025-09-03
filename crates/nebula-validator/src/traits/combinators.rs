//! Extension traits with combinator methods for validators

use async_trait::async_trait;
use serde_json::Value;
use crate::types::ValidationResult;
use super::Validatable;

/// Extension trait providing combinator methods for Validatable
pub trait ValidatableExt: Validatable + Sized {
    /// Combine with another validator using AND logic
    fn and<V>(self, other: V) -> And<Self, V>
    where
        V: Validatable,
    {
        And::new(self, other)
    }
    
    /// Combine with another validator using OR logic
    fn or<V>(self, other: V) -> Or<Self, V>
    where
        V: Validatable,
    {
        Or::new(self, other)
    }
    
    /// Negate the validator
    fn not(self) -> Not<Self> {
        Not::new(self)
    }
    
    /// Apply validator only when condition is met
    fn when<C>(self, condition: C) -> When<C, Self>
    where
        C: Validatable,
    {
        When::new(condition, self)
    }
    
    /// Apply validator unless condition is met
    fn unless<C>(self, condition: C) -> Unless<C, Self>
    where
        C: Validatable,
    {
        Unless::new(condition, self)
    }
    
    /// Make the validator optional
    fn optional(self) -> Optional<Self> {
        Optional::new(self)
    }
    
    /// Add a default value on failure
    fn or_default(self, default: Value) -> WithDefault<Self> {
        WithDefault::new(self, default)
    }
    
    /// Retry on failure
    fn retry(self, attempts: usize) -> Retry<Self> {
        Retry::new(self, attempts)
    }
    
    /// Add timeout
    fn timeout(self, duration: std::time::Duration) -> Timeout<Self> {
        Timeout::new(self, duration)
    }
    
    /// Cache results
    fn cached(self, ttl: std::time::Duration) -> Cached<Self> {
        Cached::new(self, ttl)
    }
    
    /// Add logging
    fn logged(self, level: tracing::Level) -> Logged<Self> {
        Logged::new(self, level)
    }
}

// Implement for all Validatable types
impl<T> ValidatableExt for T where T: Validatable + Sized {}

// ==================== Combinator Implementations ====================

/// AND combinator
#[derive(Debug, Clone)]
pub struct And<L, R> {
    left: L,
    right: R,
}

impl<L, R> And<L, R> {
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

#[async_trait]
impl<L, R> Validatable for And<L, R>
where
    L: Validatable,
    R: Validatable,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        self.left.validate(value).await?;
        self.right.validate(value).await
    }
    
    fn metadata(&self) -> crate::types::ValidatorMetadata {
        crate::types::ValidatorMetadata::new(
            format!("and_{}_{}", self.left.id(), self.right.id()),
            format!("{} AND {}", self.left.name(), self.right.name()),
            crate::types::ValidatorCategory::Logical,
        )
    }
    
    fn complexity(&self) -> crate::types::ValidationComplexity {
        self.left.complexity().add(self.right.complexity())
    }
}

/// OR combinator
#[derive(Debug, Clone)]
pub struct Or<L, R> {
    left: L,
    right: R,
}

impl<L, R> Or<L, R> {
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

#[async_trait]
impl<L, R> Validatable for Or<L, R>
where
    L: Validatable,
    R: Validatable,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        match self.left.validate(value).await {
            Ok(()) => Ok(()),
            Err(_) => self.right.validate(value).await,
        }
    }
    
    fn metadata(&self) -> crate::types::ValidatorMetadata {
        crate::types::ValidatorMetadata::new(
            format!("or_{}_{}", self.left.id(), self.right.id()),
            format!("{} OR {}", self.left.name(), self.right.name()),
            crate::types::ValidatorCategory::Logical,
        )
    }
}

/// NOT combinator
#[derive(Debug, Clone)]
pub struct Not<V> {
    validator: V,
}

impl<V> Not<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

#[async_trait]
impl<V> Validatable for Not<V>
where
    V: Validatable,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        match self.validator.validate(value).await {
            Ok(()) => ValidationResult::error(crate::types::ValidationError::new(
                crate::types::ErrorCode::PredicateFailed,
                "NOT condition failed - validator succeeded when it should have failed",
            )),
            Err(_) => ValidationResult::success(()),
        }
    }
    
    fn metadata(&self) -> crate::types::ValidatorMetadata {
        crate::types::ValidatorMetadata::new(
            format!("not_{}", self.validator.id()),
            format!("NOT {}", self.validator.name()),
            crate::types::ValidatorCategory::Logical,
        )
    }
}

/// When conditional combinator
#[derive(Debug, Clone)]
pub struct When<C, V> {
    condition: C,
    validator: V,
}

impl<C, V> When<C, V> {
    pub fn new(condition: C, validator: V) -> Self {
        Self { condition, validator }
    }
}

#[async_trait]
impl<C, V> Validatable for When<C, V>
where
    C: Validatable,
    V: Validatable,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if self.condition.validate(value).await.is_ok() {
            self.validator.validate(value).await
        } else {
            ValidationResult::success(())
        }
    }
    
    fn metadata(&self) -> crate::types::ValidatorMetadata {
        crate::types::ValidatorMetadata::new(
            format!("when_{}_{}", self.condition.id(), self.validator.id()),
            format!("When {} then {}", self.condition.name(), self.validator.name()),
            crate::types::ValidatorCategory::Conditional,
        )
    }
}

/// Unless conditional combinator
#[derive(Debug, Clone)]
pub struct Unless<C, V> {
    condition: C,
    validator: V,
}

impl<C, V> Unless<C, V> {
    pub fn new(condition: C, validator: V) -> Self {
        Self { condition, validator }
    }
}

#[async_trait]
impl<C, V> Validatable for Unless<C, V>
where
    C: Validatable,
    V: Validatable,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if self.condition.validate(value).await.is_err() {
            self.validator.validate(value).await
        } else {
            ValidationResult::success(())
        }
    }
    
    fn metadata(&self) -> crate::types::ValidatorMetadata {
        crate::types::ValidatorMetadata::new(
            format!("unless_{}_{}", self.condition.id(), self.validator.id()),
            format!("Unless {} then {}", self.condition.name(), self.validator.name()),
            crate::types::ValidatorCategory::Conditional,
        )
    }
}

/// Optional validator
#[derive(Debug, Clone)]
pub struct Optional<V> {
    validator: V,
}

impl<V> Optional<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

#[async_trait]
impl<V> Validatable for Optional<V>
where
    V: Validatable,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if value.is_null() {
            ValidationResult::success(())
        } else {
            self.validator.validate(value).await
        }
    }
    
    fn metadata(&self) -> crate::types::ValidatorMetadata {
        crate::types::ValidatorMetadata::new(
            format!("optional_{}", self.validator.id()),
            format!("Optional {}", self.validator.name()),
            crate::types::ValidatorCategory::Basic,
        )
    }
}

// Additional combinators (simplified implementations)

/// WithDefault combinator
#[derive(Debug, Clone)]
pub struct WithDefault<V> {
    validator: V,
    default: Value,
}

impl<V> WithDefault<V> {
    pub fn new(validator: V, default: Value) -> Self {
        Self { validator, default }
    }
}

/// Retry combinator
#[derive(Debug, Clone)]
pub struct Retry<V> {
    validator: V,
    attempts: usize,
}

impl<V> Retry<V> {
    pub fn new(validator: V, attempts: usize) -> Self {
        Self { validator, attempts }
    }
}

/// Timeout combinator
#[derive(Debug, Clone)]
pub struct Timeout<V> {
    validator: V,
    duration: std::time::Duration,
}

impl<V> Timeout<V> {
    pub fn new(validator: V, duration: std::time::Duration) -> Self {
        Self { validator, duration }
    }
}

/// Cached combinator
#[derive(Debug, Clone)]
pub struct Cached<V> {
    validator: V,
    ttl: std::time::Duration,
}

impl<V> Cached<V> {
    pub fn new(validator: V, ttl: std::time::Duration) -> Self {
        Self { validator, ttl }
    }
}

/// Logged combinator
#[derive(Debug, Clone)]
pub struct Logged<V> {
    validator: V,
    level: tracing::Level,
}

impl<V> Logged<V> {
    pub fn new(validator: V, level: tracing::Level) -> Self {
        Self { validator, level }
    }
}

/// Additional extension trait for advanced combinators
pub trait CombinatorExt: ValidatableExt {
    
}

impl<T> CombinatorExt for T where T: ValidatableExt {}