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
                return Err(ValidationError::new(
                    ErrorCode::TypeMismatch,
                    "Expected array, object, or string"
                ).with_actual_value(value.clone()));
            }
        };

        // Check exact length if specified
        if let Some(exact) = self.exact_length {
            if length != exact {
                return Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Collection length {} does not equal expected length {}", length, exact)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from(exact))));
            }
        }

        // Check minimum length if specified
        if let Some(min) = self.min_length {
            if length < min {
                return Err(ValidationError::new(
                    ErrorCode::CollectionTooSmall,
                    format!("Collection length {} is less than minimum {}", length, min)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from(min))));
            }
        }

        // Check maximum length if specified
        if let Some(max) = self.max_length {
            if length > max {
                return Err(ValidationError::new(
                    ErrorCode::CollectionTooLarge,
                    format!("Collection length {} is greater than maximum {}", length, max)
                ).with_actual_value(value.clone())
                 .with_expected_value(Value::Number(serde_json::Number::from(max))));
            }
        }

        Ok(())
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
        
        crate::ValidatorMetadata {
            name: "collection_length",
            description: Some(&description),
            category: crate::ValidatorCategory::Collection,
            tags: vec!["collection", "length"],
        }
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
                if let Err(error) = self.validator.validate(element).await {
                    let mut field_error = error;
                    field_error.field_path = format!("[{}]", index);
                    errors.push(field_error);
                }
            }
            
            if errors.is_empty() {
                Ok(())
            } else {
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("{} array elements failed validation", errors.len())
                ).with_details(errors))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected array value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "array_element",
            description: Some("Validates each element in an array"),
            category: crate::ValidatorCategory::Collection,
            tags: vec!["collection", "array", "element"],
        }
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
                Ok(())
            } else if self.required {
                Err(ValidationError::new(
                    ErrorCode::RequiredFieldMissing,
                    format!("Required field '{}' is missing", self.field_name)
                ).with_actual_value(value.clone()))
            } else {
                Ok(())
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected object value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        let description = if self.required {
            format!("Object must contain required field '{}'", self.field_name)
        } else {
            format!("Object may contain optional field '{}'", self.field_name)
        };
        
        crate::ValidatorMetadata {
            name: "object_field",
            description: Some(&description),
            category: crate::ValidatorCategory::Collection,
            tags: vec!["collection", "object", "field"],
        }
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
                        return Err(ValidationError::new(
                            ErrorCode::ValidationFailed,
                            "Array contains duplicate elements"
                        ).with_actual_value(value.clone()));
                    }
                }
                Ok(())
            }
            Value::Object(obj) => {
                let mut seen = std::collections::HashSet::new();
                for key in obj.keys() {
                    if !seen.insert(key) {
                        return Err(ValidationError::new(
                            ErrorCode::ValidationFailed,
                            "Object contains duplicate keys"
                        ).with_actual_value(value.clone()));
                    }
                }
                Ok(())
            }
            _ => {
                Err(ValidationError::new(
                    ErrorCode::TypeMismatch,
                    "Expected array or object value"
                ).with_actual_value(value.clone()))
            }
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "unique",
            description: Some("All elements in collection must be unique"),
            category: crate::ValidatorCategory::Collection,
            tags: vec!["collection", "unique"],
        }
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
                return Ok(());
            }
            
            let mut sorted = arr.clone();
            if self.ascending {
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            } else {
                sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            }
            
            if arr == &sorted {
                Ok(())
            } else {
                let direction = if self.ascending { "ascending" } else { "descending" };
                Err(ValidationError::new(
                    ErrorCode::ValidationFailed,
                    format!("Array is not sorted in {} order", direction)
                ).with_actual_value(value.clone()))
            }
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected array value"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        let direction = if self.ascending { "ascending" } else { "descending" };
        crate::ValidatorMetadata {
            name: "sorted",
            description: Some(&format!("Array must be sorted in {} order", direction)),
            category: crate::ValidatorCategory::Collection,
            tags: vec!["collection", "sorted", direction],
        }
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
                    Ok(())
                } else {
                    Err(ValidationError::new(
                        ErrorCode::ValidationFailed,
                        format!("Array is missing required elements: {:?}", missing)
                    ).with_actual_value(value.clone())
                     .with_expected_value(Value::Array(missing)))
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
                    Ok(())
                } else {
                    Err(ValidationError::new(
                        ErrorCode::ValidationFailed,
                        format!("Object is missing required keys: {:?}", missing)
                    ).with_actual_value(value.clone())
                     .with_expected_value(Value::Array(missing)))
                }
            }
            _ => {
                Err(ValidationError::new(
                    ErrorCode::TypeMismatch,
                    "Expected array or object value"
                ).with_actual_value(value.clone()))
            }
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "contains_all",
            description: Some(&format!("Collection must contain all {} required elements", self.required_elements.len())),
            category: crate::ValidatorCategory::Collection,
            tags: vec!["collection", "contains_all"],
        }
    }
}
