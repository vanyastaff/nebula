//! Enhanced conditional validators for nebula-validator
//! 
//! This module provides advanced conditional validation capabilities including
//! When chains, field conditions, and complex rule composition.

use async_trait::async_trait;
use serde_json::Value;
use std::fmt::Debug;
use std::cmp::Ordering;

use crate::traits::Validatable;
use crate::types::{ValidationResult, ValidationError, ValidatorMetadata, ValidationComplexity};
use crate::context::ValidationContext;

// ==================== Condition Trait ====================

/// Condition for When validator
pub trait Condition: Send + Sync + Debug {
    /// Evaluate condition
    fn evaluate<'a>(&'a self, value: &'a Value, context: &'a ValidationContext) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>>;
    
    /// Get condition description
    fn describe(&self) -> String;
}

// ==================== Enhanced When Validator ====================

/// Enhanced When validator with support for complex conditions
pub struct When<C, T, F> {
    condition: C,
    then_validator: T,
    else_validator: Option<F>,
    mode: WhenMode,
}

#[derive(Debug, Clone, Copy)]
pub enum WhenMode {
    /// Apply validator only if condition is true
    Simple,
    /// Apply then if true, else if false
    IfElse,
    /// Apply validators based on multiple conditions
    Switch,
}

impl<C, T, F> When<C, T, F> 
where
    C: Condition + 'static,
    T: Validatable + 'static,
    F: Validatable + 'static,
{
    /// Create simple When validator
    pub fn new(condition: C, then_validator: T) -> Self {
        Self {
            condition,
            then_validator,
            else_validator: None,
            mode: WhenMode::Simple,
        }
    }
    
    /// Add else branch
    pub fn otherwise(mut self, else_validator: F) -> Self {
        self.else_validator = Some(else_validator);
        self.mode = WhenMode::IfElse;
        self
    }
    
    /// Create chain of When validators
    pub fn and_when<C2, T2>(self, condition: C2, validator: T2) -> WhenChain 
    where
        C2: Condition + 'static,
        T2: Validatable + 'static,
    {
        WhenChain::new()
            .when(self.condition, self.then_validator)
            .when(condition, validator)
    }
}

#[async_trait]
impl<C, T, F> Validatable for When<C, T, F>
where
    C: Condition,
    T: Validatable,
    F: Validatable,
{
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let context = ValidationContext::new();
        let condition_result = self.condition.evaluate(value, &context).await;
        
        if condition_result {
            self.then_validator.validate(value).await
        } else if let Some(ref else_validator) = self.else_validator {
            else_validator.validate(value).await
        } else {
            ValidationResult::success(())
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            "when",
            "Conditional validation",
            crate::types::ValidatorCategory::Conditional,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Complex
    }
}

// ==================== When Chain ====================

/// Chain of When conditions (analog to switch/case)
pub struct WhenChain {
    cases: Vec<WhenCase>,
    default: Option<Box<dyn Validatable>>,
}

struct WhenCase {
    condition: Box<dyn Condition>,
    validator: Box<dyn Validatable>,
    stop_on_match: bool,
}

impl WhenChain {
    /// Create new WhenChain
    pub fn new() -> Self {
        Self {
            cases: Vec::new(),
            default: None,
        }
    }
    
    /// Add condition
    pub fn when<C, V>(mut self, condition: C, validator: V) -> Self 
    where
        C: Condition + 'static,
        V: Validatable + 'static,
    {
        self.cases.push(WhenCase {
            condition: Box::new(condition),
            validator: Box::new(validator),
            stop_on_match: true,
        });
        self
    }
    
    /// Add condition that doesn't stop the chain
    pub fn also_when<C, V>(mut self, condition: C, validator: V) -> Self
    where
        C: Condition + 'static,
        V: Validatable + 'static,
    {
        self.cases.push(WhenCase {
            condition: Box::new(condition),
            validator: Box::new(validator),
            stop_on_match: false,
        });
        self
    }
    
    /// Set default validator
    pub fn otherwise<V>(mut self, validator: V) -> Self
    where
        V: Validatable + 'static,
    {
        self.default = Some(Box::new(validator));
        self
    }
}

#[async_trait]
impl Validatable for WhenChain {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let context = ValidationContext::new();
        let mut matched = false;
        let mut errors = Vec::new();
        
        for case in &self.cases {
            if case.condition.evaluate(value, &context).await {
                matched = true;
                let result = case.validator.validate(value).await;
                if result.is_err() {
                    if let Some(error) = result.err() {
                        errors.extend(error);
                    }
                }
                
                if case.stop_on_match {
                    break;
                }
            }
        }
        
        // If no condition matched, apply default
        if !matched {
            if let Some(ref default_validator) = self.default {
                let result = default_validator.validate(value).await;
                if result.is_err() {
                    if let Some(error) = result.err() {
                        errors.extend(error);
                    }
                }
            }
        }
        
        if errors.is_empty() {
            ValidationResult::success(())
        } else {
            ValidationResult::failure(errors)
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            "when_chain",
            "Conditional validation chain",
            crate::types::ValidatorCategory::Conditional,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Complex
    }
}

// ==================== Field Conditions ====================

/// Field condition
#[derive(Debug, Clone)]
pub struct FieldCondition {
    field_path: String,
    predicate: FieldPredicate,
}

#[derive(Debug, Clone)]
pub enum FieldPredicate {
    /// Field exists
    Exists,
    /// Field doesn't exist
    NotExists,
    /// Field equals value
    Equals(Value),
    /// Field doesn't equal value
    NotEquals(Value),
    /// Field is in list
    In(Vec<Value>),
    /// Field is not in list
    NotIn(Vec<Value>),
    /// Field matches pattern
    Matches(String),
    /// Field greater than value
    GreaterThan(Value),
    /// Field less than value
    LessThan(Value),
    /// Field between values
    Between(Value, Value),
}

impl FieldCondition {
    /// Create new field condition
    pub fn new(field_path: impl Into<String>, predicate: FieldPredicate) -> Self {
        Self {
            field_path: field_path.into(),
            predicate,
        }
    }
    
    /// Get field value from JSON
    fn get_field_value<'a>(&self, value: &'a Value) -> Option<&'a Value> {
        if self.field_path == "." {
            Some(value)
        } else {
            value.get(&self.field_path)
        }
    }
}

impl Condition for FieldCondition {
    fn evaluate<'a>(&'a self, value: &'a Value, _context: &'a ValidationContext) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>> {
        let field_value = self.get_field_value(value);
        let predicate = self.predicate.clone();
        
        Box::pin(async move {
            match &predicate {
                FieldPredicate::Exists => field_value.is_some(),
                FieldPredicate::NotExists => field_value.is_none(),
                FieldPredicate::Equals(expected) => {
                    field_value.map_or(false, |v| v == expected)
                },
                FieldPredicate::NotEquals(expected) => {
                    field_value.map_or(true, |v| v != expected)
                },
                FieldPredicate::In(values) => {
                    field_value.map_or(false, |v| values.contains(v))
                },
                FieldPredicate::NotIn(values) => {
                    field_value.map_or(true, |v| !values.contains(v))
                },
                FieldPredicate::Matches(pattern) => {
                    if let Some(Value::String(s)) = field_value {
                        regex::Regex::new(pattern).ok()
                            .map_or(false, |re| re.is_match(s))
                    } else {
                        false
                    }
                },
                FieldPredicate::GreaterThan(threshold) => {
                    field_value.map_or(false, |v| {
                        compare_values(v, threshold, Ordering::Greater)
                    })
                },
                FieldPredicate::LessThan(threshold) => {
                    field_value.map_or(false, |v| {
                        compare_values(v, threshold, Ordering::Less)
                    })
                },
                FieldPredicate::Between(min, max) => {
                    field_value.map_or(false, |v| {
                        compare_values(v, min, Ordering::Greater) &&
                        compare_values(v, max, Ordering::Less)
                    })
                },
            }
        })
    }
    
    fn describe(&self) -> String {
        format!("Field '{}' {:?}", self.field_path, self.predicate)
    }
}

// ==================== Combined Conditions ====================

/// Combined conditions
#[derive(Debug)]
pub enum CombinedCondition {
    /// All conditions must be true
    All(Vec<Box<dyn Condition>>),
    /// At least one condition must be true
    Any(Vec<Box<dyn Condition>>),
    /// No condition should be true
    None(Vec<Box<dyn Condition>>),
    /// Condition inversion
    Not(Box<dyn Condition>),
}

impl Condition for CombinedCondition {
    fn evaluate<'a>(&'a self, value: &'a Value, context: &'a ValidationContext) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>> {
        let condition_type = match self {
            Self::All(_) => "all",
            Self::Any(_) => "any", 
            Self::None(_) => "none",
            Self::Not(_) => "not",
        };
        
        Box::pin(async move {
            match condition_type {
                "all" => {
                    match self {
                        Self::All(conditions) => {
                            for condition in conditions {
                                if !condition.evaluate(value, context).await {
                                    return false;
                                }
                            }
                            true
                        },
                        _ => false,
                    }
                },
                "any" => {
                    match self {
                        Self::Any(conditions) => {
                            for condition in conditions {
                                if condition.evaluate(value, context).await {
                                    return true;
                                }
                            }
                            false
                        },
                        _ => false,
                    }
                },
                "none" => {
                    match self {
                        Self::None(conditions) => {
                            for condition in conditions {
                                if condition.evaluate(value, context).await {
                                    return false;
                                }
                            }
                            true
                        },
                        _ => false,
                    }
                },
                "not" => {
                    match self {
                        Self::Not(condition) => {
                            !condition.evaluate(value, context).await
                        },
                        _ => false,
                    }
                },
                _ => false,
            }
        })
    }
    
    fn describe(&self) -> String {
        match self {
            Self::All(_) => "All conditions".to_string(),
            Self::Any(_) => "Any condition".to_string(),
            Self::None(_) => "No conditions".to_string(),
            Self::Not(c) => format!("Not {}", c.describe()),
        }
    }
}

// ==================== Enhanced Required Validator ====================

/// Enhanced Required validator with conditions
pub struct Required {
    mode: RequiredMode,
    conditions: Vec<Box<dyn Condition>>,
    custom_message: Option<String>,
}

#[derive(Debug)]
pub enum RequiredMode {
    /// Always required
    Always,
    /// Required if condition is true
    If(Box<dyn Condition>),
    /// Required if any condition is true
    IfAny(Vec<Box<dyn Condition>>),
    /// Required if all conditions are true
    IfAll(Vec<Box<dyn Condition>>),
    /// Required if another field exists
    IfFieldExists(String),
    /// Required if another field equals value
    IfFieldEquals(String, Value),
    /// Required based on group
    Group(String),
}

impl Required {
    /// Create simple Required validator
    pub fn new() -> Self {
        Self {
            mode: RequiredMode::Always,
            conditions: Vec::new(),
            custom_message: None,
        }
    }
    
    /// Required if condition
    pub fn if_condition<C: Condition + 'static>(mut self, condition: C) -> Self {
        self.mode = RequiredMode::If(Box::new(condition));
        self
    }
    
    /// Required if field exists
    pub fn if_field_exists(mut self, field: impl Into<String>) -> Self {
        self.mode = RequiredMode::IfFieldExists(field.into());
        self
    }
    
    /// Required if field equals value
    pub fn if_field_equals(mut self, field: impl Into<String>, value: Value) -> Self {
        self.mode = RequiredMode::IfFieldEquals(field.into(), value);
        self
    }
    
    /// Set custom error message
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.custom_message = Some(message.into());
        self
    }
    
    /// Check if field is required for given value
    async fn is_required(&self, value: &Value) -> bool {
        let context = ValidationContext::new();
        
        match &self.mode {
            RequiredMode::Always => true,
            RequiredMode::If(condition) => condition.evaluate(value, &context).await,
            RequiredMode::IfAny(conditions) => {
                for condition in conditions {
                    if condition.evaluate(value, &context).await {
                        return true;
                    }
                }
                false
            },
            RequiredMode::IfAll(conditions) => {
                for condition in conditions {
                    if !condition.evaluate(value, &context).await {
                        return false;
                    }
                }
                true
            },
            RequiredMode::IfFieldExists(field) => value.get(field).is_some(),
            RequiredMode::IfFieldEquals(field, expected) => {
                value.get(field).map_or(false, |v| v == expected)
            },
            RequiredMode::Group(_) => true, // TODO: Implement group logic
        }
    }
}

#[async_trait]
impl Validatable for Required {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        if self.is_required(value).await {
            if value.is_null() {
                let message = self.custom_message.clone()
                    .unwrap_or_else(|| "Field is required".to_string());
                ValidationResult::failure(vec![
                    ValidationError::new(
                        crate::types::ErrorCode::new("required"),
                        message,
                    )
                ])
            } else {
                ValidationResult::success(())
            }
        } else {
            ValidationResult::success(())
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            "required",
            "Required field validator",
            crate::types::ValidatorCategory::Conditional,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
}

impl Default for Required {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Enhanced Optional Validator ====================

/// Enhanced Optional validator
pub struct Optional<V> {
    validator: V,
    apply_when: OptionalMode,
    default_value: Option<Value>,
    transform: Option<Box<dyn Fn(Value) -> Value + Send + Sync>>,
}

#[derive(Debug)]
pub enum OptionalMode {
    /// Always optional
    Always,
    /// Validate only if field exists
    IfPresent,
    /// Validate only if not null
    IfNotNull,
    /// Validate only if not empty
    IfNotEmpty,
    /// Validate with condition
    IfCondition(Box<dyn Condition>),
}

impl<V: Validatable> Optional<V> {
    /// Create Optional validator
    pub fn new(validator: V) -> Self {
        Self {
            validator,
            apply_when: OptionalMode::IfPresent,
            default_value: None,
            transform: None,
        }
    }
    
    /// Set default value
    pub fn with_default(mut self, value: Value) -> Self {
        self.default_value = Some(value);
        self
    }
    
    /// Transform value before validation
    pub fn transform<F>(mut self, f: F) -> Self 
    where
        F: Fn(Value) -> Value + Send + Sync + 'static,
    {
        self.transform = Some(Box::new(f));
        self
    }
    
    /// Validate only if condition is true
    pub fn when<C: Condition + 'static>(mut self, condition: C) -> Self {
        self.apply_when = OptionalMode::IfCondition(Box::new(condition));
        self
    }
}

#[async_trait]
impl<V: Validatable> Validatable for Optional<V> {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let should_validate = match &self.apply_when {
            OptionalMode::Always => true,
            OptionalMode::IfPresent => !value.is_null(),
            OptionalMode::IfNotNull => !value.is_null(),
            OptionalMode::IfNotEmpty => {
                match value {
                    Value::String(s) => !s.is_empty(),
                    Value::Array(a) => !a.is_empty(),
                    Value::Object(o) => !o.is_empty(),
                    Value::Null => false,
                    _ => true,
                }
            },
            OptionalMode::IfCondition(condition) => {
                let context = ValidationContext::new();
                condition.evaluate(value, &context).await
            },
        };
        
        if should_validate {
            let value_to_validate = if let Some(ref transform) = self.transform {
                transform(value.clone())
            } else {
                value.clone()
            };
            
            self.validator.validate(&value_to_validate).await
        } else if value.is_null() && self.default_value.is_some() {
            // Apply default value
            ValidationResult::success(())
        } else {
            ValidationResult::success(())
        }
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::new(
            "optional",
            "Optional validator",
            crate::types::ValidatorCategory::Conditional,
        )
    }
    
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }
}

// ==================== Helper Functions ====================

/// Compare values for ordering
fn compare_values(a: &Value, b: &Value, ordering: Ordering) -> bool {
    match (a, b) {
        (Value::Number(na), Value::Number(nb)) => {
            if let (Some(fa), Some(fb)) = (na.as_f64(), nb.as_f64()) {
                match ordering {
                    Ordering::Greater => fa > fb,
                    Ordering::Less => fa < fb,
                    Ordering::Equal => fa == fb,
                }
            } else {
                false
            }
        },
        (Value::String(sa), Value::String(sb)) => {
            match ordering {
                Ordering::Greater => sa > sb,
                Ordering::Less => sa < sb,
                Ordering::Equal => sa == sb,
            }
        },
        _ => false,
    }
}

// ==================== Builder Functions ====================

/// Create field condition for field existence
pub fn field(field_path: impl Into<String>) -> FieldCondition {
    FieldCondition::new(field_path, FieldPredicate::Exists)
}

/// Create field condition for field equality
pub fn field_equals(field_path: impl Into<String>, value: Value) -> FieldCondition {
    FieldCondition::new(field_path, FieldPredicate::Equals(value))
}

/// Create field condition for field pattern matching
pub fn field_matches(field_path: impl Into<String>, pattern: impl Into<String>) -> FieldCondition {
    FieldCondition::new(field_path, FieldPredicate::Matches(pattern.into()))
}

/// Create combined condition for all
pub fn all(conditions: Vec<Box<dyn Condition>>) -> CombinedCondition {
    CombinedCondition::All(conditions)
}

/// Create combined condition for any
pub fn any(conditions: Vec<Box<dyn Condition>>) -> CombinedCondition {
    CombinedCondition::Any(conditions)
}

/// Create combined condition for none
pub fn none(conditions: Vec<Box<dyn Condition>>) -> CombinedCondition {
    CombinedCondition::None(conditions)
}

/// Create combined condition for not
pub fn not(condition: Box<dyn Condition>) -> CombinedCondition {
    CombinedCondition::Not(condition)
}
