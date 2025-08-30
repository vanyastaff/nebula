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
        self.left.validate(value).await?;
        
        // Validate right
        self.right.validate(value).await?;
        
        Ok(())
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "and",
            description: Some("All validators must pass"),
            category: crate::ValidatorCategory::Logical,
            tags: vec!["logical", "combinator"],
        }
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
        
        if left_result.is_ok() || right_result.is_ok() {
            Ok(())
        } else {
            // Combine error messages for better debugging
            let left_error = left_result.unwrap_err();
            let right_error = right_result.unwrap_err();
            
            Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                format!("Neither validator passed. Left: {}, Right: {}", 
                    left_error.message, right_error.message)
            ))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "or",
            description: Some("At least one validator must pass"),
            category: crate::ValidatorCategory::Logical,
            tags: vec!["logical", "combinator"],
        }
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
        
        let left_ok = left_result.is_ok();
        let right_ok = right_result.is_ok();
        
        match (left_ok, right_ok) {
            (true, false) => Ok(()),
            (false, true) => Ok(()),
            (true, true) => Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                "Both validators passed, but exactly one was expected"
            )),
            (false, false) => Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                "Neither validator passed, but exactly one was expected"
            )),
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "xor",
            description: Some("Exactly one validator must pass"),
            category: crate::ValidatorCategory::Logical,
            tags: vec!["logical", "combinator"],
        }
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
            Ok(_) => Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                "Validator passed, but was expected to fail"
            )),
            Err(_) => Ok(()),
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "not",
            description: Some("Validator must fail"),
            category: crate::ValidatorCategory::Logical,
            tags: vec!["logical", "combinator"],
        }
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
            if let Err(error) = validator.validate(value).await {
                errors.push(error);
            }
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                format!("{} validators failed", errors.len())
            ).with_details(errors))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "all",
            description: Some("All validators must pass"),
            category: crate::ValidatorCategory::Array,
            tags: vec!["array", "combinator"],
        }
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
            if validator.validate(value).await.is_ok() {
                return Ok(());
            } else if let Err(error) = validator.validate(value).await {
                errors.push(error);
            }
        }
        
        Err(ValidationError::new(
            ErrorCode::ValidationFailed,
            "No validators passed"
        ).with_details(errors))
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "any",
            description: Some("At least one validator must pass"),
            category: crate::ValidatorCategory::Array,
            tags: vec!["array", "combinator"],
        }
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
            if validator.validate(value).await.is_ok() {
                return Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    "A validator passed, but none were expected to pass"
                ));
            }
        }
        
        Ok(())
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "none",
            description: Some("No validators should pass"),
            category: crate::ValidatorCategory::Array,
            tags: vec!["array", "combinator"],
        }
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
