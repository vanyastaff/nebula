//! Cross-field validation operations

use async_trait::async_trait;
use serde_json::Value;
use crate::{Validatable, ValidationResult, ValidationError, ErrorCode, ValidationContext};

/// CrossFieldValidator - validates a field based on other fields in the context
pub struct CrossFieldValidator<F> {
    field_path: String,
    validator: F,
}

impl<F> CrossFieldValidator<F> {
    pub fn new(field_path: impl Into<String>, validator: F) -> Self {
        Self {
            field_path: field_path.into(),
            validator,
        }
    }
}

#[async_trait]
impl<F: Validatable + Send + Sync> Validatable for CrossFieldValidator<F> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator requires a ValidationContext to access other fields
        // For now, we'll validate the current value, but in practice this would
        // be used with a ValidationContext that provides access to the root object
        self.validator.validate(value).await
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "cross_field",
            description: Some(&format!("Cross-field validation for field: {}", self.field_path)),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "context"],
        }
    }
}

/// EqualsField validator - field value must equal another field's value
pub struct EqualsField {
    other_field: String,
}

impl EqualsField {
    pub fn new(other_field: impl Into<String>) -> Self {
        Self {
            other_field: other_field.into(),
        }
    }
}

#[async_trait]
impl Validatable for EqualsField {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is not null
        if !value.is_null() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                format!("Field value must equal field '{}'", self.other_field)
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "equals_field",
            description: Some(&format!("Field value must equal field '{}'", self.other_field)),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "equals"],
        }
    }
}

/// GreaterThanField validator - field value must be greater than another field's value
pub struct GreaterThanField {
    other_field: String,
}

impl GreaterThanField {
    pub fn new(other_field: impl Into<String>) -> Self {
        Self {
            other_field: other_field.into(),
        }
    }
}

#[async_trait]
impl Validatable for GreaterThanField {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is numeric
        if let Some(_) = value.as_f64() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value for comparison"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "greater_than_field",
            description: Some(&format!("Field value must be greater than field '{}'", self.other_field)),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "comparison"],
        }
    }
}

/// LessThanField validator - field value must be less than another field's value
pub struct LessThanField {
    other_field: String,
}

impl LessThanField {
    pub fn new(other_field: impl Into<String>) -> Self {
        Self {
            other_field: other_field.into(),
        }
    }
}

#[async_trait]
impl Validatable for LessThanField {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is numeric
        if let Some(_) = value.as_f64() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value for comparison"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "less_than_field",
            description: Some(&format!("Field value must be less than field '{}'", self.other_field)),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "comparison"],
        }
    }
}

/// RequiredIf validator - field is required if another field has a specific value
pub struct RequiredIf {
    other_field: String,
    expected_value: Value,
}

impl RequiredIf {
    pub fn new(other_field: impl Into<String>, expected_value: Value) -> Self {
        Self {
            other_field: other_field.into(),
            expected_value,
        }
    }
}

#[async_trait]
impl Validatable for RequiredIf {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is not null if it's provided
        if !value.is_null() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::RequiredFieldMissing,
                format!("Field is required when field '{}' equals {:?}", 
                    self.other_field, self.expected_value)
            ).with_actual_value(value.clone())
             .with_expected_value(self.expected_value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "required_if",
            description: Some(&format!("Field is required when field '{}' equals {:?}", 
                self.other_field, self.expected_value)),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "conditional"],
        }
    }
}

/// ForbiddenIf validator - field is forbidden if another field has a specific value
pub struct ForbiddenIf {
    other_field: String,
    expected_value: Value,
}

impl ForbiddenIf {
    pub fn new(other_field: impl Into<String>, expected_value: Value) -> Self {
        Self {
            other_field: other_field.into(),
            expected_value,
        }
    }
}

#[async_trait]
impl Validatable for ForbiddenIf {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is null if it's forbidden
        if value.is_null() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                format!("Field is forbidden when field '{}' equals {:?}", 
                    self.other_field, self.expected_value)
            ).with_actual_value(value.clone())
             .with_expected_value(self.expected_value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "forbidden_if",
            description: Some(&format!("Field is forbidden when field '{}' equals {:?}", 
                self.other_field, self.expected_value)),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "conditional"],
        }
    }
}

/// ConditionalRequired validator - field is required based on complex conditions
pub struct ConditionalRequired {
    conditions: Vec<(String, Value)>,
    operator: ConditionOperator,
}

#[derive(Debug, Clone)]
pub enum ConditionOperator {
    Any,    // Any condition must be met
    All,    // All conditions must be met
    None,   // No conditions must be met
}

impl ConditionalRequired {
    pub fn new(operator: ConditionOperator) -> Self {
        Self {
            conditions: Vec::new(),
            operator,
        }
    }
    
    pub fn add_condition(mut self, field: impl Into<String>, value: Value) -> Self {
        self.conditions.push((field.into(), value));
        self
    }
    
    pub fn any() -> Self {
        Self::new(ConditionOperator::Any)
    }
    
    pub fn all() -> Self {
        Self::new(ConditionOperator::All)
    }
    
    pub fn none() -> Self {
        Self::new(ConditionOperator::None)
    }
}

#[async_trait]
impl Validatable for ConditionalRequired {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is not null if conditions are met
        if !value.is_null() {
            Ok(())
        } else {
            let condition_desc = match self.operator {
                ConditionOperator::Any => "any of the conditions",
                ConditionOperator::All => "all of the conditions",
                ConditionOperator::None => "none of the conditions",
            };
            
            Err(ValidationError::new(
                ErrorCode::RequiredFieldMissing,
                format!("Field is required when {} are met", condition_desc)
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "conditional_required",
            description: Some("Field is required based on complex conditions"),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "conditional", "complex"],
        }
    }
}

/// FieldDependency validator - field depends on the presence/absence of other fields
pub struct FieldDependency {
    dependencies: Vec<FieldDependencyRule>,
}

#[derive(Debug, Clone)]
pub enum FieldDependencyRule {
    Required(String),
    Forbidden(String),
    RequiredWithValue(String, Value),
    ForbiddenWithValue(String, Value),
}

impl FieldDependency {
    pub fn new() -> Self {
        Self { dependencies: Vec::new() }
    }
    
    pub fn required(mut self, field: impl Into<String>) -> Self {
        self.dependencies.push(FieldDependencyRule::Required(field.into()));
        self
    }
    
    pub fn forbidden(mut self, field: impl Into<String>) -> Self {
        self.dependencies.push(FieldDependencyRule::Forbidden(field.into()));
        self
    }
    
    pub fn required_with_value(mut self, field: impl Into<String>, value: Value) -> Self {
        self.dependencies.push(FieldDependencyRule::RequiredWithValue(field.into(), value));
        self
    }
    
    pub fn forbidden_with_value(mut self, field: impl Into<String>, value: Value) -> Self {
        self.dependencies.push(FieldDependencyRule::ForbiddenWithValue(field.into(), value));
        self
    }
}

impl Default for FieldDependency {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validatable for FieldDependency {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is not null
        if !value.is_null() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                "Field has dependency rules that must be satisfied"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "field_dependency",
            description: Some("Field depends on the presence/absence of other fields"),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "dependency"],
        }
    }
}

/// SumEquals validator - sum of multiple fields must equal a specific value
pub struct SumEquals {
    fields: Vec<String>,
    expected_sum: f64,
}

impl SumEquals {
    pub fn new(fields: Vec<String>, expected_sum: f64) -> Self {
        Self {
            fields,
            expected_sum,
        }
    }
}

#[async_trait]
impl Validatable for SumEquals {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is numeric
        if let Some(_) = value.as_f64() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value for sum validation"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "sum_equals",
            description: Some(&format!("Sum of fields {:?} must equal {}", self.fields, self.expected_sum)),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "sum"],
        }
    }
}

/// UniqueTogether validator - combination of fields must be unique
pub struct UniqueTogether {
    fields: Vec<String>,
}

impl UniqueTogether {
    pub fn new(fields: Vec<String>) -> Self {
        Self { fields }
    }
}

#[async_trait]
impl Validatable for UniqueTogether {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is not null
        if !value.is_null() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::ValidationFailed,
                format!("Combination of fields {:?} must be unique", self.fields)
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "unique_together",
            description: Some(&format!("Combination of fields {:?} must be unique", self.fields)),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "uniqueness"],
        }
    }
}

/// CrossFieldRange validator - field value must be within range of another field
pub struct CrossFieldRange {
    min_field: Option<String>,
    max_field: Option<String>,
    inclusive: bool,
}

impl CrossFieldRange {
    pub fn new() -> Self {
        Self {
            min_field: None,
            max_field: None,
            inclusive: true,
        }
    }
    
    pub fn min_field(mut self, field: impl Into<String>) -> Self {
        self.min_field = Some(field.into());
        self
    }
    
    pub fn max_field(mut self, field: impl Into<String>) -> Self {
        self.max_field = Some(field.into());
        self
    }
    
    pub fn exclusive(mut self) -> Self {
        self.inclusive = false;
        self
    }
    
    pub fn inclusive(mut self) -> Self {
        self.inclusive = true;
        self
    }
}

impl Default for CrossFieldRange {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Validatable for CrossFieldRange {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // This validator would typically be used with a ValidationContext
        // For now, we'll just validate that the value is numeric
        if let Some(_) = value.as_f64() {
            Ok(())
        } else {
            Err(ValidationError::new(
                ErrorCode::TypeMismatch,
                "Expected numeric value for range validation"
            ).with_actual_value(value.clone()))
        }
    }
    
    fn metadata(&self) -> crate::ValidatorMetadata {
        crate::ValidatorMetadata {
            name: "cross_field_range",
            description: Some("Field value must be within range of other fields"),
            category: crate::ValidatorCategory::CrossField,
            tags: vec!["cross_field", "range"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_conditional_required() {
        let required_any = ConditionalRequired::any()
            .add_condition("field1".to_string(), json!("value1"))
            .add_condition("field2".to_string(), json!("value2"));
        
        let required_all = ConditionalRequired::all()
            .add_condition("field1".to_string(), json!("value1"))
            .add_condition("field2".to_string(), json!("value2"));
        
        let required_none = ConditionalRequired::none()
            .add_condition("field1".to_string(), json!("value1"));
        
        // Test that they can be created
        assert_eq!(required_any.conditions.len(), 2);
        assert_eq!(required_all.conditions.len(), 2);
        assert_eq!(required_none.conditions.len(), 1);
    }

    #[tokio::test]
    async fn test_field_dependency() {
        let dependency = FieldDependency::new()
            .required("field1")
            .forbidden("field2")
            .required_with_value("field3", json!("value3"))
            .forbidden_with_value("field4", json!("value4"));
        
        assert_eq!(dependency.dependencies.len(), 4);
        
        // Test that it can validate
        assert!(dependency.validate(&json!("test")).await.is_ok());
        assert!(dependency.validate(&json!(null)).await.is_err());
    }

    #[tokio::test]
    async fn test_cross_field_range() {
        let range = CrossFieldRange::new()
            .min_field("min_field")
            .max_field("max_field")
            .inclusive();
        
        assert!(range.min_field.is_some());
        assert!(range.max_field.is_some());
        assert!(range.inclusive);
        
        // Test that it can validate
        assert!(range.validate(&json!(42)).await.is_ok());
        assert!(range.validate(&json!("not_numeric")).await.is_err());
    }
}
