# Validation Context

Validation context system for cross-field validation and complex business rules.

## Overview

`ValidationContext` provides a way to pass additional data during validation,
enabling validators to access multiple fields and implement complex business
logic that depends on relationships between values.

## Key Components

### ValidationContext

The main context struct that holds validation data:

```rust
use nebula_validator::core::ValidationContext;

let mut ctx = ValidationContext::new();
ctx.insert("max_length", 100usize);
ctx.insert("user_role", "admin".to_string());

let max_len: Option<&usize> = ctx.get("max_length");
```

### ContextualValidator Trait

Trait for validators that need context:

```rust
use nebula_validator::core::{ContextualValidator, ValidationContext, ValidationError};

struct DateRangeValidator;

impl ContextualValidator for DateRangeValidator {
    type Input = DateRange;
    type Output = ();
    type Error = ValidationError;

    fn validate_with_context(
        &self,
        input: &DateRange,
        ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        if input.start >= input.end {
            return Err(ValidationError::new(
                "invalid_range",
                "Start must be before end"
            ));
        }
        Ok(())
    }
}
```

### ValidationContextBuilder

Fluent API for building contexts:

```rust
use nebula_validator::core::ValidationContextBuilder;

let ctx = ValidationContextBuilder::new()
    .with("max_items", 100usize)
    .with("min_length", 5usize)
    .with("user_role", "admin".to_string())
    .build();
```

## Use Cases

### 1. Cross-Field Validation

Validate relationships between multiple fields:

```rust
struct PasswordMatch;

impl ContextualValidator for PasswordMatch {
    type Input = User;
    type Output = ();
    type Error = ValidationError;

    fn validate_with_context(
        &self,
        input: &User,
        _ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        if input.password != input.password_confirmation {
            return Err(ValidationError::new(
                "password_mismatch",
                "Passwords must match"
            ));
        }
        Ok(())
    }
}
```

### 2. Conditional Validation

Make validation rules conditional based on context:

```rust
struct ConditionalRequired;

impl ContextualValidator for ConditionalRequired {
    type Input = Form;
    type Output = ();
    type Error = ValidationError;

    fn validate_with_context(
        &self,
        input: &Form,
        ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        let require_email = ctx
            .get::<bool>("require_email")
            .copied()
            .unwrap_or(false);

        if require_email && input.email.is_empty() {
            return Err(ValidationError::new(
                "email_required",
                "Email is required"
            ));
        }
        Ok(())
    }
}
```

### 3. Business Rules

Implement complex business logic:

```rust
struct DiscountValidator;

impl ContextualValidator for DiscountValidator {
    type Input = Order;
    type Output = ();
    type Error = ValidationError;

    fn validate_with_context(
        &self,
        input: &Order,
        ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        let max_discount = ctx
            .get::<f64>("max_discount_percent")
            .copied()
            .unwrap_or(10.0);

        if input.discount_percent > max_discount {
            return Err(ValidationError::new(
                "discount_too_high",
                format!("Discount cannot exceed {}%", max_discount)
            ));
        }
        Ok(())
    }
}
```

### 4. Field Path Tracking

Track nested field paths for detailed error messages:

```rust
let mut ctx = ValidationContext::new();

ctx.push_field("user");
ctx.push_field("profile");
ctx.push_field("email");

assert_eq!(ctx.field_path(), "user.profile.email");

// Use in error messages
let error = ValidationError::new("invalid_email", "Invalid format")
    .with_field(&ctx.field_path());
```

## Features

### Hierarchical Data

Context supports parent-child relationships:

```rust
let mut parent = ValidationContext::new();
parent.insert("global_max", 1000usize);

let child = ValidationContext::with_parent(parent);
// Child can access parent values
assert_eq!(child.get::<usize>("global_max"), Some(&1000));
```

### Type-Safe Value Storage

Values are stored with type safety:

```rust
let mut ctx = ValidationContext::new();
ctx.insert("count", 42usize);

// Correct type
let count: Option<&usize> = ctx.get("count");
assert_eq!(count, Some(&42));

// Wrong type returns None
let count: Option<&String> = ctx.get("count");
assert_eq!(count, None);
```

### Path Management

Built-in field path tracking for nested validation:

```rust
let mut ctx = ValidationContext::new();

ctx.push_field("address");
ctx.push_field("street");
assert_eq!(ctx.field_path(), "address.street");

ctx.pop_field();
assert_eq!(ctx.field_path(), "address");

ctx.clear_path();
assert_eq!(ctx.field_path(), "");
```

## Integration with TypedValidator

Existing `TypedValidator` implementations can be adapted:

```rust
use nebula_validator::core::context::ContextAdapter;

struct MinLength { min: usize }

impl TypedValidator for MinLength {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        // ... validation logic
    }
}

// Adapt to work with context
let adapter = ContextAdapter::new(MinLength { min: 5 });

// Now can be used with context
let ctx = ValidationContext::new();
adapter.validate_with_context("hello", &ctx)?;
```

## Best Practices

### 1. Use Builder for Complex Contexts

```rust
let ctx = ValidationContextBuilder::new()
    .with("user_role", "admin".to_string())
    .with("max_file_size", 10_000_000usize)
    .with("allowed_formats", vec!["jpg", "png"])
    .build();
```

### 2. Document Context Requirements

```rust
/// Validates user permissions.
///
/// # Context Requirements
///
/// - `user_role`: &str - Current user's role
/// - `required_permission`: &str - Required permission level
struct PermissionValidator;
```

### 3. Provide Sensible Defaults

```rust
let max_length = ctx
    .get::<usize>("max_length")
    .copied()
    .unwrap_or(100); // Default to 100
```

### 4. Use Field Paths for Nested Objects

```rust
fn validate_nested_object(obj: &NestedObj, ctx: &mut ValidationContext) {
    ctx.push_field("field_name");
    // ... validate field
    ctx.pop_field();
}
```

## Performance Considerations

- Context operations are O(1) for local data
- Parent lookup is O(depth) but typically shallow
- Use `contains()` before `get()` if checking existence
- Consider caching frequently accessed context values

## Thread Safety

`ValidationContext` uses `Box<dyn Any + Send + Sync>`, making it:

- **Send**: Can be transferred between threads
- **Sync**: Can be shared between threads (with proper synchronization)

## See Also

- [ContextualValidator](../traits.rs) - Core trait definition
- [TypedValidator](../traits.rs) - Standard validator trait
- [ValidationError](../error.rs) - Error type
- [Combinators](../../combinators/mod.rs) - Validator combinators
