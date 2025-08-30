//! Set membership validation operations

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode};

/// In validator - value must be in the allowed set
pub struct In<T> {
    allowed_values: HashSet<T>,
}

impl<T: Clone + Eq + std::hash::Hash> In<T> {
    pub fn new<I>(values: I) -> Self 
    where 
        I: IntoIterator<Item = T>
    {
        Self { 
            allowed_values: values.into_iter().collect() 
        }
    }
}

#[async_trait]
impl<T: Clone + Eq + std::hash::Hash + Send + Sync> Validatable for In<T> 
where 
    T: serde::Serialize
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // Try to deserialize the value to type T
        let deserialized: Result<T, _> = serde_json::from_value(value.clone());
        
        match deserialized {
            Ok(val) => {
                if self.allowed_values.contains(&val) {
                    Ok(())
                } else {
                    let allowed: Vec<&T> = self.allowed_values.iter().collect();
                    Err(ValidationError::new(
                        ErrorCode::ValidationFailed,
                        format!("Value is not in allowed set: {:?}", allowed)
                    ).with_actual_value(value.clone())
                     .with_expected_value(Value::Array(
                         allowed.iter()
                             .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                             .collect()
                     )))
                }
            }
            Err(_) => Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Value type does not match expected type"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "in",
            description: Some(&format!("Value must be in allowed set of {} items", self.allowed_values.len())),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["set", "membership"],
        }
    }
}

/// NotIn validator - value must not be in the forbidden set
pub struct NotIn<T> {
    forbidden_values: HashSet<T>,
}

impl<T: Clone + Eq + std::hash::Hash> NotIn<T> {
    pub fn new<I>(values: I) -> Self 
    where 
        I: IntoIterator<Item = T>
    {
        Self { 
            forbidden_values: values.into_iter().collect() 
        }
    }
}

#[async_trait]
impl<T: Clone + Eq + std::hash::Hash + Send + Sync> Validatable for NotIn<T> 
where 
    T: serde::Serialize
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // Try to deserialize the value to type T
        let deserialized: Result<T, _> = serde_json::from_value(value.clone());
        
        match deserialized {
            Ok(val) => {
                if !self.forbidden_values.contains(&val) {
                    Ok(())
                } else {
                    let forbidden: Vec<&T> = self.forbidden_values.iter().collect();
                    Err(ValidationError::new(
                        ErrorCode::ValidationFailed,
                        format!("Value is in forbidden set: {:?}", forbidden)
                    ).with_actual_value(value.clone())
                     .with_expected_value(Value::Array(
                         forbidden.iter()
                             .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                             .collect()
                     )))
                }
            }
            Err(_) => Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Value type does not match expected type"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "not_in",
            description: Some(&format!("Value must not be in forbidden set of {} items", self.forbidden_values.len())),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["set", "exclusion"],
        }
    }
}

/// StringIn validator - string must be in the allowed set (optimized for strings)
pub struct StringIn {
    allowed_values: HashSet<String>,
}

impl StringIn {
    pub fn new<I>(values: I) -> Self 
    where 
        I: IntoIterator<Item = impl Into<String>>
    {
        Self { 
            allowed_values: values.into_iter().map(|v| v.into()).collect() 
        }
    }
}

#[async_trait]
impl Validatable for StringIn {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if self.allowed_values.contains(s) {
                Ok(())
            } else {
                let allowed: Vec<&String> = self.allowed_values.iter().collect();
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("String '{}' is not in allowed set: {:?}", s, allowed)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Array(
                     allowed.iter()
                         .map(|v| Value::String(v.clone()))
                         .collect()
                 )))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "string_in",
            description: Some(&format!("String must be in allowed set of {} items", self.allowed_values.len())),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["set", "string", "membership"],
        }
    }
}

/// StringNotIn validator - string must not be in the forbidden set (optimized for strings)
pub struct StringNotIn {
    forbidden_values: HashSet<String>,
}

impl StringNotIn {
    pub fn new<I>(values: I) -> Self 
    where 
        I: IntoIterator<Item = impl Into<String>>
    {
        Self { 
            forbidden_values: values.into_iter().map(|v| v.into()).collect() 
        }
    }
}

#[async_trait]
impl Validatable for StringNotIn {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(s) = value.as_str() {
            if !self.forbidden_values.contains(s) {
                Ok(())
            } else {
                let forbidden: Vec<&String> = self.forbidden_values.iter().collect();
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("String '{}' is in forbidden set: {:?}", s, forbidden)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Array(
                     forbidden.iter()
                         .map(|v| Value::String(v.clone()))
                         .collect()
                 )))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected string value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "string_not_in",
            description: Some(&format!("String must not be in forbidden set of {} items", self.forbidden_values.len())),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["set", "string", "exclusion"],
        }
    }
}

/// NumberIn validator - number must be in the allowed set (optimized for numbers)
pub struct NumberIn {
    allowed_values: HashSet<f64>,
}

impl NumberIn {
    pub fn new<I>(values: I) -> Self 
    where 
        I: IntoIterator<Item = f64>
    {
        Self { 
            allowed_values: values.into_iter().collect() 
        }
    }
}

#[async_trait]
impl Validatable for NumberIn {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if self.allowed_values.contains(&n) {
                Ok(())
            } else {
                let allowed: Vec<&f64> = self.allowed_values.iter().collect();
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Number {} is not in allowed set: {:?}", n, allowed)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Array(
                     allowed.iter()
                         .map(|v| Value::Number(serde_json::Number::from_f64(*v).unwrap()))
                         .collect()
                 )))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "number_in",
            description: Some(&format!("Number must be in allowed set of {} items", self.allowed_values.len())),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["set", "number", "membership"],
        }
    }
}

/// NumberNotIn validator - number must not be in the forbidden set (optimized for numbers)
pub struct NumberNotIn {
    forbidden_values: HashSet<f64>,
}

impl NumberNotIn {
    pub fn new<I>(values: I) -> Self 
    where 
        I: IntoIterator<Item = f64>
    {
        Self { 
            forbidden_values: values.into_iter().collect() 
        }
    }
}

#[async_trait]
impl Validatable for NumberNotIn {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Some(n) = value.as_f64() {
            if !self.forbidden_values.contains(&n) {
                Ok(())
            } else {
                let forbidden: Vec<&f64> = self.forbidden_values.iter().collect();
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Number {} is in forbidden set: {:?}", n, forbidden)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Array(
                     forbidden.iter()
                         .map(|v| Value::Number(serde_json::Number::from_f64(*v).unwrap()))
                         .collect()
                 )))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "number_not_in",
            description: Some(&format!("Number must not be in forbidden set of {} items", self.forbidden_values.len())),
            category: crate::ValidatorCategory::Basic,
            tags: vec!["set", "number", "exclusion"],
        }
    }
}
