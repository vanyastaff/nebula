# Combinator Error Handling

Unified error handling system for all validator combinators in nebula-validator.

## Overview

`CombinatorError` provides a consistent, composable error type for all combinators,
replacing the previous approach where each combinator had its own error type.

## Design Goals

- **Unified**: Single error type for all combinators
- **Composable**: Easy to combine and nest errors
- **Debuggable**: Rich context for error diagnosis
- **Interoperable**: Seamless conversion to/from `ValidationError`

## Error Variants

### `OrAllFailed`
Used when OR combinator fails - all alternatives were tried and failed.

```rust
let error = CombinatorError::or_all_failed(
    ValidationError::new("too_short", "Less than 5"),
    ValidationError::new("too_long", "More than 10")
);
```

### `AndFailed`
Used when AND combinator fails - one of the validators failed.

```rust
let error = CombinatorError::and_failed(
    ValidationError::new("invalid_format", "Invalid email format")
);
```

### `NotValidatorPassed`
Used when NOT combinator fails - the inner validator unexpectedly succeeded.

```rust
let error = CombinatorError::not_passed();
// Error: "Validation must NOT pass, but it did"
```

### `FieldFailed`
Used when validating struct fields - includes field name for context.

```rust
let error = CombinatorError::field_failed(
    "user.email",
    ValidationError::new("email_invalid", "Invalid email")
);
```

### `RequiredValueMissing`
Used when a required value is None (Optional/Required combinators).

```rust
let error = CombinatorError::required_missing();
// Error: "Value is required but was None"
```

### `ValidationFailed`
Generic wrapper for inner validator errors.

```rust
let error = CombinatorError::validation_failed(
    ValidationError::new("custom", "Custom validation failed")
);
```

### `MultipleFailed`
Used when multiple independent validations fail.

```rust
let errors = vec![
    ValidationError::new("err1", "First error"),
    ValidationError::new("err2", "Second error"),
];
let error = CombinatorError::multiple_failed(errors);
```

### `Custom`
For custom error scenarios not covered by other variants.

```rust
let error = CombinatorError::custom(
    "custom_code",
    "Custom error message"
);
```

## Conversions

### To ValidationError

`CombinatorError` can be converted to `ValidationError` for consistency:

```rust
let combo_error = CombinatorError::required_missing();
let validation_error: ValidationError = combo_error.into();
assert_eq!(validation_error.code, "required");
```

### From ValidationError

`ValidationError` can be wrapped in `CombinatorError`:

```rust
let ve = ValidationError::new("test", "Test error");
let combo_error: CombinatorError<ValidationError> = ve.into();
```

## Usage Examples

### Basic Usage

```rust
use nebula_validator::combinators::error::CombinatorError;
use nebula_validator::core::ValidationError;

// Create an error
let error = CombinatorError::field_failed(
    "email",
    ValidationError::new("invalid", "Invalid format")
);

// Check error type
assert!(error.is_field_error());
assert_eq!(error.field_name(), Some("email"));

// Display error
println!("{}", error);
// Output: "Validation failed for field 'email': Invalid format"
```

### Error Composition

```rust
// Combine multiple errors
let errors = vec![
    ValidationError::new("min_length", "Too short"),
    ValidationError::new("pattern", "Invalid pattern"),
];

let error = CombinatorError::multiple_failed(errors);

// Convert to ValidationError with nested errors
let ve: ValidationError = error.into();
assert_eq!(ve.nested.len(), 2);
```

### Field Path Tracking

```rust
// Create field error with path
let error = CombinatorError::field_failed(
    "user.address.zipcode",
    ValidationError::new("invalid_zipcode", "Must be 5 digits")
);

// Convert preserves field path
let ve: ValidationError = error.into();
assert_eq!(ve.field, Some("user.address.zipcode".to_string()));
```

## Migration from Legacy Error Types

Previous combinator-specific error types are deprecated but still available for
backward compatibility:

```rust
// Old way (deprecated)
use nebula_validator::combinators::or::OrError;

// New way (recommended)
use nebula_validator::combinators::error::CombinatorError;
let error = CombinatorError::or_all_failed(left_err, right_err);
```

Legacy types automatically convert to `CombinatorError`:

```rust
let old_error = OrError { left_error: err1, right_error: err2 };
let new_error: CombinatorError<_> = old_error.into();
```

## Benefits

1. **Consistency**: All combinators use the same error type
2. **Composability**: Easily nest and combine errors
3. **Debuggability**: Rich context (field names, multiple errors, etc.)
4. **Interoperability**: Seamless conversion to `ValidationError`
5. **Type Safety**: Compile-time guarantees about error handling
6. **Extensibility**: Easy to add new error variants

## See Also

- [ValidationError](../../core/error.rs) - Core error type
- [Combinators](../mod.rs) - Combinator overview
- [Field Combinator](../field.rs) - Field validation example
