//! Logical combinators for combining validators
//! 
//! This module provides logical operators (AND, OR, NOT, XOR) for combining
//! multiple validators into complex validation chains.

use async_trait::async_trait;
use serde_json::Value;
use crate::types::{ValidationResult, ValidationError, ValidatorMetadata, ValidationComplexity, ErrorCode};
use crate::traits::Validatable;

// ==================== AND Validator ====================

/// AND validator that combines two validators
/// 
/// Both validators must pass for the combined validator to succeed.
pub struct And<L, R> {
    left: L,
    right: R,
}

impl<L, R> And<L, R> {
    /// Create new AND validator
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

#[async_trait]
impl<L, R> Validatable for And<L, R>
where
    L: Validatable + Send + Sync,
    R: Validatable + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let left_result = self.left.validate(value).await;
        if !left_result.is_success() {
            return left_result;
        }
        
        self.right.validate(value).await
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let left_meta = self.left.metadata();
        let right_meta = self.right.metadata();
        
        ValidatorMetadata::new(
            format!("{}_AND_{}", left_meta.id.as_str(), right_meta.id.as_str()),
            format!("{} AND {}", left_meta.name, right_meta.name),
            crate::types::ValidatorCategory::Logical,
        )
        .with_description("Combines two validators with AND logic")
        .with_tags(vec!["logical".to_string(), "composite".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        let left_complexity = self.left.complexity() as u8;
        let right_complexity = self.right.complexity() as u8;
        let combined = (left_complexity + right_complexity).min(4);
        
        match combined {
            1 => ValidationComplexity::Simple,
            2 => ValidationComplexity::Moderate,
            3 => ValidationComplexity::Complex,
            _ => ValidationComplexity::VeryComplex,
        }
    }
}

// ==================== OR Validator ====================

/// OR validator that combines two validators
/// 
/// At least one validator must pass for the combined validator to succeed.
pub struct Or<L, R> {
    left: L,
    right: R,
}

impl<L, R> Or<L, R> {
    /// Create new OR validator
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

#[async_trait]
impl<L, R> Validatable for Or<L, R>
where
    L: Validatable + Send + Sync,
    R: Validatable + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let left_result = self.left.validate(value).await;
        if left_result.is_success() {
            return left_result;
        }
        
        self.right.validate(value).await
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let left_meta = self.left.metadata();
        let right_meta = self.right.metadata();
        
        ValidatorMetadata::new(
            format!("{}_OR_{}", left_meta.id.as_str(), right_meta.id.as_str()),
            format!("{} OR {}", left_meta.name, right_meta.name),
            crate::types::ValidatorCategory::Logical,
        )
        .with_description("Combines two validators with OR logic")
        .with_tags(vec!["logical".to_string(), "composite".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        let left_complexity = self.left.complexity() as u8;
        let right_complexity = self.right.complexity() as u8;
        let combined = (left_complexity + right_complexity).min(4);
        
        match combined {
            1 => ValidationComplexity::Simple,
            2 => ValidationComplexity::Moderate,
            3 => ValidationComplexity::Complex,
            _ => ValidationComplexity::VeryComplex,
        }
    }
}

// ==================== NOT Validator ====================

/// NOT validator that negates another validator
/// 
/// The negated validator succeeds when the original fails and vice versa.
pub struct Not<V> {
    validator: V,
}

impl<V> Not<V> {
    /// Create new NOT validator
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

#[async_trait]
impl<V> Validatable for Not<V>
where
    V: Validatable + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let result = self.validator.validate(value).await;
        if result.is_success() {
            ValidationResult::failure(vec![
                ValidationError::new(
                    ErrorCode::Custom("negation".to_string()),
                    "Value passed validation but NOT validator expected failure"
                )
            ])
        } else {
            ValidationResult::success(())
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let validator_meta = self.validator.metadata();
        
        ValidatorMetadata::new(
            format!("NOT_{}", validator_meta.id.as_str()),
            format!("NOT {}", validator_meta.name),
            crate::types::ValidatorCategory::Logical,
        )
        .with_description("Negates the result of another validator")
        .with_tags(vec!["logical".to_string(), "negation".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        self.validator.complexity()
    }
}

// ==================== XOR Validator ====================

/// XOR validator that requires exactly one validator to pass
/// 
/// Exactly one validator must pass for the combined validator to succeed.
pub struct Xor<L, R> {
    left: L,
    right: R,
}

impl<L, R> Xor<L, R> {
    /// Create new XOR validator
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

#[async_trait]
impl<L, R> Validatable for Xor<L, R>
where
    L: Validatable + Send + Sync,
    R: Validatable + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let left_result = self.left.validate(value).await;
        let right_result = self.right.validate(value).await;
        
        let left_success = left_result.is_success();
        let right_success = right_result.is_success();
        
        match (left_success, right_success) {
            (true, false) => left_result,
            (false, true) => right_result,
            (true, true) => ValidationResult::failure(vec![
                ValidationError::new(
                    ErrorCode::Custom("xor_violation".to_string()),
                    "Both validators passed, but XOR requires exactly one to pass"
                )
            ]),
            (false, false) => {
                // Combine errors from both validators
                let mut all_errors = left_result.errors.clone();
                all_errors.extend(right_result.errors);
                ValidationResult::failure(all_errors)
            }
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let left_meta = self.left.metadata();
        let right_meta = self.right.metadata();
        
        ValidatorMetadata::new(
            format!("{}_XOR_{}", left_meta.id.as_str(), right_meta.id.as_str()),
            format!("{} XOR {}", left_meta.name, right_meta.name),
            crate::types::ValidatorCategory::Logical,
        )
        .with_description("Requires exactly one validator to pass (XOR logic)")
        .with_tags(vec!["logical".to_string(), "composite".to_string(), "xor".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        let left_complexity = self.left.complexity() as u8;
        let right_complexity = self.right.complexity() as u8;
        let combined = (left_complexity + right_complexity).min(4);
        
        match combined {
            1 => ValidationComplexity::Simple,
            2 => ValidationComplexity::Moderate,
            3 => ValidationComplexity::Complex,
            _ => ValidationComplexity::VeryComplex,
        }
    }
}

// ==================== When Validator ====================

/// When validator that only runs when a condition is met
/// 
/// The validation only runs when the condition is met.
pub struct When<C, V> {
    condition: C,
    validator: V,
}

impl<C, V> When<C, V> {
    /// Create new when validator
    pub fn new(condition: C, validator: V) -> Self {
        Self { condition, validator }
    }
}

#[async_trait]
impl<C, V> Validatable for When<C, V>
where
    C: Validatable + Send + Sync,
    V: Validatable + Send + Sync,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let condition_result = self.condition.validate(value).await;
        if condition_result.is_success() {
            self.validator.validate(value).await
        } else {
            ValidationResult::success(())
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        let condition_meta = self.condition.metadata();
        let validator_meta = self.validator.metadata();
        
        ValidatorMetadata::new(
            format!("WHEN_{}_THEN_{}", condition_meta.id.as_str(), validator_meta.id.as_str()),
            format!("WHEN {} THEN {}", condition_meta.name, validator_meta.name),
            crate::types::ValidatorCategory::Conditional,
        )
        .with_description("Conditionally applies validation based on a condition")
        .with_tags(vec!["conditional".to_string(), "control".to_string()])
    }
    
    fn complexity(&self) -> ValidationComplexity {
        let condition_complexity = self.condition.complexity() as u8;
        let validator_complexity = self.validator.complexity() as u8;
        let combined = (condition_complexity + validator_complexity).min(4);
        
        match combined {
            1 => ValidationComplexity::Simple,
            2 => ValidationComplexity::Moderate,
            3 => ValidationComplexity::Complex,
            _ => ValidationComplexity::VeryComplex,
        }
    }
}
