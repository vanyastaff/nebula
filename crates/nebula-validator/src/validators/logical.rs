//! Logical validation combinators

use async_trait::async_trait;
use serde_json::Value;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode};

/// AND combinator - all validators must pass
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
impl<L: Validatable, R: Validatable> Validatable for And<L, R> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // Validate left first
        let left_result = self.left.validate(value).await;
        if left_result.is_failure() {
            return left_result;
        }
        
        // Validate right
        let right_result = self.right.validate(value).await;
        if right_result.is_failure() {
            return right_result;
        }
        
        ValidationResult::success(())
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "and",
            "and",
            crate::ValidatorCategory::Logical,
        )
        .with_description("All validators must pass")
        .with_tags(vec!["logical".to_string(), "combinator".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        std::cmp::max(self.left.complexity(), self.right.complexity())
    }
}

/// OR combinator - at least one validator must pass
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
impl<L: Validatable, R: Validatable> Validatable for Or<L, R> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let left_result = self.left.validate(value).await;
        let right_result = self.right.validate(value).await;
        
        if left_result.is_success() || right_result.is_success() {
            ValidationResult::success(())
        } else {
            // Combine error messages for better debugging
            let mut all_errors = Vec::new();
            all_errors.extend(left_result.errors);
            all_errors.extend(right_result.errors);
            
            ValidationResult::failure(all_errors)
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "or",
            "or",
            crate::ValidatorCategory::Logical,
        )
        .with_description("At least one validator must pass")
        .with_tags(vec!["logical".to_string(), "combinator".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        std::cmp::max(self.left.complexity(), self.right.complexity())
    }
}

/// XOR combinator - exactly one validator must pass
pub struct Xor<L, R> {
    left: L,
    right: R,
}

impl<L, R> Xor<L, R> {
    pub fn new(left: L, right: R) -> Self {
        Self { left, right }
    }
}

#[async_trait]
impl<L: Validatable, R: Validatable> Validatable for Xor<L, R> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let left_result = self.left.validate(value).await;
        let right_result = self.right.validate(value).await;
        
        let left_ok = left_result.is_success();
        let right_ok = right_result.is_success();
        
        match (left_ok, right_ok) {
            (true, false) => ValidationResult::success(()),
            (false, true) => ValidationResult::success(()),
            (true, true) => ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::XorValidationFailed,
                "Both validators passed, but exactly one was expected"
            )]),
            (false, false) => ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::XorValidationFailed,
                "Neither validator passed, but exactly one was expected"
            )]),
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "xor",
            "xor",
            crate::ValidatorCategory::Logical,
        )
        .with_description("Exactly one validator must pass")
        .with_tags(vec!["logical".to_string(), "combinator".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        std::cmp::max(self.left.complexity(), self.right.complexity())
    }
}

/// NOT combinator - validator must fail
pub struct Not<V> {
    validator: V,
}

impl<V> Not<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

#[async_trait]
impl<V: Validatable> Validatable for Not<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        match self.validator.validate(value).await {
            result if result.is_success() => ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::XorValidationFailed,
                "Validator passed, but was expected to fail"
            )]),
            _ => ValidationResult::success(()),
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "not",
            "not",
            crate::ValidatorCategory::Logical,
        )
        .with_description("Validator must fail")
        .with_tags(vec!["logical".to_string(), "combinator".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        self.validator.complexity()
    }
}

/// ALL combinator - all validators in array must pass
pub struct All<V> {
    validators: Vec<V>,
}

impl<V> All<V> {
    pub fn new(validators: Vec<V>) -> Self {
        Self { validators }
    }
    
    pub fn add(mut self, validator: V) -> Self {
        self.validators.push(validator);
        self
    }
}

#[async_trait]
impl<V: Validatable> Validatable for All<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let mut errors = Vec::new();
        
        for validator in &self.validators {
            let result = validator.validate(value).await;
            if result.is_failure() {
                errors.extend(result.errors);
            }
        }
        
        if errors.is_empty() {
            ValidationResult::success(())
        } else {
            ValidationResult::failure(errors)
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "all",
            "all",
            crate::ValidatorCategory::Array,
        )
        .with_description("All validators must pass")
        .with_tags(vec!["array".to_string(), "combinator".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        if self.validators.is_empty() {
            crate::ValidationComplexity::Simple
        } else {
            let max_complexity = self.validators.iter()
                .map(|v| v.complexity())
                .max()
                .unwrap_or(crate::ValidationComplexity::Simple);
            
            match self.validators.len() {
                1..=3 => max_complexity,
                4..=10 => std::cmp::max(max_complexity, crate::ValidationComplexity::Moderate),
                _ => std::cmp::max(max_complexity, crate::ValidationComplexity::Complex),
            }
        }
    }
}

/// ANY combinator - at least one validator must pass
pub struct Any<V> {
    validators: Vec<V>,
}

impl<V> Any<V> {
    pub fn new(validators: Vec<V>) -> Self {
        Self { validators }
    }
    
    pub fn add(mut self, validator: V) -> Self {
        self.validators.push(validator);
        self
    }
}

#[async_trait]
impl<V: Validatable> Validatable for Any<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let mut errors = Vec::new();
        
        for validator in &self.validators {
            let result = validator.validate(value).await;
            if result.is_success() {
                return ValidationResult::success(());
            } else {
                errors.extend(result.errors);
            }
        }
        
        ValidationResult::failure(errors)
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "any",
            "any",
            crate::ValidatorCategory::Array,
        )
        .with_description("At least one validator must pass")
        .with_tags(vec!["array".to_string(), "combinator".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        if self.validators.is_empty() {
            crate::ValidationComplexity::Simple
        } else {
            let max_complexity = self.validators.iter()
                .map(|v| v.complexity())
                .max()
                .unwrap_or(crate::ValidationComplexity::Simple);
            
            match self.validators.len() {
                1..=3 => max_complexity,
                4..=10 => std::cmp::max(max_complexity, crate::ValidationComplexity::Moderate),
                _ => std::cmp::max(max_complexity, crate::ValidationComplexity::Complex),
            }
        }
    }
}

/// NONE combinator - no validators should pass
pub struct None<V> {
    validators: Vec<V>,
}

impl<V> None<V> {
    pub fn new(validators: Vec<V>) -> Self {
        Self { validators }
    }
    
    pub fn add(mut self, validator: V) -> Self {
        self.validators.push(validator);
        self
    }
}

#[async_trait]
impl<V: Validatable> Validatable for None<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        for validator in &self.validators {
            let result = validator.validate(value).await;
            if result.is_success() {
                return ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::XorValidationFailed,
                    "A validator passed, but none were expected to pass"
                )]);
            }
        }
        
        ValidationResult::success(())
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "none",
            "none",
            crate::ValidatorCategory::Array,
        )
        .with_description("No validators should pass")
        .with_tags(vec!["array".to_string(), "combinator".to_string()])
    }
    
    fn complexity(&self) -> crate::ValidationComplexity {
        if self.validators.is_empty() {
            crate::ValidationComplexity::Simple
        } else {
            let max_complexity = self.validators.iter()
                .map(|v| v.complexity())
                .max()
                .unwrap_or(crate::ValidationComplexity::Simple);
            
            match self.validators.len() {
                1..=3 => max_complexity,
                4..=10 => std::cmp::max(max_complexity, crate::ValidationComplexity::Moderate),
                _ => std::cmp::max(max_complexity, crate::ValidationComplexity::Complex),
            }
        }
    }
}
