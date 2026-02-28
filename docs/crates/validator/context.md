# Validation Context

`ValidationContext` enables cross-field validation — cases where a field's validity
depends on the value of another field (e.g., password confirmation, date ranges).

---

## ValidationContext

A typed key-value store with a parent chain and a field-path tracker.

```rust
pub struct ValidationContext {
    data: HashMap<String, Box<dyn Any + Send + Sync>>,
    parent: Option<Arc<ValidationContext>>,
    field_path: Vec<String>,
}
```

### Construction

```rust
// Empty context
let ctx = ValidationContext::new();

// With parent (child inherits parent's data via Arc)
let child = ValidationContext::with_parent(Arc::new(parent_ctx));

// Builder
let ctx = ValidationContextBuilder::new()
    .with("max_items", 100usize)
    .with("min_length", 5usize)
    .build();
```

### Data Access

```rust
ctx.insert("max_age", 120u32);
ctx.get::<u32>("max_age")  // Option<&u32>
ctx.get_mut::<u32>("max_age")
ctx.contains("max_age")    // bool

// Parent chain traversal: local → parent → grandparent
let child = ValidationContext::with_parent(Arc::new(ctx));
child.get::<u32>("max_age") // finds in parent
```

### Field Path Tracking

```rust
ctx.push_field("user");
ctx.push_field("address");
ctx.push_field("zipcode");
ctx.field_path()  // "user.address.zipcode"
ctx.pop_field();
ctx.clear_path();
```

### Parent-Child Split

```rust
// Convert a context into an Arc and get a fresh child sharing the parent's data
let (parent_arc, child_ctx) = ctx.child();
```

---

## ContextualValidator

Trait for validators that need the context to run:

```rust
pub trait ContextualValidator {
    type Input: ?Sized;

    fn validate_with_context(
        &self,
        input: &Self::Input,
        ctx: &ValidationContext,
    ) -> Result<(), ValidationError>;
}
```

### Example: Password Confirmation

```rust
struct PasswordConfirmation;

impl ContextualValidator for PasswordConfirmation {
    type Input = RegisterForm;

    fn validate_with_context(
        &self,
        input: &RegisterForm,
        _ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        if input.password != input.password_confirmation {
            return Err(ValidationError::new(
                "password_mismatch",
                "Passwords do not match",
            ));
        }
        Ok(())
    }
}
```

### Example: Date Range

```rust
struct DateRangeValidator;

impl ContextualValidator for DateRangeValidator {
    type Input = DateRange;

    fn validate_with_context(
        &self,
        input: &DateRange,
        _ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        if input.start >= input.end {
            return Err(ValidationError::new(
                "invalid_date_range",
                "Start date must be before end date",
            ));
        }
        Ok(())
    }
}
```

---

## ContextAdapter

Wraps any `Validate<T>` into a `ContextualValidator`, providing backward compatibility:

```rust
let adapter = ContextAdapter::<_, str>::new(min_length(5));
let ctx = ValidationContext::new();
adapter.validate_with_context("hello", &ctx)?;
```

---

## When to Use Context vs Plain Validate

| Scenario | Use |
|---|---|
| Single-field constraint (length, range, format) | `Validate<T>` |
| Multiple independent fields | Compose with `Field` combinator |
| Fields that depend on each other | `ContextualValidator` |
| Business rules spanning the whole struct | `ContextualValidator` |
