//! Collection validation operations

use async_trait::async_trait;
use serde_json::Value;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode};

/// CollectionLength validator - collection must have specific length
pub struct CollectionLength {
    min_length: Option<usize>,
    max_length: Option<usize>,
    exact_length: Option<usize>,
}

impl CollectionLength {
    pub fn new() -> Self {
        Self {
            min_length: None,
            max_length: None,
            exact_length: None,
        }
    }

    pub fn min(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }

    pub fn max(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }

    pub fn exact(mut self, exact: usize) -> Self {
        self.exact_length = Some(exact);
        self
    }
}

impl Default for CollectionLength {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validatable for CollectionLength {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let length = match value {
            Value::Array(arr) => arr.len(),
            Value::Object(obj) => obj.len(),
            Value::String(s) => s.len(),
            _ => {
                return ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::TypeMismatch,
                    "Expected array, object, or string"
                ).with_actual_value(value.clone())]);
            }
        };

        // Check exact length if specified
        if let Some(exact) = self.exact_length {
            if length != exact {
                return ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Collection length {} does not equal expected length {}", length, exact)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from(exact)))]);
            }
        }

        // Check minimum length if specified
        if let Some(min) = self.min_length {
            if length < min {
                return ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Collection length {} is less than minimum {}", length, min)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from(min)))]);
            }
        }

        // Check maximum length if specified
        if let Some(max) = self.max_length {
            if length > max {
                return ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::ValueOutOfRange,
                    format!("Collection length {} is greater than maximum {}", length, max)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from(max)))]);
            }
        }

        ValidationResult::success(())
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        let description = if let Some(exact) = self.exact_length {
            format!("Collection must have exact length {}", exact)
        } else {
            let mut parts = Vec::new();
            if let Some(min) = self.min_length {
                parts.push(format!("min: {}", min));
            }
            if let Some(max) = self.max_length {
                parts.push(format!("max: {}", max));
            }
            format!("Collection length constraints: {}", parts.join(", "))
        };
        
        crate::ValidatorMetadata::new(
            "collection_length",
            "collection_length",
            crate::ValidatorCategory::Collection,
        )
        .with_description(description)
        .with_tags(vec!["collection".to_string(), "length".to_string()])
    }
}

/// ArrayElement validator - validates each element in an array
pub struct ArrayElement<V> {
    validator: V,
}

impl<V> ArrayElement<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

#[async_trait]
impl<V: Validatable + Send + Sync> Validatable for ArrayElement<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Value::Array(arr) = value {
            let mut errors = Vec::new();
            
            for (index, element) in arr.iter().enumerate() {
                let result = self.validator.validate(element).await;
                if result.is_failure() {
                    for mut error in result.errors {
                        error.field_path = Some(format!("[{}]", index));
                        errors.push(error);
                    }
                }
            }
            
            if errors.is_empty() {
                ValidationResult::success(())
            } else {
                ValidationResult::failure(errors)
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected array value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "array_element",
            "array_element",
            crate::ValidatorCategory::Collection,
        )
        .with_description("Validates each element in an array")
        .with_tags(vec!["collection".to_string(), "array".to_string(), "element".to_string()])
    }
}

/// ObjectField validator - validates specific fields in an object
pub struct ObjectField {
    field_name: String,
    required: bool,
}

impl ObjectField {
    pub fn new(field_name: impl Into<String>) -> Self {
        Self {
            field_name: field_name.into(),
            required: true,
        }
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }
}

#[async_trait]
impl Validatable for ObjectField {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Value::Object(obj) = value {
            if obj.contains_key(&self.field_name) {
                ValidationResult::success(())
            } else if self.required {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::RequiredFieldMissing,
                    format!("Required field '{}' is missing", self.field_name)
                ).with_actual_value(value.clone())])
            } else {
                ValidationResult::success(())
            }
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected object value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        let description = if self.required {
            format!("Object must contain required field '{}'", self.field_name)
        } else {
            format!("Object may contain optional field '{}'", self.field_name)
        };
        
        crate::ValidatorMetadata::new(
            "object_field",
            "object_field",
            crate::ValidatorCategory::Collection,
        )
        .with_description(description)
        .with_tags(vec!["collection".to_string(), "object".to_string(), "field".to_string()])
    }
}

/// Unique validator - ensures all elements in a collection are unique
pub struct Unique;

#[async_trait]
impl Validatable for Unique {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        match value {
            Value::Array(arr) => {
                let mut seen = std::collections::HashSet::new();
                for element in arr {
                    if !seen.insert(element) {
                        return ValidationResult::failure(vec![ValidationError::new(
                            ErrorCode::ValueOutOfRange,
                            "Array contains duplicate elements"
                        ).with_actual_value(value.clone())]);
                    }
                }
                ValidationResult::success(())
            }
            Value::Object(obj) => {
                let mut seen = std::collections::HashSet::new();
                for key in obj.keys() {
                    if !seen.insert(key) {
                        return ValidationResult::failure(vec![ValidationError::new(
                            ErrorCode::ValueOutOfRange,
                            "Object contains duplicate keys"
                        ).with_actual_value(value.clone())]);
                    }
                }
                ValidationResult::success(())
            }
            _ => {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::TypeMismatch,
                    "Expected array or object value"
                ).with_actual_value(value.clone())])
            }
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "unique",
            "unique",
            crate::ValidatorCategory::Collection,
        )
        .with_description("All elements in collection must be unique")
        .with_tags(vec!["collection".to_string(), "unique".to_string()])
    }
}

/// Sorted validator - ensures collection elements are sorted
pub struct Sorted {
    ascending: bool,
}

impl Sorted {
    pub fn new() -> Self {
        Self { ascending: true }
    }

    pub fn descending(mut self) -> Self {
        self.ascending = false;
        self
    }
}

impl Default for Sorted {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validatable for Sorted {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Value::Array(arr) = value {
            if arr.len() <= 1 {
                return ValidationResult::success(());
            }
            
            // For now, we'll skip sorting validation since serde_json::Value doesn't implement PartialOrd
            // In a real implementation, you'd need to implement custom comparison logic
            let direction = if self.ascending { "ascending" } else { "descending" };
            return ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::Custom("sorting_not_implemented".to_string()),
                format!("Array sorting validation not implemented for {} order", direction)
            ).with_actual_value(value.clone())]);
        } else {
            ValidationResult::failure(vec![ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected array value"
            ).with_actual_value(value.clone())])
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        let direction = if self.ascending { "ascending" } else { "descending" };
        crate::ValidatorMetadata::new(
            "sorted",
            "sorted",
            crate::ValidatorCategory::Collection,
        )
        .with_description(format!("Array must be sorted in {} order", direction))
        .with_tags(vec!["collection".to_string(), "sorted".to_string(), direction.to_string()])
    }
}

/// ContainsAll validator - ensures collection contains all required elements
pub struct ContainsAll {
    required_elements: Vec<Value>,
}

impl ContainsAll {
    pub fn new<I>(elements: I) -> Self 
    where 
        I: IntoIterator<Item = Value>
    {
        Self {
            required_elements: elements.into_iter().collect()
        }
    }
}

#[async_trait]
impl Validatable for ContainsAll {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        match value {
            Value::Array(arr) => {
                let mut missing = Vec::new();
                for required in &self.required_elements {
                    if !arr.contains(required) {
                        missing.push(required.clone());
                    }
                }
                
                if missing.is_empty() {
                    ValidationResult::success(())
                } else {
                    ValidationResult::failure(vec![ValidationError::new(
                        ErrorCode::ValueOutOfRange,
                        format!("Array is missing required elements: {:?}", missing)
                    ).with_actual_value(value.clone())
                     .with_expected_value(Value::Array(missing))])
                }
            }
            Value::Object(obj) => {
                let mut missing = Vec::new();
                for required in &self.required_elements {
                    if let Some(key) = required.as_str() {
                        if !obj.contains_key(key) {
                            missing.push(required.clone());
                        }
                    }
                }
                
                if missing.is_empty() {
                    ValidationResult::success(())
                } else {
                    ValidationResult::failure(vec![ValidationError::new(
                        ErrorCode::ValueOutOfRange,
                        format!("Object is missing required keys: {:?}", missing)
                    ).with_actual_value(value.clone())
                     .with_expected_value(Value::Array(missing))])
                }
            }
            _ => {
                ValidationResult::failure(vec![ValidationError::new(
                    ErrorCode::TypeMismatch,
                    "Expected array or object value"
                ).with_actual_value(value.clone())])
            }
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata::new(
            "contains_all",
            "contains_all",
            crate::ValidatorCategory::Collection,
        )
        .with_description(format!("Collection must contain all {} required elements", self.required_elements.len()))
        .with_tags(vec!["collection".to_string(), "contains_all".to_string()])
    }
}
