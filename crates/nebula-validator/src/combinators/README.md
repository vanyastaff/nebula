# Validator Combinators

Combinators for composing validators in powerful and expressive ways.

## Overview

Combinators are higher-order functions that take validators as input and return new validators. They enable building complex validation logic from simple, reusable components.

## Available Combinators

### 🔗 Logical Combinators

#### AND - Both must pass
```rust
let validator = min_length(5).and(max_length(20));

assert!(validator.validate("hello").is_ok());     // 5-20 chars ✓
assert!(validator.validate("hi").is_err());       // too short ✗
assert!(validator.validate("verylongstring...").is_err()); // too long ✗
```

**Properties:**
- ✅ Short-circuits on first failure
- ✅ Associative: `(a AND b) AND c = a AND (b AND c)`
- ✅ Complexity: max(left, right)

#### OR - At least one must pass
```rust
let validator = exact_length(5).or(exact_length(10));

assert!(validator.validate("hello").is_ok());      // length 5 ✓
assert!(validator.validate("helloworld").is_ok()); // length 10 ✓
assert!(validator.validate("hi").is_err());        // neither ✗
```

**Properties:**
- ✅ Short-circuits on first success
- ✅ Associative: `(a OR b) OR c = a OR (b OR c)`
- ✅ Returns combined error if all fail

#### NOT - Must not pass
```rust
let validator = contains("test").not();

assert!(validator.validate("hello world").is_ok());  // no "test" ✓
assert!(validator.validate("test string").is_err()); // has "test" ✗
```

**Properties:**
- ✅ Double negation: `NOT(NOT(a)) = a`
- ✅ De Morgan's laws apply

### 🔄 Transformational Combinators

#### MAP - Transform output
```rust
let validator = min_length(5).map(|_| "Valid!");

let result = validator.validate("hello")?;
assert_eq!(result, "Valid!");
```

**Variants:**
- `when(condition)` - Validate if condition is true
- `unless(condition)` - Validate if condition is false
- `when_not_empty()` - Only validate non-empty strings
- `when_some()` - Only validate Some values

**Properties:**
- ✅ Skips validation when condition is false
- ⚠️ Cannot be cached (condition may vary)
- ✅ Chainable for multiple conditions

### 🎯 Optional Combinators

#### OPTIONAL - None is valid
```rust
let validator = min_length(5).optional();

assert!(validator.validate(&None).is_ok());           // None ✓
assert!(validator.validate(&Some("hello")).is_ok());  // Valid Some ✓
assert!(validator.validate(&Some("hi")).is_err());    // Invalid Some ✗
```

#### REQUIRED_SOME - Must be Some
```rust
let validator = required_some(min_length(5));

assert!(validator.validate(&None).is_err());          // None ✗
assert!(validator.validate(&Some("hello")).is_ok());  // Valid Some ✓
assert!(validator.validate(&Some("hi")).is_err());    // Invalid Some ✗
```

**Aliases:**
- `nullable(validator)` - Same as `optional()`

### ⚡ Performance Combinators

#### CACHED - Memoize results
```rust
let validator = expensive_database_check().cached();

validator.validate("test@example.com")?;  // Slow: DB lookup
validator.validate("test@example.com")?;  // Fast: cached!
```

**Features:**
- ✅ Thread-safe (uses RwLock)
- ✅ Caches both success and errors
- ✅ O(1) lookup after first call
- ⚠️ Unbounded memory (use LRU variant for bounded)

**Variants:**
```rust
// LRU cache with capacity limit
#[cfg(feature = "lru")]
let validator = lru_cached(expensive_validator(), 100);
```

**API:**
```rust
validator.cache_size();      // Get cache size
validator.clear_cache();     // Clear all cached entries
validator.cache_stats();     // Get cache statistics
```

---

## Composition Patterns

### Sequential Validation
```rust
// All must pass in order
let validator = required()
    .and(string())
    .and(min_length(5))
    .and(max_length(20))
    .and(alphanumeric());
```

### Alternative Validation
```rust
// Accept multiple formats
let validator = url_format()
    .or(email_format())
    .or(phone_format());
```

### Exclusion Validation
```rust
// Must NOT match
let validator = contains("admin").not()
    .and(contains("root").not())
    .and(contains("system").not());
```

### Conditional with Fallback
```rust
// Validate email format only if not empty
let validator = email_format()
    .when(|s| !s.is_empty())
    .or(exact_length(0));
```

### Complex Business Logic
```rust
let password_validator = min_length(8)
    .and(max_length(64))
    .and(contains_digit())
    .and(contains_uppercase())
    .and(contains_lowercase())
    .and(contains_special_char())
    .when(|p| !p.is_empty())
    .optional();
```

### Performance-Optimized
```rust
// Cache expensive validations
let email_validator = not_null()
    .and(email_format())
    .and(domain_exists_check().cached())  // Cache DNS lookups
    .and(not_disposable_email());
```

---

## Best Practices

### 1. Order Validators by Cost

Run cheap validators first to fail fast:

```rust
// ✅ Good: cheap to expensive
let validator = not_null()         // O(1) - instant
    .and(min_length(5))            // O(1) - instant
    .and(email_format())           // O(n) - fast
    .and(regex_check())            // O(n²) - slow
    .and(database_unique().cached()); // I/O - slowest

// ❌ Bad: expensive first
let validator = database_unique()  // Slow I/O first!
    .and(not_null());              // Could have failed instantly
```

### 2. Use Short-Circuit Logic

Take advantage of AND/OR short-circuiting:

```rust
// AND short-circuits on first failure
let validator = cheap_check()      // Fails fast
    .and(expensive_check());       // Only runs if cheap_check passes

// OR short-circuits on first success
let validator = quick_valid()      // Succeeds fast
    .or(slow_valid());             // Only runs if quick_valid fails
```

### 3. Cache Expensive Operations

Use caching for validators that:
- Make network requests
- Query databases
- Run complex regex
- Perform heavy computations

```rust
let validator = api_lookup().cached();  // Cache API responses
let validator = dns_check().cached();   // Cache DNS lookups
let validator = complex_regex().cached(); // Cache regex results
```

**Don't cache:**
- Simple checks (overhead > benefit)
- Validators with side effects
- Validators depending on time/state

### 4. Group Related Validations

Create reusable validator compositions:

```rust
fn username_validator() -> impl TypedValidator<Input = str, Output = (), Error = ValidationError> {
    min_length(3)
        .and(max_length(20))
        .and(alphanumeric_with_underscore())
        .and(starts_with_letter())
}

fn email_validator() -> impl TypedValidator<Input = str, Output = (), Error = ValidationError> {
    not_empty()
        .and(email_format())
        .and(max_length(254))
}

// Use them
let user_validator = username_validator()
    .and(email_validator());
```

### 5. Handle Optional Values Explicitly

```rust
// ✅ Good: explicit about optionality
let validator = email_format().optional();

// ✅ Good: required with validation
let validator = required_some(email_format());

// ❌ Bad: unclear if None is valid
let validator = email_format();  // What about None?
```

### 6. Use Conditional Validation Wisely

```rust
// ✅ Good: clear condition
let validator = strong_password()
    .when(|u| u.is_admin);

// ✅ Good: multiple conditions
let validator = url_format()
    .when(|s| !s.is_empty())
    .when(|s| s.starts_with("http"));

// ⚠️ Caution: complex conditions
let validator = expensive_check()
    .when(|s| complex_business_logic(s));  // Hard to reason about
```

### 7. Compose Small, Testable Units

```rust
// ✅ Good: small, testable validators
let min_validator = min_length(5);
let max_validator = max_length(20);
let combined = min_validator.and(max_validator);

// Each can be tested independently
#[test]
fn test_min_length() {
    assert!(min_validator.validate("hello").is_ok());
    assert!(min_validator.validate("hi").is_err());
}

// ❌ Bad: monolithic validator
let validator = MonolithicValidator { /* many rules */ };
```

---

## Performance Considerations

### Complexity Analysis

| Combinator | Time Complexity | Space Complexity | Notes |
|------------|----------------|------------------|-------|
| `AND` | O(left + right) | O(1) | Short-circuits |
| `OR` | O(left + right) | O(1) | Short-circuits |
| `NOT` | O(inner) | O(1) | Simple inversion |
| `MAP` | O(inner) | O(1) | Zero-cost transform |
| `WHEN` | O(inner) or O(1) | O(1) | Depends on condition |
| `OPTIONAL` | O(inner) or O(1) | O(1) | O(1) for None |
| `CACHED` | O(1) after first | O(n) entries | Hash lookup |

### Benchmarking

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_combinators(c: &mut Criterion) {
    let simple = min_length(5);
    let combined = min_length(5).and(max_length(20));
    let cached = min_length(5).cached();

    c.bench_function("simple", |b| {
        b.iter(|| simple.validate(black_box("hello")))
    });

    c.bench_function("combined", |b| {
        b.iter(|| combined.validate(black_box("hello")))
    });

    c.bench_function("cached_hit", |b| {
        // Prime cache
        cached.validate("hello").unwrap();
        b.iter(|| cached.validate(black_box("hello")))
    });
}

criterion_group!(benches, bench_combinators);
criterion_main!(benches);
```

---

## Advanced Patterns

### Type-Level Validator Chains

Build validators that prove properties at compile-time:

```rust
// Each step refines the type
let validator = not_null()        // Proves not null
    .and(string())                 // Proves is string
    .and(min_length(5))           // Proves >= 5 chars
    .map(|_| Validated::new());   // Create refined type
```

### Dynamic Validator Construction

Build validators at runtime based on configuration:

```rust
fn build_validator(config: &Config) -> Box<dyn Validator> {
    let mut validator = Box::new(not_null()) as Box<dyn Validator>;
    
    if let Some(min) = config.min_length {
        validator = Box::new(validator.and(min_length(min)));
    }
    
    if config.cache_enabled {
        validator = Box::new(validator.cached());
    }
    
    validator
}
```

### Validator Registry

Register and retrieve validators by name:

```rust
let mut registry = ValidatorRegistry::new();

registry.register("email", email_format());
registry.register("username", username_validator());

// Use registered validators
let validator = registry.get("email")?.and(registry.get("username")?);
```

---

## Algebraic Laws

Combinators follow algebraic laws that enable reasoning about compositions:

### AND Laws
```rust
// Associativity
(a AND b) AND c = a AND (b AND c)

// Identity
a AND AlwaysValid = a

// Annihilator
a AND AlwaysFails = AlwaysFails
```

### OR Laws
```rust
// Associativity
(a OR b) OR c = a OR (b OR c)

// Identity
a OR AlwaysFails = a

// Annihilator
a OR AlwaysValid = AlwaysValid

// Commutativity (for success/failure)
a OR b ≈ b OR a
```

### NOT Laws
```rust
// Double negation
NOT(NOT(a)) = a

// De Morgan's laws
NOT(a AND b) = NOT(a) OR NOT(b)
NOT(a OR b) = NOT(a) AND NOT(b)
```

### MAP Laws
```rust
// Identity
map(|x| x) = validator

// Composition
map(f).map(g) = map(|x| g(f(x)))
```

---

## Testing Combinators

### Unit Tests

```rust
#[test]
fn test_and_combinator() {
    let validator = min_length(5).and(max_length(10));
    
    assert!(validator.validate("hello").is_ok());
    assert!(validator.validate("hi").is_err());
    assert!(validator.validate("verylongstring").is_err());
}
```

### Property-Based Tests

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_and_associativity(s: String) {
        let left = (min_length(3).and(max_length(10))).and(alphanumeric());
        let right = min_length(3).and(max_length(10).and(alphanumeric()));
        
        assert_eq!(
            left.validate(&s).is_ok(),
            right.validate(&s).is_ok()
        );
    }
}
```

### Integration Tests

```rust
#[test]
fn test_complex_email_validation() {
    let validator = not_empty()
        .and(email_format())
        .and(max_length(254))
        .and(not_disposable().cached())
        .optional();
    
    // Test various scenarios
    assert!(validator.validate(&None).is_ok());
    assert!(validator.validate(&Some("user@example.com")).is_ok());
    assert!(validator.validate(&Some("invalid")).is_err());
}
```

---

## Migration Guide

### From v1 to v2

```rust
// v1: Dynamic dispatch
let validator: Box<dyn Validator> = Box::new(min_length(5).and(max_length(20)));

// v2: Static dispatch (preferred)
let validator = min_length(5).and(max_length(20));

// v2: Explicit dynamic dispatch when needed
let validator: Box<dyn Validator> = Box::new(min_length(5).and(max_length(20)));
```

### Error Handling Changes

```rust
// v1: Simple errors
let result = validator.validate(input); // Result<(), String>

// v2: Structured errors
let result = validator.validate(input); // Result<(), ValidationError>
match result {
    Err(e) => println!("Error: {} (code: {})", e.message, e.code),
    Ok(_) => println!("Valid!"),
}
```

---

## Feature Flags

```toml
[dependencies]
nebula-validator = { version = "2.0", features = ["lru"] }
```

| Feature | Description | Default |
|---------|-------------|---------|
| `async` | Async validator support | ✓ |
| `lru` | LRU cache combinator | ✗ |
| `serde` | Serialization support | ✗ |

---

## Examples

See the `examples/` directory for complete examples:

- `examples/basic_combinators.rs` - Basic usage
- `examples/complex_composition.rs` - Complex validation logic
- `examples/performance.rs` - Performance optimization
- `examples/async_validation.rs` - Async validators

Run examples:
```bash
cargo run --example basic_combinators
```

---

## Further Reading

- [Core Traits](../core/README.md) - Core validator traits
- [Validators](../validators/README.md) - Built-in validators
- [Architecture Decisions](../docs/adr/) - Design rationale
- [Performance Guide](../docs/performance.md) - Optimization tips
- `map(f)` - Transform with function
- `map_to(value)` - Replace with constant
- `map_unit()` - Discard output
- `map_with_input(f)` - Access original input

**Properties:**
- ✅ Preserves validation logic
- ✅ Composable: `map(f).map(g) = map(|x| g(f(x)))`
- ✅ Identity: `map(|x| x) = validator`

### ⚡ Conditional Combinators

#### WHEN - Conditional validation
```rust
let validator = min_length(10).when(|s| s.starts_with("long_"));

assert!(validator.validate("short").is_ok());        // condition false, skipped ✓
assert!(validator.validate("long_enough").is_ok());  // condition true, valid ✓
assert!(validator.validate("long_").is_err());       // condition true, too short ✗
```

**Variants:**