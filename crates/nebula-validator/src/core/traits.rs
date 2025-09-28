//! Core validation traits for nebula-validator

use async_trait::async_trait;
use nebula_value::Value;
use std::collections::HashMap;

use super::{Valid, Invalid};

/// Main validation trait - the core interface for all validators
#[async_trait]
pub trait Validator: Send + Sync {
    /// Validate a value with optional context for cross-field validation
    async fn validate(&self, value: &Value, context: Option<&ValidationContext>) -> Result<Valid<()>, Invalid<()>>;

    /// Get the validator name/identifier
    fn name(&self) -> &str;

    /// Get optional description of what this validator does
    fn description(&self) -> Option<&str> {
        None
    }

    /// Get validation complexity level (for optimization)
    fn complexity(&self) -> ValidationComplexity {
        ValidationComplexity::Simple
    }

    /// Whether this validator can be cached
    fn is_cacheable(&self) -> bool {
        true
    }
}

/// Validation context for cross-field validation
#[derive(Debug, Clone)]
pub struct ValidationContext {
    /// The root object being validated (for cross-field access)
    pub root_object: Value,

    /// Current field path (e.g., "user.profile.email")
    pub current_path: String,

    /// Additional metadata that can be passed to validators
    pub metadata: HashMap<String, Value>,
}

/// Lightweight validation context with references to avoid cloning
#[derive(Debug)]
pub struct ValidationContextRef<'a> {
    /// Reference to the root object being validated
    pub root_object: &'a Value,

    /// Current field path
    pub current_path: &'a str,

    /// Additional metadata
    pub metadata: &'a HashMap<String, Value>,
}

impl ValidationContext {
    /// Create a new validation context
    pub fn new(root_object: Value, current_path: String) -> Self {
        Self {
            root_object,
            current_path,
            metadata: HashMap::new(),
        }
    }

    /// Create a simple context with just the root object
    pub fn simple(root_object: Value) -> Self {
        Self::new(root_object, String::new())
    }

    /// Get value at a specific path in the root object
    pub fn get_field(&self, path: &str) -> Option<&Value> {
        get_nested_value(&self.root_object, path)
    }

    /// Get value of a sibling field (same parent)
    pub fn get_sibling(&self, field_name: &str) -> Option<&Value> {
        if let Some(parent_path) = self.current_path.rfind('.') {
            let parent = &self.current_path[..parent_path];
            let sibling_path = format!("{}.{}", parent, field_name);
            self.get_field(&sibling_path)
        } else {
            // We're at root level, so sibling is just the field name
            self.get_field(field_name)
        }
    }

    /// Add metadata to the context
    pub fn with_metadata(mut self, key: String, value: Value) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// Get metadata value
    pub fn get_metadata(&self, key: &str) -> Option<&Value> {
        self.metadata.get(key)
    }

    /// Create a child context for nested validation (expensive due to cloning)
    pub fn child_context(&self, field_name: &str) -> ValidationContext {
        let new_path = if self.current_path.is_empty() {
            field_name.to_string()
        } else {
            format!("{}.{}", self.current_path, field_name)
        };

        ValidationContext {
            root_object: self.root_object.clone(),
            current_path: new_path,
            metadata: self.metadata.clone(),
        }
    }

    /// Create a lightweight reference-based child context (more efficient)
    pub fn child_context_ref<'a>(&'a self, _field_name: &'a str, full_path: &'a str) -> ValidationContextRef<'a> {
        ValidationContextRef {
            root_object: &self.root_object,
            current_path: full_path,
            metadata: &self.metadata,
        }
    }

    /// Convert to reference-based context
    pub fn as_ref(&self) -> ValidationContextRef<'_> {
        ValidationContextRef {
            root_object: &self.root_object,
            current_path: &self.current_path,
            metadata: &self.metadata,
        }
    }
}

impl<'a> ValidationContextRef<'a> {
    /// Get value at a specific path in the root object
    pub fn get_field(&self, path: &str) -> Option<&Value> {
        get_nested_value(self.root_object, path)
    }

    /// Get value of a sibling field (same parent)
    pub fn get_sibling(&self, field_name: &str) -> Option<&Value> {
        if let Some(parent_path) = self.current_path.rfind('.') {
            let parent = &self.current_path[..parent_path];
            let sibling_path = format!("{}.{}", parent, field_name);
            self.get_field(&sibling_path)
        } else {
            self.get_field(field_name)
        }
    }

    /// Get metadata value
    pub fn get_metadata(&self, key: &str) -> Option<&Value> {
        self.metadata.get(key)
    }
}

/// Validation complexity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ValidationComplexity {
    /// O(1) - Simple check
    Simple,
    /// O(n) - Linear with input size
    Moderate,
    /// O(n²) - Quadratic complexity
    Complex,
    /// O(n³) or higher - Very complex
    VeryComplex,
}

impl ValidationComplexity {
    /// Combine two complexity levels (takes the higher one)
    pub fn combine(self, other: Self) -> Self {
        std::cmp::max(self, other)
    }
}

/// Extension trait for Validator to provide convenient combinators
pub trait ValidatorExt: Validator + Sized {
    /// Combine this validator with another using AND logic
    fn and<V: Validator>(self, other: V) -> AndValidator<Self, V> {
        AndValidator::new(self, other)
    }

    /// Combine this validator with another using OR logic
    fn or<V: Validator>(self, other: V) -> OrValidator<Self, V> {
        OrValidator::new(self, other)
    }

    /// Negate this validator (NOT logic)
    fn not(self) -> NotValidator<Self> {
        NotValidator::new(self)
    }

    /// Make this validator conditional based on a condition
    fn when<C: Validator>(self, condition: C) -> ConditionalValidator<Self, C> {
        ConditionalValidator::new(self, condition)
    }
}

// Implement ValidatorExt for all types that implement Validator
impl<T: Validator> ValidatorExt for T {}

// ============= Logical Combinators =============

/// AND combinator - both validators must pass (uses static dispatch for better performance)
pub struct AndValidator<L, R>
where
    L: Validator,
    R: Validator,
{
    left: L,
    right: R,
    name: String,
}

impl<L: Validator, R: Validator> AndValidator<L, R> {
    pub fn new(left: L, right: R) -> Self {
        let name = format!("({} AND {})", left.name(), right.name());
        Self { left, right, name }
    }
}

#[async_trait]
impl<L: Validator, R: Validator> Validator for AndValidator<L, R> {
    async fn validate(&self, value: &Value, context: Option<&ValidationContext>) -> Result<Valid<()>, Invalid<()>> {
        // Both must pass - collect all errors
        let left_result = self.left.validate(value, context).await;
        let right_result = self.right.validate(value, context).await;

        match (left_result, right_result) {
            (Ok(_), Ok(_)) => Ok(Valid::simple(())),
            (Err(left_invalid), Err(right_invalid)) => {
                // Combine errors from both validators
                Err(left_invalid.combine(right_invalid)
                    .with_validator_name(&self.name))
            },
            (Err(invalid), _) => Err(invalid.with_validator_name(&self.name)),
            (_, Err(invalid)) => Err(invalid.with_validator_name(&self.name)),
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn complexity(&self) -> ValidationComplexity {
        self.left.complexity().combine(self.right.complexity())
    }
}

/// OR combinator - at least one validator must pass (uses static dispatch for better performance)
pub struct OrValidator<L, R>
where
    L: Validator,
    R: Validator,
{
    left: L,
    right: R,
    name: String,
}

impl<L: Validator, R: Validator> OrValidator<L, R> {
    pub fn new(left: L, right: R) -> Self {
        let name = format!("({} OR {})", left.name(), right.name());
        Self { left, right, name }
    }
}

#[async_trait]
impl<L: Validator, R: Validator> Validator for OrValidator<L, R> {
    async fn validate(&self, value: &Value, context: Option<&ValidationContext>) -> Result<Valid<()>, Invalid<()>> {
        // Try both - if either passes, succeed; if both fail, combine errors
        let left_result = self.left.validate(value, context).await;
        let right_result = self.right.validate(value, context).await;

        match (left_result, right_result) {
            (Ok(valid), _) | (_, Ok(valid)) => Ok(valid),
            (Err(left_invalid), Err(right_invalid)) => {
                // Both failed - combine errors with context
                Err(left_invalid.combine(right_invalid)
                    .with_context("All OR conditions failed")
                    .with_validator_name(&self.name))
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn complexity(&self) -> ValidationComplexity {
        // OR can short-circuit, so take the simpler complexity
        std::cmp::min(self.left.complexity(), self.right.complexity())
    }
}

/// NOT combinator - validator must fail for this to pass (uses static dispatch for better performance)
pub struct NotValidator<V>
where
    V: Validator,
{
    validator: V,
    name: String,
}

impl<V: Validator> NotValidator<V> {
    pub fn new(validator: V) -> Self {
        let name = format!("NOT {}", validator.name());
        Self { validator, name }
    }
}

#[async_trait]
impl<V: Validator> Validator for NotValidator<V> {
    async fn validate(&self, value: &Value, context: Option<&ValidationContext>) -> Result<Valid<()>, Invalid<()>> {
        match self.validator.validate(value, context).await {
            Ok(_) => {
                // Validator passed, so NOT fails
                Err(Invalid::simple(format!("NOT condition failed: {} should not be valid", self.validator.name())))
            },
            Err(_) => {
                // Validator failed, so NOT passes
                Ok(Valid::simple(()))
            },
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn complexity(&self) -> ValidationComplexity {
        self.validator.complexity()
    }
}

/// Conditional validator - only run if condition passes (uses static dispatch for better performance)
pub struct ConditionalValidator<V, C>
where
    V: Validator,
    C: Validator,
{
    validator: V,
    condition: C,
    name: String,
}

impl<V: Validator, C: Validator> ConditionalValidator<V, C> {
    pub fn new(validator: V, condition: C) -> Self {
        let name = format!("{} WHEN {}", validator.name(), condition.name());
        Self { validator, condition, name }
    }
}

#[async_trait]
impl<V: Validator, C: Validator> Validator for ConditionalValidator<V, C> {
    async fn validate(&self, value: &Value, context: Option<&ValidationContext>) -> Result<Valid<()>, Invalid<()>> {
        // Check condition first
        if self.condition.validate(value, context).await.is_ok() {
            // Condition passed, run the actual validator
            self.validator.validate(value, context).await
        } else {
            // Condition failed, so this validator is skipped (considered passing)
            Ok(Valid::simple(()))
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn complexity(&self) -> ValidationComplexity {
        self.validator.complexity().combine(self.condition.complexity())
    }
}

// ============= Helper Functions =============

/// Get nested value from JSON object using dot notation path
fn get_nested_value<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(value);
    }

    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;

    for part in parts {
        match current {
            Value::Object(obj) => {
                current = obj.get(part)?;
            },
            Value::Array(arr) => {
                let index: usize = part.parse().ok()?;
                current = arr.get(index)?;
            },
            _ => return None,
        }
    }

    Some(current)
}