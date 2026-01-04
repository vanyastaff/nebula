# Validation API Redesign - Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace builder-based validation in nebula-parameter with a thin wrapper over nebula-validator that supports cross-field validation.

**Architecture:** 
- Add Value type wrappers to nebula-validator (`ValueString`, `ValueInteger`, etc.)
- Add cross-field validators and conditions to nebula-validator
- Rewrite `ParameterValidation` in nebula-parameter as a thin wrapper
- Add `FieldCondition`, `CrossFieldRule`, and `FormRule` enums

**Tech Stack:** Rust 2024, nebula-validator, nebula-value, nebula-parameter, async-trait, serde

---

## Phase 1: Value Type Wrappers in nebula-validator

### Task 1.1: Create validators/value.rs module

**Files:**
- Create: `crates/nebula-validator/src/validators/value.rs`
- Modify: `crates/nebula-validator/src/validators/mod.rs`
- Modify: `crates/nebula-validator/Cargo.toml`

**Step 1: Add nebula-value dependency to nebula-validator**

```toml
# In crates/nebula-validator/Cargo.toml, add under [dependencies]:
nebula-value = { path = "../nebula-value" }
```

**Step 2: Create value.rs with ValueString wrapper**

```rust
//! Value type wrappers for nebula-value::Value
//!
//! These wrappers extract typed values from `Value` and delegate to typed validators.

use crate::core::{TypedValidator, ValidationError};
use nebula_value::Value;
use std::marker::PhantomData;

/// Wrapper that extracts `&str` from `Value::Text` and validates it
pub struct ValueString<V> {
    validator: V,
}

impl<V> ValueString<V> {
    /// Create a new ValueString wrapper
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    /// Get reference to inner validator
    pub fn inner(&self) -> &V {
        &self.validator
    }
}

impl<V> TypedValidator for ValueString<V>
where
    V: TypedValidator<Input = str, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let s = input
            .as_str()
            .ok_or_else(|| ValidationError::new("type_error", "Expected text value"))?;
        self.validator.validate(s)
    }
}

/// Convenience function to create a ValueString validator
pub fn value_string<V>(validator: V) -> ValueString<V>
where
    V: TypedValidator<Input = str, Output = (), Error = ValidationError>,
{
    ValueString::new(validator)
}
```

**Step 3: Run test to verify it compiles**

Run: `cargo check -p nebula-validator`
Expected: Compiles successfully

**Step 4: Add basic test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::validators::string::min_length;

    #[test]
    fn test_value_string_valid() {
        let validator = value_string(min_length(3));
        assert!(validator.validate(&Value::text("hello")).is_ok());
    }

    #[test]
    fn test_value_string_invalid() {
        let validator = value_string(min_length(3));
        assert!(validator.validate(&Value::text("hi")).is_err());
    }

    #[test]
    fn test_value_string_wrong_type() {
        let validator = value_string(min_length(3));
        assert!(validator.validate(&Value::integer(42)).is_err());
    }
}
```

**Step 5: Run tests**

Run: `cargo test -p nebula-validator value_string`
Expected: All 3 tests pass

**Step 6: Export from mod.rs**

```rust
// In crates/nebula-validator/src/validators/mod.rs, add:
pub mod value;
pub use value::*;
```

**Step 7: Commit**

```bash
git add crates/nebula-validator/Cargo.toml crates/nebula-validator/src/validators/value.rs crates/nebula-validator/src/validators/mod.rs
git commit -m "feat(nebula-validator): add ValueString wrapper for Value type"
```

---

### Task 1.2: Add ValueInteger wrapper

**Files:**
- Modify: `crates/nebula-validator/src/validators/value.rs`

**Step 1: Add ValueInteger wrapper**

```rust
/// Wrapper that extracts `i64` from `Value::Integer` and validates it
pub struct ValueInteger<V> {
    validator: V,
}

impl<V> ValueInteger<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    pub fn inner(&self) -> &V {
        &self.validator
    }
}

impl<V> TypedValidator for ValueInteger<V>
where
    V: TypedValidator<Input = i64, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let n = input
            .as_integer()
            .ok_or_else(|| ValidationError::new("type_error", "Expected integer value"))?;
        self.validator.validate(&n.value())
    }
}

pub fn value_integer<V>(validator: V) -> ValueInteger<V>
where
    V: TypedValidator<Input = i64, Output = (), Error = ValidationError>,
{
    ValueInteger::new(validator)
}
```

**Step 2: Add tests**

```rust
#[test]
fn test_value_integer_valid() {
    use crate::validators::numeric::min;
    let validator = value_integer(min(18i64));
    assert!(validator.validate(&Value::integer(25)).is_ok());
}

#[test]
fn test_value_integer_invalid() {
    use crate::validators::numeric::min;
    let validator = value_integer(min(18i64));
    assert!(validator.validate(&Value::integer(15)).is_err());
}
```

**Step 3: Run tests**

Run: `cargo test -p nebula-validator value_integer`
Expected: All tests pass

**Step 4: Commit**

```bash
git add crates/nebula-validator/src/validators/value.rs
git commit -m "feat(nebula-validator): add ValueInteger wrapper"
```

---

### Task 1.3: Add ValueFloat wrapper

**Files:**
- Modify: `crates/nebula-validator/src/validators/value.rs`

**Step 1: Add ValueFloat wrapper**

```rust
/// Wrapper that extracts `f64` from `Value::Float` and validates it
pub struct ValueFloat<V> {
    validator: V,
}

impl<V> ValueFloat<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    pub fn inner(&self) -> &V {
        &self.validator
    }
}

impl<V> TypedValidator for ValueFloat<V>
where
    V: TypedValidator<Input = f64, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let n = input
            .as_float_lossy()
            .ok_or_else(|| ValidationError::new("type_error", "Expected numeric value"))?;
        self.validator.validate(&n.value())
    }
}

pub fn value_float<V>(validator: V) -> ValueFloat<V>
where
    V: TypedValidator<Input = f64, Output = (), Error = ValidationError>,
{
    ValueFloat::new(validator)
}
```

**Step 2: Add tests and run**

Run: `cargo test -p nebula-validator value_float`
Expected: All tests pass

**Step 3: Commit**

```bash
git add crates/nebula-validator/src/validators/value.rs
git commit -m "feat(nebula-validator): add ValueFloat wrapper"
```

---

### Task 1.4: Add ValueBoolean, ValueArray, ValueObject wrappers

**Files:**
- Modify: `crates/nebula-validator/src/validators/value.rs`

**Step 1: Add remaining wrappers**

```rust
/// Wrapper for boolean validation
pub struct ValueBoolean<V> {
    validator: V,
}

impl<V> ValueBoolean<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

impl<V> TypedValidator for ValueBoolean<V>
where
    V: TypedValidator<Input = bool, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let b = input
            .as_boolean()
            .ok_or_else(|| ValidationError::new("type_error", "Expected boolean value"))?;
        self.validator.validate(&b)
    }
}

pub fn value_boolean<V>(validator: V) -> ValueBoolean<V>
where
    V: TypedValidator<Input = bool, Output = (), Error = ValidationError>,
{
    ValueBoolean::new(validator)
}

/// Wrapper for array validation
pub struct ValueArray<V> {
    validator: V,
}

impl<V> ValueArray<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

impl<V> TypedValidator for ValueArray<V>
where
    V: TypedValidator<Input = nebula_value::Array, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let arr = input
            .as_array()
            .ok_or_else(|| ValidationError::new("type_error", "Expected array value"))?;
        self.validator.validate(arr)
    }
}

pub fn value_array<V>(validator: V) -> ValueArray<V>
where
    V: TypedValidator<Input = nebula_value::Array, Output = (), Error = ValidationError>,
{
    ValueArray::new(validator)
}

/// Wrapper for object validation
pub struct ValueObject<V> {
    validator: V,
}

impl<V> ValueObject<V> {
    pub fn new(validator: V) -> Self {
        Self { validator }
    }
}

impl<V> TypedValidator for ValueObject<V>
where
    V: TypedValidator<Input = nebula_value::Object, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let obj = input
            .as_object()
            .ok_or_else(|| ValidationError::new("type_error", "Expected object value"))?;
        self.validator.validate(obj)
    }
}

pub fn value_object<V>(validator: V) -> ValueObject<V>
where
    V: TypedValidator<Input = nebula_value::Object, Output = (), Error = ValidationError>,
{
    ValueObject::new(validator)
}
```

**Step 2: Run all value tests**

Run: `cargo test -p nebula-validator validators::value`
Expected: All tests pass

**Step 3: Commit**

```bash
git add crates/nebula-validator/src/validators/value.rs
git commit -m "feat(nebula-validator): add ValueBoolean, ValueArray, ValueObject wrappers"
```

---

## Phase 2: FieldCondition enum in nebula-parameter

### Task 2.1: Create field_condition.rs

**Files:**
- Create: `crates/nebula-parameter/src/core/field_condition.rs`
- Modify: `crates/nebula-parameter/src/core/mod.rs`

**Step 1: Create field_condition.rs**

```rust
//! Field conditions for cross-field validation
//!
//! Similar to `DisplayCondition` but for validation purposes.

use nebula_value::Value;
use serde::{Deserialize, Serialize};

/// Condition to evaluate against a field value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldCondition {
    // === Value comparisons ===
    
    /// Value equals the specified value
    Equals(Value),
    
    /// Value does not equal the specified value
    NotEquals(Value),
    
    /// Value is one of the specified values
    OneOf(Vec<Value>),
    
    // === Presence checks ===
    
    /// Value is not null
    IsSet,
    
    /// Value is null
    IsNull,
    
    /// Value is empty (null, empty string, empty array/object)
    IsEmpty,
    
    /// Value is not empty
    IsNotEmpty,
    
    // === Validation state ===
    
    /// Field has passed validation
    IsValid,
    
    /// Field has failed validation
    IsInvalid,
    
    // === Numeric comparisons ===
    
    /// Numeric value is greater than threshold
    GreaterThan(f64),
    
    /// Numeric value is greater than or equal to threshold
    GreaterOrEqual(f64),
    
    /// Numeric value is less than threshold
    LessThan(f64),
    
    /// Numeric value is less than or equal to threshold
    LessOrEqual(f64),
    
    /// Numeric value is within range (inclusive)
    InRange { min: f64, max: f64 },
    
    // === String operations ===
    
    /// String contains substring
    Contains(String),
    
    /// String starts with prefix
    StartsWith(String),
    
    /// String ends with suffix
    EndsWith(String),
    
    /// String matches regex pattern
    MatchesPattern(String),
    
    // === Boolean ===
    
    /// Boolean value is true
    IsTrue,
    
    /// Boolean value is false
    IsFalse,
}

impl FieldCondition {
    /// Evaluate the condition against a value
    #[must_use]
    pub fn evaluate(&self, value: &Value) -> bool {
        match self {
            Self::Equals(expected) => value == expected,
            Self::NotEquals(expected) => value != expected,
            Self::OneOf(values) => values.iter().any(|v| v == value),
            
            Self::IsSet => !value.is_null(),
            Self::IsNull => value.is_null(),
            Self::IsEmpty => Self::is_value_empty(value),
            Self::IsNotEmpty => !Self::is_value_empty(value),
            
            // IsValid/IsInvalid require context, return false here
            Self::IsValid | Self::IsInvalid => false,
            
            Self::GreaterThan(threshold) => {
                Self::get_numeric(value).is_some_and(|n| n > *threshold)
            }
            Self::GreaterOrEqual(threshold) => {
                Self::get_numeric(value).is_some_and(|n| n >= *threshold)
            }
            Self::LessThan(threshold) => {
                Self::get_numeric(value).is_some_and(|n| n < *threshold)
            }
            Self::LessOrEqual(threshold) => {
                Self::get_numeric(value).is_some_and(|n| n <= *threshold)
            }
            Self::InRange { min, max } => {
                Self::get_numeric(value).is_some_and(|n| n >= *min && n <= *max)
            }
            
            Self::Contains(substring) => {
                Self::get_string(value).is_some_and(|s| s.contains(substring))
            }
            Self::StartsWith(prefix) => {
                Self::get_string(value).is_some_and(|s| s.starts_with(prefix))
            }
            Self::EndsWith(suffix) => {
                Self::get_string(value).is_some_and(|s| s.ends_with(suffix))
            }
            Self::MatchesPattern(pattern) => {
                Self::get_string(value).is_some_and(|s| {
                    regex::Regex::new(pattern)
                        .map(|re| re.is_match(s))
                        .unwrap_or(false)
                })
            }
            
            Self::IsTrue => value.as_boolean() == Some(true),
            Self::IsFalse => value.as_boolean() == Some(false),
        }
    }
    
    /// Check if this condition requires validation state
    #[must_use]
    pub fn requires_validation_state(&self) -> bool {
        matches!(self, Self::IsValid | Self::IsInvalid)
    }
    
    fn is_value_empty(value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::Text(t) => t.as_str().is_empty(),
            Value::Array(arr) => arr.is_empty(),
            Value::Object(obj) => obj.is_empty(),
            _ => false,
        }
    }
    
    fn get_numeric(value: &Value) -> Option<f64> {
        value.as_float_lossy().map(|f| f.value())
    }
    
    fn get_string(value: &Value) -> Option<&str> {
        value.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equals() {
        let cond = FieldCondition::Equals(Value::text("test"));
        assert!(cond.evaluate(&Value::text("test")));
        assert!(!cond.evaluate(&Value::text("other")));
    }

    #[test]
    fn test_is_set() {
        let cond = FieldCondition::IsSet;
        assert!(cond.evaluate(&Value::text("hello")));
        assert!(!cond.evaluate(&Value::Null));
    }

    #[test]
    fn test_greater_than() {
        let cond = FieldCondition::GreaterThan(10.0);
        assert!(cond.evaluate(&Value::integer(15)));
        assert!(!cond.evaluate(&Value::integer(5)));
    }

    #[test]
    fn test_contains() {
        let cond = FieldCondition::Contains("test".to_string());
        assert!(cond.evaluate(&Value::text("this is a test")));
        assert!(!cond.evaluate(&Value::text("hello world")));
    }
}
```

**Step 2: Export from mod.rs**

```rust
// In crates/nebula-parameter/src/core/mod.rs, add:
pub mod field_condition;
pub use field_condition::*;
```

**Step 3: Run tests**

Run: `cargo test -p nebula-parameter field_condition`
Expected: All tests pass

**Step 4: Commit**

```bash
git add crates/nebula-parameter/src/core/field_condition.rs crates/nebula-parameter/src/core/mod.rs
git commit -m "feat(nebula-parameter): add FieldCondition enum"
```

---

### Task 2.2: Create cross_field.rs with CrossFieldRule enum

**Files:**
- Create: `crates/nebula-parameter/src/core/cross_field.rs`
- Modify: `crates/nebula-parameter/src/core/mod.rs`

**Step 1: Create cross_field.rs**

```rust
//! Cross-field validation rules
//!
//! Rules for comparing the current field against other fields.

use nebula_core::ParameterKey;
use serde::{Deserialize, Serialize};

/// Rule comparing current field to another field
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CrossFieldRule {
    // === Equality ===
    
    /// Current field must equal the other field
    EqualsField(ParameterKey),
    
    /// Current field must not equal the other field
    NotEqualsField(ParameterKey),
    
    // === Numeric comparisons ===
    
    /// Current field must be greater than the other field
    GreaterThanField(ParameterKey),
    
    /// Current field must be greater than or equal to the other field
    GreaterOrEqualField(ParameterKey),
    
    /// Current field must be less than the other field
    LessThanField(ParameterKey),
    
    /// Current field must be less than or equal to the other field
    LessOrEqualField(ParameterKey),
    
    // === String operations ===
    
    /// Current field contains the value of the other field
    ContainsField(ParameterKey),
    
    /// Current field's value is contained in the other field
    ContainedInField(ParameterKey),
    
    // === Date/time comparisons ===
    
    /// Current field date is before the other field date
    BeforeField(ParameterKey),
    
    /// Current field date is after the other field date
    AfterField(ParameterKey),
}

impl CrossFieldRule {
    /// Get the field this rule references
    #[must_use]
    pub fn referenced_field(&self) -> &ParameterKey {
        match self {
            Self::EqualsField(f)
            | Self::NotEqualsField(f)
            | Self::GreaterThanField(f)
            | Self::GreaterOrEqualField(f)
            | Self::LessThanField(f)
            | Self::LessOrEqualField(f)
            | Self::ContainsField(f)
            | Self::ContainedInField(f)
            | Self::BeforeField(f)
            | Self::AfterField(f) => f,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_referenced_field() {
        let key = ParameterKey::new("password").unwrap();
        let rule = CrossFieldRule::EqualsField(key.clone());
        assert_eq!(rule.referenced_field(), &key);
    }
}
```

**Step 2: Export from mod.rs**

```rust
// In crates/nebula-parameter/src/core/mod.rs, add:
pub mod cross_field;
pub use cross_field::*;
```

**Step 3: Run tests**

Run: `cargo test -p nebula-parameter cross_field`
Expected: All tests pass

**Step 4: Commit**

```bash
git add crates/nebula-parameter/src/core/cross_field.rs crates/nebula-parameter/src/core/mod.rs
git commit -m "feat(nebula-parameter): add CrossFieldRule enum"
```

---

### Task 2.3: Create form_rule.rs with FormRule enum

**Files:**
- Create: `crates/nebula-parameter/src/core/form_rule.rs`
- Modify: `crates/nebula-parameter/src/core/mod.rs`

**Step 1: Create form_rule.rs**

```rust
//! Form-level validation rules
//!
//! Rules that span multiple fields in a form/parameter group.

use super::FieldCondition;
use nebula_core::ParameterKey;
use serde::{Deserialize, Serialize};

/// Rule spanning multiple fields
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormRule {
    /// At least one field must be set
    AtLeastOneOf(Vec<ParameterKey>),
    
    /// Exactly one field must be set
    ExactlyOneOf(Vec<ParameterKey>),
    
    /// All fields must be set, or none
    AllOrNone(Vec<ParameterKey>),
    
    /// Fields are mutually exclusive (at most one set)
    MutuallyExclusive(Vec<ParameterKey>),
    
    /// If condition field matches, then require other fields
    DependentFields {
        condition_field: ParameterKey,
        condition: FieldCondition,
        required_fields: Vec<ParameterKey>,
    },
    
    /// If condition field matches, then forbid other fields
    ForbiddenFields {
        condition_field: ParameterKey,
        condition: FieldCondition,
        forbidden_fields: Vec<ParameterKey>,
    },
    
    /// Combine rules with AND
    All(Vec<FormRule>),
    
    /// Combine rules with OR
    Any(Vec<FormRule>),
    
    /// Negate a rule
    Not(Box<FormRule>),
}

impl FormRule {
    /// Create AtLeastOneOf rule
    pub fn at_least_one_of(fields: impl IntoIterator<Item = ParameterKey>) -> Self {
        Self::AtLeastOneOf(fields.into_iter().collect())
    }
    
    /// Create ExactlyOneOf rule
    pub fn exactly_one_of(fields: impl IntoIterator<Item = ParameterKey>) -> Self {
        Self::ExactlyOneOf(fields.into_iter().collect())
    }
    
    /// Create AllOrNone rule
    pub fn all_or_none(fields: impl IntoIterator<Item = ParameterKey>) -> Self {
        Self::AllOrNone(fields.into_iter().collect())
    }
    
    /// Create MutuallyExclusive rule
    pub fn mutually_exclusive(fields: impl IntoIterator<Item = ParameterKey>) -> Self {
        Self::MutuallyExclusive(fields.into_iter().collect())
    }
    
    /// Create DependentFields rule
    pub fn dependent_fields(
        condition_field: ParameterKey,
        condition: FieldCondition,
        required_fields: impl IntoIterator<Item = ParameterKey>,
    ) -> Self {
        Self::DependentFields {
            condition_field,
            condition,
            required_fields: required_fields.into_iter().collect(),
        }
    }
    
    /// Get all fields this rule depends on
    #[must_use]
    pub fn dependencies(&self) -> Vec<ParameterKey> {
        match self {
            Self::AtLeastOneOf(fields)
            | Self::ExactlyOneOf(fields)
            | Self::AllOrNone(fields)
            | Self::MutuallyExclusive(fields) => fields.clone(),
            
            Self::DependentFields {
                condition_field,
                required_fields,
                ..
            } => {
                let mut deps = vec![condition_field.clone()];
                deps.extend(required_fields.clone());
                deps
            }
            
            Self::ForbiddenFields {
                condition_field,
                forbidden_fields,
                ..
            } => {
                let mut deps = vec![condition_field.clone()];
                deps.extend(forbidden_fields.clone());
                deps
            }
            
            Self::All(rules) | Self::Any(rules) => {
                rules.iter().flat_map(|r| r.dependencies()).collect()
            }
            
            Self::Not(rule) => rule.dependencies(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> ParameterKey {
        ParameterKey::new(s).unwrap()
    }

    #[test]
    fn test_at_least_one_of() {
        let rule = FormRule::at_least_one_of([key("email"), key("phone")]);
        let deps = rule.dependencies();
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_dependent_fields() {
        let rule = FormRule::dependent_fields(
            key("payment_type"),
            FieldCondition::Equals(nebula_value::Value::text("card")),
            [key("card_number"), key("expiry")],
        );
        let deps = rule.dependencies();
        assert_eq!(deps.len(), 3);
    }
}
```

**Step 2: Export from mod.rs and run tests**

Run: `cargo test -p nebula-parameter form_rule`
Expected: All tests pass

**Step 3: Commit**

```bash
git add crates/nebula-parameter/src/core/form_rule.rs crates/nebula-parameter/src/core/mod.rs
git commit -m "feat(nebula-parameter): add FormRule enum for form-level validation"
```

---

## Phase 3: ValidationContext

### Task 3.1: Create validation_context.rs

**Files:**
- Create: `crates/nebula-parameter/src/core/validation_context.rs`
- Modify: `crates/nebula-parameter/src/core/mod.rs`

**Step 1: Create validation_context.rs**

```rust
//! Validation context for cross-field validation
//!
//! Contains all parameter values and validation state for evaluating
//! cross-field conditions.

use super::{FieldCondition, ParameterValues};
use nebula_core::ParameterKey;
use nebula_validator::core::ValidationError;
use nebula_value::Value;
use std::collections::HashMap;

/// Context for validation containing all parameter values and state
#[derive(Debug, Clone, Default)]
pub struct ValidationContext {
    /// All parameter values
    values: ParameterValues,
    
    /// Validation state for each parameter (true = valid, false = invalid)
    validation_state: HashMap<ParameterKey, bool>,
    
    /// Validation errors for each parameter
    errors: HashMap<ParameterKey, Vec<ValidationError>>,
}

impl ValidationContext {
    /// Create a new empty context
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Create from existing ParameterValues
    #[must_use]
    pub fn from_values(values: ParameterValues) -> Self {
        Self {
            values,
            validation_state: HashMap::new(),
            errors: HashMap::new(),
        }
    }
    
    // === Value access ===
    
    /// Get a parameter value by key
    #[must_use]
    pub fn get(&self, key: &ParameterKey) -> Option<&Value> {
        self.values.get(key.clone())
    }
    
    /// Get a string value
    #[must_use]
    pub fn get_string(&self, key: &ParameterKey) -> Option<&str> {
        self.get(key).and_then(|v| v.as_str())
    }
    
    /// Get a numeric value as f64
    #[must_use]
    pub fn get_number(&self, key: &ParameterKey) -> Option<f64> {
        self.get(key)
            .and_then(|v| v.as_float_lossy())
            .map(|f| f.value())
    }
    
    /// Get all values
    #[must_use]
    pub fn values(&self) -> &ParameterValues {
        &self.values
    }
    
    // === Validation state ===
    
    /// Check if a parameter has been validated and is valid
    #[must_use]
    pub fn is_valid(&self, key: &ParameterKey) -> bool {
        self.validation_state.get(key).copied() == Some(true)
    }
    
    /// Check if a parameter has been validated and is invalid
    #[must_use]
    pub fn is_invalid(&self, key: &ParameterKey) -> bool {
        self.validation_state.get(key).copied() == Some(false)
    }
    
    /// Mark a parameter as valid
    pub fn mark_valid(&mut self, key: ParameterKey) {
        self.validation_state.insert(key.clone(), true);
        self.errors.remove(&key);
    }
    
    /// Mark a parameter as invalid with errors
    pub fn mark_invalid(&mut self, key: ParameterKey, errs: Vec<ValidationError>) {
        self.validation_state.insert(key.clone(), false);
        self.errors.insert(key, errs);
    }
    
    /// Get validation errors for a parameter
    #[must_use]
    pub fn get_errors(&self, key: &ParameterKey) -> Option<&Vec<ValidationError>> {
        self.errors.get(key)
    }
    
    // === Condition evaluation ===
    
    /// Evaluate a field condition against a specific field
    #[must_use]
    pub fn evaluate_condition(&self, key: &ParameterKey, condition: &FieldCondition) -> bool {
        match condition {
            FieldCondition::IsValid => self.is_valid(key),
            FieldCondition::IsInvalid => self.is_invalid(key),
            _ => {
                if let Some(value) = self.get(key) {
                    condition.evaluate(value)
                } else {
                    false
                }
            }
        }
    }
    
    // === Builder pattern ===
    
    /// Add a value (builder pattern)
    #[must_use]
    pub fn with_value(mut self, key: ParameterKey, value: Value) -> Self {
        self.values.set(key, value);
        self
    }
    
    /// Set a value
    pub fn set_value(&mut self, key: ParameterKey, value: Value) {
        self.values.set(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> ParameterKey {
        ParameterKey::new(s).unwrap()
    }

    #[test]
    fn test_get_value() {
        let ctx = ValidationContext::new()
            .with_value(key("name"), Value::text("Alice"));
        
        assert_eq!(ctx.get_string(&key("name")), Some("Alice"));
    }

    #[test]
    fn test_validation_state() {
        let mut ctx = ValidationContext::new();
        
        ctx.mark_valid(key("email"));
        assert!(ctx.is_valid(&key("email")));
        assert!(!ctx.is_invalid(&key("email")));
        
        ctx.mark_invalid(key("password"), vec![ValidationError::new("weak", "Too weak")]);
        assert!(ctx.is_invalid(&key("password")));
    }

    #[test]
    fn test_evaluate_condition() {
        let ctx = ValidationContext::new()
            .with_value(key("age"), Value::integer(25));
        
        assert!(ctx.evaluate_condition(&key("age"), &FieldCondition::GreaterThan(18.0)));
        assert!(!ctx.evaluate_condition(&key("age"), &FieldCondition::GreaterThan(30.0)));
    }
}
```

**Step 2: Export and run tests**

Run: `cargo test -p nebula-parameter validation_context`
Expected: All tests pass

**Step 3: Commit**

```bash
git add crates/nebula-parameter/src/core/validation_context.rs crates/nebula-parameter/src/core/mod.rs
git commit -m "feat(nebula-parameter): add ValidationContext for cross-field validation"
```

---

## Phase 4: Rewrite ParameterValidation

### Task 4.1: Create new validation.rs (replace old)

**Files:**
- Rewrite: `crates/nebula-parameter/src/core/validation.rs`

**Step 1: Backup old validation.rs**

Run: `cp crates/nebula-parameter/src/core/validation.rs crates/nebula-parameter/src/core/validation_old.rs`

**Step 2: Write new validation.rs**

```rust
//! Parameter validation using nebula-validator
//!
//! This module provides a thin wrapper over `nebula-validator` for parameter validation.
//!
//! # Examples
//!
//! ```rust
//! use nebula_parameter::prelude::*;
//! use nebula_validator::validators::prelude::*;
//!
//! // String validation
//! let validation = ParameterValidation::string(
//!     min_length(3).and(max_length(50))
//! );
//!
//! // Number validation
//! let validation = ParameterValidation::integer(min(18i64));
//!
//! // Cross-field validation
//! let validation = ParameterValidation::string(not_empty())
//!     .equals_field("password");
//! ```

use super::{CrossFieldRule, FieldCondition, ValidationContext};
use nebula_core::ParameterKey;
use nebula_validator::core::{TypedValidator, ValidationError};
use nebula_validator::validators::value::*;
use nebula_value::Value;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Trait for validators that work on Value
pub trait ValueValidator: Send + Sync {
    /// Validate a Value
    fn validate_value(&self, value: &Value) -> Result<(), ValidationError>;
}

impl<V> ValueValidator for ValueString<V>
where
    V: TypedValidator<Input = str, Output = (), Error = ValidationError> + Send + Sync,
{
    fn validate_value(&self, value: &Value) -> Result<(), ValidationError> {
        self.validate(value)
    }
}

impl<V> ValueValidator for ValueInteger<V>
where
    V: TypedValidator<Input = i64, Output = (), Error = ValidationError> + Send + Sync,
{
    fn validate_value(&self, value: &Value) -> Result<(), ValidationError> {
        self.validate(value)
    }
}

impl<V> ValueValidator for ValueFloat<V>
where
    V: TypedValidator<Input = f64, Output = (), Error = ValidationError> + Send + Sync,
{
    fn validate_value(&self, value: &Value) -> Result<(), ValidationError> {
        self.validate(value)
    }
}

impl<V> ValueValidator for ValueBoolean<V>
where
    V: TypedValidator<Input = bool, Output = (), Error = ValidationError> + Send + Sync,
{
    fn validate_value(&self, value: &Value) -> Result<(), ValidationError> {
        self.validate(value)
    }
}

/// Condition for when validation should be applied
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhenCondition {
    /// Field to check
    pub field: ParameterKey,
    /// Condition that must be met
    pub condition: FieldCondition,
}

/// Validation configuration for parameters
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ParameterValidation {
    /// Single-field validator (type-erased)
    #[serde(skip)]
    validator: Option<Arc<dyn ValueValidator>>,
    
    /// Cross-field comparison rules
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    cross_field_rules: Vec<CrossFieldRule>,
    
    /// Conditional validation - only validate when condition met
    #[serde(skip_serializing_if = "Option::is_none")]
    when_condition: Option<WhenCondition>,
    
    /// Custom validation message
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl std::fmt::Debug for ParameterValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParameterValidation")
            .field("has_validator", &self.validator.is_some())
            .field("cross_field_rules", &self.cross_field_rules)
            .field("when_condition", &self.when_condition)
            .field("message", &self.message)
            .finish()
    }
}

impl ParameterValidation {
    /// Create a new empty validation
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    
    // === Type constructors ===
    
    /// Create string validation
    pub fn string<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = str, Output = (), Error = ValidationError> + Send + Sync + 'static,
    {
        Self {
            validator: Some(Arc::new(ValueString::new(validator))),
            cross_field_rules: Vec::new(),
            when_condition: None,
            message: None,
        }
    }
    
    /// Create integer validation
    pub fn integer<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = i64, Output = (), Error = ValidationError> + Send + Sync + 'static,
    {
        Self {
            validator: Some(Arc::new(ValueInteger::new(validator))),
            cross_field_rules: Vec::new(),
            when_condition: None,
            message: None,
        }
    }
    
    /// Create float validation
    pub fn float<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = f64, Output = (), Error = ValidationError> + Send + Sync + 'static,
    {
        Self {
            validator: Some(Arc::new(ValueFloat::new(validator))),
            cross_field_rules: Vec::new(),
            when_condition: None,
            message: None,
        }
    }
    
    /// Create boolean validation
    pub fn boolean<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = bool, Output = (), Error = ValidationError> + Send + Sync + 'static,
    {
        Self {
            validator: Some(Arc::new(ValueBoolean::new(validator))),
            cross_field_rules: Vec::new(),
            when_condition: None,
            message: None,
        }
    }
    
    // === Cross-field methods ===
    
    /// Add equals_field rule
    #[must_use]
    pub fn equals_field(mut self, field: impl Into<ParameterKey>) -> Self {
        self.cross_field_rules.push(CrossFieldRule::EqualsField(field.into()));
        self
    }
    
    /// Add not_equals_field rule
    #[must_use]
    pub fn not_equals_field(mut self, field: impl Into<ParameterKey>) -> Self {
        self.cross_field_rules.push(CrossFieldRule::NotEqualsField(field.into()));
        self
    }
    
    /// Add greater_than_field rule
    #[must_use]
    pub fn greater_than_field(mut self, field: impl Into<ParameterKey>) -> Self {
        self.cross_field_rules.push(CrossFieldRule::GreaterThanField(field.into()));
        self
    }
    
    /// Add less_than_field rule
    #[must_use]
    pub fn less_than_field(mut self, field: impl Into<ParameterKey>) -> Self {
        self.cross_field_rules.push(CrossFieldRule::LessThanField(field.into()));
        self
    }
    
    // === Conditional methods ===
    
    /// Only validate when field matches condition
    #[must_use]
    pub fn when_field(mut self, field: impl Into<ParameterKey>, condition: FieldCondition) -> Self {
        self.when_condition = Some(WhenCondition {
            field: field.into(),
            condition,
        });
        self
    }
    
    /// Only validate when field is set (not null)
    #[must_use]
    pub fn when_field_set(self, field: impl Into<ParameterKey>) -> Self {
        self.when_field(field, FieldCondition::IsSet)
    }
    
    /// Only validate when field is empty
    #[must_use]
    pub fn when_field_empty(self, field: impl Into<ParameterKey>) -> Self {
        self.when_field(field, FieldCondition::IsEmpty)
    }
    
    /// Only validate when field equals value
    #[must_use]
    pub fn when_field_equals(self, field: impl Into<ParameterKey>, value: Value) -> Self {
        self.when_field(field, FieldCondition::Equals(value))
    }
    
    // === Message ===
    
    /// Set custom error message
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
    
    /// Get custom message
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    
    // === Validation ===
    
    /// Validate a value (single-field only)
    pub async fn validate(&self, value: &Value) -> Result<(), ValidationError> {
        if let Some(validator) = &self.validator {
            let result = validator.validate_value(value);
            
            if let Err(mut err) = result {
                if let Some(msg) = &self.message {
                    err = ValidationError::new(&err.code, msg);
                }
                return Err(err);
            }
        }
        Ok(())
    }
    
    /// Validate with context (supports cross-field)
    pub async fn validate_with_context(
        &self,
        value: &Value,
        context: &ValidationContext,
    ) -> Result<(), ValidationError> {
        // Check when condition
        if let Some(when) = &self.when_condition {
            if !context.evaluate_condition(&when.field, &when.condition) {
                return Ok(()); // Skip validation
            }
        }
        
        // Run single-field validation
        self.validate(value).await?;
        
        // Run cross-field rules
        for rule in &self.cross_field_rules {
            self.validate_cross_field_rule(value, rule, context)?;
        }
        
        Ok(())
    }
    
    fn validate_cross_field_rule(
        &self,
        value: &Value,
        rule: &CrossFieldRule,
        context: &ValidationContext,
    ) -> Result<(), ValidationError> {
        let other_value = context.get(rule.referenced_field());
        
        match rule {
            CrossFieldRule::EqualsField(field) => {
                if other_value != Some(value) {
                    return Err(ValidationError::new(
                        "fields_not_equal",
                        self.message.as_deref().unwrap_or("Fields must be equal"),
                    ).with_field(field.as_str()));
                }
            }
            CrossFieldRule::NotEqualsField(field) => {
                if other_value == Some(value) {
                    return Err(ValidationError::new(
                        "fields_must_differ",
                        self.message.as_deref().unwrap_or("Fields must be different"),
                    ).with_field(field.as_str()));
                }
            }
            CrossFieldRule::GreaterThanField(field) => {
                let this_num = value.as_float_lossy().map(|f| f.value());
                let other_num = other_value.and_then(|v| v.as_float_lossy()).map(|f| f.value());
                
                match (this_num, other_num) {
                    (Some(a), Some(b)) if a <= b => {
                        return Err(ValidationError::new(
                            "must_be_greater",
                            self.message.as_deref().unwrap_or("Must be greater than referenced field"),
                        ).with_field(field.as_str()));
                    }
                    _ => {}
                }
            }
            CrossFieldRule::LessThanField(field) => {
                let this_num = value.as_float_lossy().map(|f| f.value());
                let other_num = other_value.and_then(|v| v.as_float_lossy()).map(|f| f.value());
                
                match (this_num, other_num) {
                    (Some(a), Some(b)) if a >= b => {
                        return Err(ValidationError::new(
                            "must_be_less",
                            self.message.as_deref().unwrap_or("Must be less than referenced field"),
                        ).with_field(field.as_str()));
                    }
                    _ => {}
                }
            }
            // TODO: Implement remaining rules
            _ => {}
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_validator::validators::string::min_length;

    #[tokio::test]
    async fn test_string_validation() {
        let validation = ParameterValidation::string(min_length(3));
        
        assert!(validation.validate(&Value::text("hello")).await.is_ok());
        assert!(validation.validate(&Value::text("hi")).await.is_err());
    }

    #[tokio::test]
    async fn test_equals_field() {
        let validation = ParameterValidation::new()
            .equals_field(ParameterKey::new("password").unwrap());
        
        let ctx = ValidationContext::new()
            .with_value(
                ParameterKey::new("password").unwrap(),
                Value::text("secret123"),
            );
        
        // Same value - should pass
        assert!(validation
            .validate_with_context(&Value::text("secret123"), &ctx)
            .await
            .is_ok());
        
        // Different value - should fail
        assert!(validation
            .validate_with_context(&Value::text("different"), &ctx)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_when_field_set() {
        let validation = ParameterValidation::string(min_length(5))
            .when_field_set(ParameterKey::new("country").unwrap());
        
        // Country not set - validation skipped
        let ctx = ValidationContext::new();
        assert!(validation
            .validate_with_context(&Value::text("hi"), &ctx)
            .await
            .is_ok());
        
        // Country set - validation runs
        let ctx = ValidationContext::new()
            .with_value(ParameterKey::new("country").unwrap(), Value::text("US"));
        assert!(validation
            .validate_with_context(&Value::text("hi"), &ctx)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_custom_message() {
        let validation = ParameterValidation::string(min_length(5))
            .with_message("Username must be at least 5 characters");
        
        let result = validation.validate(&Value::text("hi")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("5 characters"));
    }
}
```

**Step 3: Run tests**

Run: `cargo test -p nebula-parameter validation`
Expected: All tests pass

**Step 4: Clean up - remove old backup**

Run: `rm crates/nebula-parameter/src/core/validation_old.rs`

**Step 5: Commit**

```bash
git add crates/nebula-parameter/src/core/validation.rs
git commit -m "refactor(nebula-parameter): rewrite ParameterValidation with thin wrapper API"
```

---

## Phase 5: Integration and Cleanup

### Task 5.1: Update prelude and exports

**Files:**
- Modify: `crates/nebula-parameter/src/lib.rs`
- Modify: `crates/nebula-parameter/src/core/mod.rs`

**Step 1: Update exports in core/mod.rs**

Ensure all new types are exported:

```rust
pub mod collection;
pub mod cross_field;
pub mod display;
mod error;
pub mod field_condition;
pub mod form_rule;
mod kind;
mod metadata;
pub mod option;
pub mod traits;
pub mod validation;
pub mod validation_context;
pub mod values;

pub use collection::*;
pub use cross_field::*;
pub use display::*;
pub use error::*;
pub use field_condition::*;
pub use form_rule::*;
pub use kind::*;
pub use metadata::*;
pub use option::SelectOption;
pub use traits::*;
pub use validation::*;
pub use validation_context::*;
pub use values::*;
```

**Step 2: Run full test suite**

Run: `cargo test -p nebula-parameter`
Expected: All tests pass

**Step 3: Run clippy**

Run: `cargo clippy -p nebula-parameter -- -D warnings`
Expected: No warnings

**Step 4: Commit**

```bash
git add crates/nebula-parameter/src/
git commit -m "feat(nebula-parameter): complete validation API redesign"
```

---

### Task 5.2: Update existing parameter types to use new validation

**Files:**
- Modify: `crates/nebula-parameter/src/types/text.rs`
- Modify: `crates/nebula-parameter/src/types/number.rs`
- (and others as needed)

**Step 1: Check if parameter types reference old validation API**

Search for usage of old builder pattern and update as needed.

**Step 2: Run full workspace tests**

Run: `cargo test --workspace`
Expected: All tests pass

**Step 3: Commit**

```bash
git add .
git commit -m "refactor(nebula-parameter): update parameter types to use new validation API"
```

---

## Phase 6: Documentation and Examples

### Task 6.1: Add documentation examples

**Files:**
- Modify: `crates/nebula-parameter/src/core/validation.rs` (add doc examples)
- Create: `crates/nebula-parameter/examples/validation.rs` (optional)

**Step 1: Add comprehensive doc examples to validation.rs**

Add examples showing:
- Single-field validation
- Cross-field validation
- Conditional validation
- Custom messages

**Step 2: Run doc tests**

Run: `cargo test -p nebula-parameter --doc`
Expected: All doc tests pass

**Step 3: Commit**

```bash
git add .
git commit -m "docs(nebula-parameter): add validation API documentation and examples"
```

---

## Summary

| Phase | Tasks | Description |
|-------|-------|-------------|
| 1 | 1.1-1.4 | Add Value type wrappers to nebula-validator |
| 2 | 2.1-2.3 | Add FieldCondition, CrossFieldRule, FormRule enums |
| 3 | 3.1 | Add ValidationContext |
| 4 | 4.1 | Rewrite ParameterValidation |
| 5 | 5.1-5.2 | Integration and cleanup |
| 6 | 6.1 | Documentation |

**Total estimated tasks:** 12
**Commit frequency:** After each task
