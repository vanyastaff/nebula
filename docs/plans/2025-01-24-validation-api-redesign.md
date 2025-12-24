# Validation API Redesign

## Overview

Redesign the parameter validation system in `nebula-parameter` to eliminate boilerplate builders and leverage `nebula-validator` directly. The new API should be clean, composable, and support cross-field validation.

## Goals

1. **Remove builder boilerplate** - No more `StringValidationBuilder`, `NumberValidationBuilder`
2. **Thin wrapper over nebula-validator** - Updates to nebula-validator automatically available
3. **Cross-field validation** - Support for field comparisons and conditional validation
4. **Form-level rules** - Validation across multiple fields
5. **Consistent with DisplayCondition pattern** - Similar enum-based approach where appropriate

## Current Problems

```rust
// Current: Verbose builder pattern that duplicates nebula-validator
let validation = ParameterValidation::string()
    .min_length(3)
    .max_length(50)
    .pattern(r"^[a-z]+$")
    .required()  // duplicates metadata.required
    .build();
```

## New API Design

### Level 1: Single-Field Validation

Thin wrapper that extracts typed value from `Value` and delegates to nebula-validator.

```rust
use nebula_validator::validators::prelude::*;

// String validation
let username = ParameterValidation::string(
    min_length(3).and(max_length(50)).and(alphanumeric())
);

// Email with custom message
let email = ParameterValidation::string(email())
    .with_message("Please enter a valid email address");

// Integer validation
let age = ParameterValidation::integer(in_range(18, 120));

// Float validation
let price = ParameterValidation::float(positive().and(max(10000.0)));

// Boolean validation
let accepted = ParameterValidation::boolean(is_true());

// Array validation
let tags = ParameterValidation::array(
    min_size(1).and(max_size(10))
);

// Object validation
let config = ParameterValidation::object(has_key("version"));
```

### Level 2: Cross-Field Validation (on parameter)

Parameter-level validation that references other fields.

```rust
// Password confirmation
let confirm_password = ParameterValidation::string(not_empty())
    .equals_field("password")
    .with_message("Passwords must match");

// Date range
let end_date = ParameterValidation::date(valid_date())
    .greater_than_field("start_date")
    .with_message("End date must be after start date");

// Conditional validation
let api_key = ParameterValidation::string(min_length(32))
    .when_field("auth_type", FieldCondition::Equals(Value::text("api_key")));

// Only validate if another field is set
let city = ParameterValidation::string(not_empty())
    .when_field_set("country");
```

### Level 3: Form-Level Rules

Validation rules that span multiple fields, defined separately from parameters.

```rust
let form_rules = ValidationRuleSet::new()
    // At least one contact method required
    .rule(AtLeastOneOf::fields(["email", "phone", "address"]))
    
    // Exactly one authentication method
    .rule(ExactlyOneOf::fields(["api_key", "oauth_token", "basic_auth"]))
    
    // All or none - shipping fields
    .rule(AllOrNone::fields(["street", "city", "zip", "country"]))
    
    // Mutually exclusive options
    .rule(MutuallyExclusive::fields(["use_default", "custom_config"]))
    
    // Dependent fields - if payment selected, card required
    .rule(DependentFields::when("payment_method")
        .equals(Value::text("card"))
        .require(["card_number", "expiry", "cvv"]));
```

## Cross-Field Condition System

### FieldCondition Enum

Similar to `DisplayCondition`, but for validation:

```rust
/// Condition to evaluate against a field value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldCondition {
    // Value comparisons
    Equals(Value),
    NotEquals(Value),
    OneOf(Vec<Value>),
    
    // Presence checks
    IsSet,          // not null
    IsEmpty,        // null, empty string, empty array/object
    IsNotEmpty,
    
    // Validation state
    IsValid,        // field passed validation
    IsInvalid,      // field failed validation
    
    // Numeric comparisons
    GreaterThan(f64),
    LessThan(f64),
    InRange { min: f64, max: f64 },
    
    // String operations
    Contains(String),
    StartsWith(String),
    EndsWith(String),
    MatchesPattern(String),
    
    // Boolean
    IsTrue,
    IsFalse,
}
```

### CrossFieldRule Enum

For field-to-field comparisons:

```rust
/// Rule comparing current field to another field
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CrossFieldRule {
    // Equality
    EqualsField(ParameterKey),
    NotEqualsField(ParameterKey),
    
    // Numeric comparisons
    GreaterThanField(ParameterKey),
    GreaterOrEqualField(ParameterKey),
    LessThanField(ParameterKey),
    LessOrEqualField(ParameterKey),
    
    // String operations
    ContainsField(ParameterKey),      // this field contains other field's value
    ContainedInField(ParameterKey),   // this field's value is in other field
    
    // Date/time comparisons
    BeforeField(ParameterKey),
    AfterField(ParameterKey),
    
    // Array operations
    SubsetOfField(ParameterKey),
    SupersetOfField(ParameterKey),
}
```

### FormRule Enum

For multi-field rules:

```rust
/// Rule spanning multiple fields
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    
    /// Custom validation function
    Custom {
        name: String,
        fields: Vec<ParameterKey>,
        // validator stored separately (not serializable)
    },
    
    /// Combine rules with AND
    All(Vec<FormRule>),
    
    /// Combine rules with OR
    Any(Vec<FormRule>),
    
    /// Negate a rule
    Not(Box<FormRule>),
}
```

## ValidationContext

Context containing all field values for cross-field validation:

```rust
/// Context for validation containing all parameter values
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
    pub fn new() -> Self { ... }
    pub fn from_values(values: ParameterValues) -> Self { ... }
    
    // Value access
    pub fn get(&self, key: &str) -> Option<&Value> { ... }
    pub fn get_string(&self, key: &str) -> Option<&str> { ... }
    pub fn get_number(&self, key: &str) -> Option<f64> { ... }
    
    // Validation state
    pub fn is_valid(&self, key: &str) -> bool { ... }
    pub fn is_invalid(&self, key: &str) -> bool { ... }
    pub fn mark_valid(&mut self, key: &str) { ... }
    pub fn mark_invalid(&mut self, key: &str, error: ValidationError) { ... }
    
    // Condition evaluation
    pub fn evaluate_condition(&self, key: &str, condition: &FieldCondition) -> bool { ... }
}
```

## ParameterValidation Structure

```rust
/// Validation configuration for a parameter
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParameterValidation {
    /// Single-field validator (type-erased)
    #[serde(skip)]
    validator: Option<Arc<dyn ValueValidator>>,
    
    /// Cross-field comparison rules
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cross_field_rules: Vec<CrossFieldRule>,
    
    /// Conditional validation - only validate when condition met
    #[serde(skip_serializing_if = "Option::is_none")]
    when_condition: Option<WhenCondition>,
    
    /// Custom validation message
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// When to apply validation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhenCondition {
    pub field: ParameterKey,
    pub condition: FieldCondition,
}
```

## Type Constructors

```rust
impl ParameterValidation {
    // === Single-field constructors ===
    
    /// String validation (extracts &str from Value::Text)
    pub fn string<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = str, Output = (), Error = ValidationError>
            + Send + Sync + 'static
    { ... }
    
    /// Integer validation (extracts i64 from Value::Integer)
    pub fn integer<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = i64, Output = (), Error = ValidationError>
            + Send + Sync + 'static
    { ... }
    
    /// Float validation (extracts f64 from Value::Float)
    pub fn float<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = f64, Output = (), Error = ValidationError>
            + Send + Sync + 'static
    { ... }
    
    /// Boolean validation
    pub fn boolean<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = bool, Output = (), Error = ValidationError>
            + Send + Sync + 'static
    { ... }
    
    /// Array validation
    pub fn array<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = Array, Output = (), Error = ValidationError>
            + Send + Sync + 'static
    { ... }
    
    /// Object validation
    pub fn object<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = Object, Output = (), Error = ValidationError>
            + Send + Sync + 'static
    { ... }
    
    // === Cross-field methods (builder pattern) ===
    
    pub fn equals_field(mut self, field: impl Into<ParameterKey>) -> Self { ... }
    pub fn not_equals_field(mut self, field: impl Into<ParameterKey>) -> Self { ... }
    pub fn greater_than_field(mut self, field: impl Into<ParameterKey>) -> Self { ... }
    pub fn less_than_field(mut self, field: impl Into<ParameterKey>) -> Self { ... }
    
    // === Conditional methods ===
    
    pub fn when_field(mut self, field: impl Into<ParameterKey>, condition: FieldCondition) -> Self { ... }
    pub fn when_field_set(self, field: impl Into<ParameterKey>) -> Self { ... }
    pub fn when_field_empty(self, field: impl Into<ParameterKey>) -> Self { ... }
    pub fn when_field_equals(self, field: impl Into<ParameterKey>, value: Value) -> Self { ... }
    pub fn when_field_valid(self, field: impl Into<ParameterKey>) -> Self { ... }
    
    // === Message ===
    
    pub fn with_message(mut self, message: impl Into<String>) -> Self { ... }
    
    // === Validation ===
    
    /// Validate value (single-field only)
    pub async fn validate(&self, value: &Value) -> Result<(), ValidationError> { ... }
    
    /// Validate with context (supports cross-field)
    pub async fn validate_with_context(
        &self,
        value: &Value,
        context: &ValidationContext,
    ) -> Result<(), ValidationError> { ... }
}
```

## Changes to nebula-validator

New validators/combinators to add:

### validators/value.rs (NEW)

Bridge validators that work directly on `nebula_value::Value`:

```rust
/// Validator wrapper that extracts string from Value
pub struct ValueString<V>(pub V);

impl<V> TypedValidator for ValueString<V>
where
    V: TypedValidator<Input = str>
{
    type Input = Value;
    type Output = V::Output;
    type Error = ValidationError;
    
    fn validate(&self, input: &Value) -> Result<Self::Output, Self::Error> {
        let s = input.as_str()
            .ok_or_else(|| ValidationError::new("type_error", "Expected text value"))?;
        self.0.validate(s).map_err(Into::into)
    }
}

// Similar for ValueInteger, ValueFloat, ValueBoolean, ValueArray, ValueObject
```

### validators/cross.rs (NEW)

Cross-field validators:

```rust
/// Validates that two fields are equal
pub struct FieldsEqual {
    pub field_a: ParameterKey,
    pub field_b: ParameterKey,
}

/// Validates field A > field B
pub struct FieldGreaterThan {
    pub field: ParameterKey,
    pub other: ParameterKey,
}

// etc.
```

### combinators/conditional.rs (NEW or extend when.rs)

Context-aware conditional validation:

```rust
/// Validates only when a field condition is met
pub struct WhenField<V> {
    pub validator: V,
    pub field: ParameterKey,
    pub condition: FieldCondition,
}
```

## Migration Path

### Before (current API)
```rust
let validation = ParameterValidation::string()
    .min_length(3)
    .max_length(50)
    .email()
    .required()
    .build();
```

### After (new API)
```rust
let validation = ParameterValidation::string(
    min_length(3).and(max_length(50)).and(email())
);
// Note: required is handled by ParameterMetadata, not validation
```

## Files to Modify

### nebula-validator
- [ ] `src/validators/mod.rs` - Add `value` module export
- [ ] `src/validators/value.rs` - NEW: Value type wrappers
- [ ] `src/validators/cross.rs` - NEW: Cross-field validators
- [ ] `src/combinators/conditional.rs` - NEW or extend: Context-aware conditions

### nebula-parameter
- [ ] `src/core/validation.rs` - Rewrite with new API
- [ ] `src/core/validation_context.rs` - NEW: ValidationContext
- [ ] `src/core/field_condition.rs` - NEW: FieldCondition enum
- [ ] `src/core/cross_field.rs` - NEW: CrossFieldRule, FormRule
- [ ] `src/core/traits.rs` - Update Validatable trait to use new validation

## Open Questions

1. **Async validation context** - Should `ValidationContext` support async field access for lazy evaluation?

2. **Validation ordering** - When validating a form, should we validate in dependency order? (e.g., validate `password` before `confirm_password`)

3. **Error aggregation** - Should cross-field errors be attached to a specific field or be form-level errors?

4. **Serialization** - The validator itself isn't serializable. Should we store a "recipe" that can reconstruct validators? (Like storing `{type: "string", rules: [{min_length: 3}, {max_length: 50}]}`)

## Examples

### Complete Example: User Registration Form

```rust
use nebula_parameter::prelude::*;
use nebula_validator::validators::prelude::*;

// Individual parameter validations
let username = ParameterValidation::string(
    min_length(3)
        .and(max_length(20))
        .and(alphanumeric())
);

let email = ParameterValidation::string(email());

let password = ParameterValidation::string(
    min_length(8)
        .and(contains_uppercase())
        .and(contains_digit())
);

let confirm_password = ParameterValidation::string(not_empty())
    .equals_field("password")
    .with_message("Passwords must match");

let age = ParameterValidation::integer(min(18))
    .with_message("Must be 18 or older");

let phone = ParameterValidation::string(phone_number())
    .when_field_empty("email");  // Required if email not provided

// Form-level rules
let form_rules = ValidationRuleSet::new()
    .rule(AtLeastOneOf::fields(["email", "phone"]))
    .rule(DependentFields::when("newsletter")
        .equals(Value::boolean(true))
        .require(["email"]));
```

### Complete Example: Date Range Filter

```rust
let start_date = ParameterValidation::string(iso_date());

let end_date = ParameterValidation::string(iso_date())
    .greater_than_field("start_date")
    .when_field_set("start_date")
    .with_message("End date must be after start date");

let date_preset = ParameterValidation::string(
    one_of(["today", "week", "month", "year", "custom"])
);

// Form rules
let rules = ValidationRuleSet::new()
    .rule(DependentFields::when("date_preset")
        .equals(Value::text("custom"))
        .require(["start_date", "end_date"]));
```
