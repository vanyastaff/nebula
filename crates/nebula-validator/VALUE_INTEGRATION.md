# Value Integration: Old vs New Approach

## Problem with Old Approach

The original integration required manual wrapper types for each combination:

```rust
// OLD WAY - Manual wrappers everywhere ❌
use nebula_validator::bridge::value::{for_string, for_i64, for_array};

let string_validator = for_string(min_length(5));
let number_validator = for_i64(positive());
let array_validator = for_array(min_size(2));

// Problems:
// 1. Need separate function for each type
// 2. Can't easily compose validators
// 3. Verbose and repetitive
// 4. Hard to extend
```

## New Trait-Based Approach ✨

Uses Rust's trait system for automatic type extraction:

```rust
// NEW WAY - Automatic with traits ✅
use nebula_validator::bridge::ValueValidatorExt;

let string_validator = min_length(5).for_value();
let number_validator = positive().for_value();
let array_validator = min_size(2).for_value();

// Benefits:
// 1. One method works for all types
// 2. Composable with combinators
// 3. Concise and elegant
// 4. Extensible via trait implementations
```

## How It Works

### 1. Extract Trait

The core is the `Extract` trait that defines how to get a type from Value:

```rust
pub trait Extract {
    fn extract(value: &Value) -> Result<&Self, ValidationError>;
}

// Implemented for common types:
impl Extract for str { ... }
impl Extract for bool { ... }
impl Extract for nebula_value::Array { ... }
```

### 2. ValueAdapter

Automatically wraps any validator to work with Value:

```rust
pub struct ValueAdapter<V> {
    inner: V,
}

impl<V> TypedValidator for ValueAdapter<V>
where
    V: TypedValidator,
    V::Input: Extract,  // ← Key constraint
{
    type Input = Value;
    
    fn validate(&self, input: &Value) -> Result<...> {
        let extracted = <V::Input>::extract(input)?;  // Auto extraction
        self.inner.validate(extracted).map_err(Into::into)
    }
}
```

### 3. Extension Trait

Adds `.for_value()` to ALL validators via blanket impl:

```rust
pub trait ValueValidatorExt: TypedValidator + Sized {
    fn for_value(self) -> ValueAdapter<Self>
    where
        Self::Input: Extract,
    {
        ValueAdapter::new(self)
    }
}

// Blanket implementation:
impl<T: TypedValidator> ValueValidatorExt for T {}
```

## Comparison Examples

### String Validation

```rust
// OLD
let validator = for_string(min_length(5));

// NEW
let validator = min_length(5).for_value();
```

### With Combinators

```rust
// OLD - Awkward composition
let inner = and(min_length(3), max_length(20));
let validator = for_string(inner);

// NEW - Natural composition
let validator = and(min_length(3), max_length(20)).for_value();
```

### Type Safety

```rust
let validator = min_length(5).for_value();

// Correct type:
validator.validate(&Value::Text("hello".into()))  // ✓ OK

// Wrong type - automatic error:
validator.validate(&Value::Integer(42))  // ✗ Type mismatch error
```

## Extending for Custom Types

Want to support a new type? Just implement `Extract`:

```rust
// Add support for your custom type
impl Extract for MyCustomType {
    fn extract(value: &Value) -> Result<&Self, ValidationError> {
        match value {
            Value::Custom(c) if c.is_my_type() => Ok(c.as_my_type()),
            _ => Err(ValidationError::type_mismatch(...))
        }
    }
}

// Now ANY validator for MyCustomType automatically works with Value!
my_custom_validator.for_value()  // ✨ Just works
```

## Architecture Benefits

### 1. Type-Driven Design
The compiler ensures correctness - if `Extract` is implemented, it will work.

### 2. Zero Cost Abstraction
The `ValueAdapter` is a zero-sized wrapper - no runtime overhead.

### 3. Composability
Works with all combinators (and, or, not, map, when, cached, etc.).

### 4. Extensibility
New types only need one trait impl, not new wrapper functions.

### 5. Ergonomics
One method (`.for_value()`) instead of many (`for_string`, `for_i64`, etc.).

## Migration Guide

If you have existing code using the old bridge:

```rust
// Before
use nebula_validator::bridge::value::for_string;
let validator = for_string(min_length(5));

// After
use nebula_validator::bridge::ValueValidatorExt;
let validator = min_length(5).for_value();
```

The old API is still available for backward compatibility but is considered deprecated.

## Summary

| Feature | Old Approach | New Approach |
|---------|-------------|--------------|
| Type extraction | Manual per type | Automatic via trait |
| API surface | Many functions | One method |
| Composability | Limited | Full |
| Extensibility | Hard | Easy |
| Type safety | Runtime errors | Compile-time errors |
| Boilerplate | High | Minimal |
| Performance | Same | Same (zero-cost) |

**The trait-based approach is the recommended way to integrate with nebula-value.**
