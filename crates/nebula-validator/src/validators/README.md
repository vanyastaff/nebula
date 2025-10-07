# Built-in Validators

Comprehensive collection of ready-to-use validators for common scenarios.

## Overview

This module provides 40+ validators organized into four categories:

- **String** (15 validators) - Length, patterns, formats
- **Numeric** (7 validators) - Range, properties
- **Collection** (8 validators) - Size, elements, structure
- **Logical** (4 validators) - Boolean, nullable

## Quick Start

```rust
use nebula_validator::validators::prelude::*;

// String validation
let username = min_length(3)
    .and(max_length(20))
    .and(alphanumeric());

// Numeric validation
let age = in_range(18, 100);

// Collection validation
let tags = min_size(1).and(max_size(10)).and(unique());

// Format validation
let email_validator = not_empty().and(email());
```

---

## String Validators

### Length Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `min_length(n)` | Minimum length | `min_length(5)` |
| `max_length(n)` | Maximum length | `max_length(20)` |
| `exact_length(n)` | Exact length | `exact_length(10)` |
| `length_range(min, max)` | Length range | `length_range(5, 20)` |
| `not_empty()` | Not empty | `not_empty()` |

```rust
// Username: 3-20 characters
let validator = min_length(3).and(max_length(20));

// ZIP code: exactly 5 digits
let validator = exact_length(5).and(numeric());

// Password: at least 8 characters
let validator = min_length(8);
```

### Pattern Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `contains(s)` | Contains substring | `contains("@")` |
| `starts_with(s)` | Starts with prefix | `starts_with("http://")` |
| `ends_with(s)` | Ends with suffix | `ends_with(".com")` |
| `alphanumeric()` | Only letters/numbers | `alphanumeric()` |
| `alphabetic()` | Only letters | `alphabetic()` |
| `numeric()` | Only numbers | `numeric()` |
| `lowercase()` | All lowercase | `lowercase()` |
| `uppercase()` | All uppercase | `uppercase()` |

```rust
// Username: alphanumeric, 3-20 chars
let username = alphanumeric()
    .and(min_length(3))
    .and(max_length(20));

// Tag: lowercase, starts with #
let tag = lowercase().and(starts_with("#"));
```

### Format Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `email()` | Email format | `email()` |
| `url()` | URL format | `url()` |
| `uuid()` | UUID format | `uuid()` |
| `matches_regex(pattern)` | Custom regex | `matches_regex(r"^\d{3}-\d{4}$")` |

```rust
// Email validation
let validator = not_empty().and(email());

// Phone: ###-####
let phone = matches_regex(r"^\d{3}-\d{4}$").unwrap();

// URL: http(s) only
let validator = url();
```

---

## Numeric Validators

### Range Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `min(n)` | Minimum value | `min(0)` |
| `max(n)` | Maximum value | `max(100)` |
| `in_range(min, max)` | Value range | `in_range(18, 65)` |

```rust
// Age: 18-100
let age = in_range(18, 100);

// Score: 0-100
let score = min(0).and(max(100));

// Temperature: above freezing
let temp = min(0);
```

### Property Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `positive()` | Positive number | `positive()` |
| `negative()` | Negative number | `negative()` |
| `even()` | Even number | `even()` |
| `odd()` | Odd number | `odd()` |

```rust
// Positive integer
let count = positive();

// Even number
let validator = even();

// Odd page number
let page = odd().and(positive());
```

---

## Collection Validators

### Size Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `min_size(n)` | Minimum size | `min_size(1)` |
| `max_size(n)` | Maximum size | `max_size(10)` |
| `exact_size(n)` | Exact size | `exact_size(3)` |
| `not_empty_collection()` | Not empty | `not_empty_collection()` |

```rust
// Tags: 1-10 items
let tags = min_size(1).and(max_size(10));

// Triplet: exactly 3 items
let triplet = exact_size(3);

// Non-empty list
let items = not_empty_collection();
```

### Element Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `all(validator)` | All elements valid | `all(positive())` |
| `any(validator)` | At least one valid | `any(even())` |
| `contains_element(x)` | Contains element | `contains_element(42)` |
| `unique()` | All unique | `unique()` |

```rust
// All positive numbers
let numbers = all(positive());

// At least one even number
let has_even = any(even());

// Unique tags
let tags = unique();
```

### Structure Validators (Maps)

| Validator | Description | Example |
|-----------|-------------|---------|
| `has_key(k)` | Has specific key | `has_key("email")` |

```rust
// Map must have "email" key
let validator = has_key("email");
```

---

## Logical Validators

### Boolean Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `is_true()` | Must be true | `is_true()` |
| `is_false()` | Must be false | `is_false()` |

```rust
// Terms must be accepted
let terms_accepted = is_true();

// Debug mode disabled
let debug = is_false();
```

### Nullable Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `required()` | Must be Some | `required()` |
| `not_null()` | Not None (alias) | `not_null()` |

```rust
// Required field
let email = required();

// Not null
let id = not_null();
```

---

## Composition Examples

### User Registration

```rust
use nebula_validator::validators::prelude::*;

// Username: 3-20 alphanumeric chars
let username = min_length(3)
    .and(max_length(20))
    .and(alphanumeric());

// Email: valid format
let email = not_empty().and(email());

// Password: 8+ chars, mixed case
let password = min_length(8);

// Age: 18-100
let age = in_range(18, 100);
```

### Product Form

```rust
// Product name: 1-100 chars
let name = min_length(1).and(max_length(100));

// Price: positive
let price = positive();

// Tags: 1-10 unique tags
let tags = min_size(1)
    .and(max_size(10))
    .and(unique());

// Description: optional, max 500 chars
let description = max_length(500).optional();
```

### Search Query

```rust
// Query: 2-50 chars
let query = min_length(2).and(max_length(50));

// Filters: 0-20 items
let filters = max_size(20);

// Page: positive integer
let page = positive();

// Page size: 10-100
let page_size = in_range(10, 100);
```

---

## Advanced Usage

### Custom Validators

Combine built-in validators to create custom ones:

```rust
// Strong password validator
fn strong_password() -> impl TypedValidator<Input = str, Output = (), Error = ValidationError> {
    min_length(8)
        .and(contains_uppercase())
        .and(contains_lowercase())
        .and(contains_digit())
}

// Username validator
fn valid_username() -> impl TypedValidator<Input = str, Output = (), Error = ValidationError> {
    min_length(3)
        .and(max_length(20))
        .and(alphanumeric())
        .and(starts_with_letter())
}
```

### Conditional Validation

```rust
// Only validate if not empty
let validator = email().when(|s: &&str| !s.is_empty());

// Validate URL only if starts with http
let validator = url().when(|s: &&str| s.starts_with("http"));
```

### Performance Optimization

```rust
// Order by cost: cheap first
let validator = not_empty()          // O(1)
    .and(min_length(3))              // O(1)
    .and(max_length(20))             // O(1)
    .and(alphanumeric())             // O(n)
    .and(email())                    // O(n) with regex
    .and(check_database().cached()); // I/O, cached
```

---

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_validator::validators::prelude::*;

    #[test]
    fn test_username_validation() {
        let validator = min_length(3)
            .and(max_length(20))
            .and(alphanumeric());

        assert!(validator.validate("john").is_ok());
        assert!(validator.validate("hi").is_err());
        assert!(validator.validate("john_doe").is_err());
    }

    #[test]
    fn test_email_validation() {
        let validator = email();

        assert!(validator.validate("user@example.com").is_ok());
        assert!(validator.validate("invalid").is_err());
    }
}
```

---

## Performance Tips

1. **Order validators by cost**:
   ```rust
   // ✅ Good: cheap first
   not_empty().and(email()).and(database_check())
   
   // ❌ Bad: expensive first
   database_check().and(not_empty())
   ```

2. **Cache expensive validators**:
   ```rust
   let validator = email().and(dns_check().cached());
   ```

3. **Use specific validators**:
   ```rust
   // ✅ Good: specific
   alphanumeric()
   
   // ❌ Bad: generic regex
   matches_regex(r"^[a-zA-Z0-9]+$")
   ```

---

## Migration from v1

```rust
// v1: Dynamic dispatch
let validator: Box<dyn Validator> = Box::new(min_length(5));

// v2: Static dispatch (faster)
let validator = min_length(5);

// v2: Explicit boxing when needed
let validator: Box<dyn Validator> = Box::new(min_length(5));
```

---

## See Also

- [Core Traits](../core/README.md) - Validator traits
- [Combinators](../combinators/README.md) - Composing validators
- [Macros](../macros/README.md) - Creating validators