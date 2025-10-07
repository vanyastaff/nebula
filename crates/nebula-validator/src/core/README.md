# nebula-validator Core

Core types and traits for the Nebula validation system.

## Overview

The `core` module provides the fundamental building blocks for type-safe, composable validation:

- **Traits**: `TypedValidator`, `AsyncValidator`, `ValidatorExt`
- **Errors**: Rich, structured error types with nested support
- **Metadata**: Runtime introspection and performance tracking
- **Refined Types**: Compile-time validation guarantees
- **Type-State Pattern**: State-based validation workflow

## Key Features

### 🎯 Type Safety

Validators are generic over their input type, providing compile-time guarantees:

```rust
impl TypedValidator for MinLength {
    type Input = str;     // Only validates strings
    type Output = ();
    type Error = ValidationError;
    
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.len() >= self.min {
            Ok(())
        } else {
            Err(ValidationError::min_length("field", self.min, input.len()))
        }
    }
}
```

### 🧩 Composition

Chain validators with logical combinators:

```rust
let validator = min_length(5)
    .and(max_length(20))
    .and(alphanumeric())
    .or(exact_length(0));  // Allow empty OR valid
```

### ⚡ Zero-Cost Abstractions

Validators compile to efficient code with no runtime overhead:

```rust
// These have identical performance:
let manual = input.len() >= 5 && input.len() <= 20;
let validator = min_length(5).and(max_length(20)).validate(input).is_ok();
```

### 📝 Rich Errors

Structured errors with field paths, error codes, and parameters:

```rust
ValidationError::new("min_length", "String is too short")
    .with_field("user.username")
    .with_param("min", "5")
    .with_param("actual", "3")
    .with_help("Username must be at least 5 characters long");
```

## Module Structure

```
core/
├── traits.rs      # TypedValidator, AsyncValidator, ValidatorExt
├── error.rs       # ValidationError, ValidationErrors
├── metadata.rs    # ValidatorMetadata, ValidationComplexity
├── refined.rs     # Refined<T, V> types
├── state.rs       # Type-state pattern (Parameter<T, S>)
└── mod.rs         # Public API and utilities
```

## Core Concepts

### 1. TypedValidator Trait

The foundation of all validators:

```rust
pub trait TypedValidator {
    type Input: ?Sized;
    type Output;
    type Error: std::error::Error;
    
    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error>;
    fn metadata(&self) -> ValidatorMetadata { ... }
}
```

**Key points:**
- Generic over `Input` type for type safety
- `?Sized` bound allows validating DSTs like `str` and `[T]`
- `Output` can be `()` or a refined type
- Metadata enables introspection and optimization

### 2. Refined Types

Values that carry compile-time validation proofs:

```rust
// Create a validated string
let validator = MinLength { min: 5 };
let validated: Refined<String, MinLength> = Refined::new("hello".to_string(), &validator)?;

// Type system knows this is valid!
fn process(s: Refined<String, MinLength>) {
    // s.len() >= 5 is GUARANTEED by the type system
}
```

**Benefits:**
- Impossible states are unrepresentable
- Self-documenting function signatures
- Zero runtime cost (validated at construction)
- Compiler enforces validation

### 3. Type-State Pattern

Tracks validation state at compile-time:

```rust
// Start unvalidated
let param: Parameter<String, Unvalidated> = Parameter::new("hello".to_string());

// Validate - type changes!
let validated: Parameter<String, Validated<MinLength>> = param.validate(&validator)?;

// Safe unwrap - compiler knows it's validated
let value = validated.unwrap();
```

**States:**
- `Unvalidated` - Not yet validated
- `Validated<V>` - Validated by validator `V`

The type system prevents using unvalidated values where validated ones are required.

### 4. Combinators

Compose validators using `ValidatorExt`:

```rust
pub trait ValidatorExt: TypedValidator + Sized {
    fn and<V>(self, other: V) -> And<Self, V>;     // Logical AND
    fn or<V>(self, other: V) -> Or<Self, V>;       // Logical OR
    fn not(self) -> Not<Self>;                      // Logical NOT
    fn map<F, O>(self, f: F) -> Map<Self, F>;      // Transform output
    fn when<C>(self, condition: C) -> When<Self, C>; // Conditional
    fn cached(self) -> Cached<Self>;                // Add caching
}
```

**Example:**

```rust
let email_validator = not_null()
    .and(string())
    .and(contains("@"))
    .and(regex(r"^[\w\.-]+@[\w\.-]+\.\w+$"))
    .when(|s| !s.is_empty());
```

### 5. Error System

Structured errors with rich information:

```rust
pub struct ValidationError {
    pub code: String,                      // e.g., "min_length"
    pub message: String,                   // Human-readable message
    pub field: Option<String>,             // e.g., "user.email"
    pub params: HashMap<String, String>,   // Template parameters
    pub nested: Vec<ValidationError>,      // Nested errors
    pub severity: ErrorSeverity,           // Error/Warning/Info
    pub help: Option<String>,              // Help text
}
```

**Convenience constructors:**

```rust
ValidationError::required("email");
ValidationError::min_length("username", 5, 3);
ValidationError::out_of_range("age", 18, 100, 15);
ValidationError::type_mismatch("field", "string", "number");
```

### 6. Metadata System

Runtime introspection and optimization:

```rust
pub struct ValidatorMetadata {
    pub name: String,
    pub description: Option<String>,
    pub complexity: ValidationComplexity,  // O(1), O(n), O(n²)
    pub cacheable: bool,
    pub estimated_time: Option<Duration>,
    pub tags: Vec<String>,
}
```

**Use cases:**
- Sort validators by complexity (cheap first)
- Generate documentation
- Build admin UIs
- Cache expensive validators
- Track performance statistics

## Usage Examples

### Basic Validation

```rust
use nebula_validator::core::prelude::*;

// Define a validator
struct MinLength { min: usize }

impl TypedValidator for MinLength {
    type Input = str;
    type Output = ();
    type Error = ValidationError;
    
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.len() >= self.min {
            Ok(())
        } else {
            Err(ValidationError::min_length("", self.min, input.len()))
        }
    }
}

// Use it
let validator = MinLength { min: 5 };
assert!(validator.validate("hello").is_ok());
assert!(validator.validate("hi").is_err());
```

### Composition

```rust
let username_validator = MinLength { min: 3 }
    .and(MaxLength { max: 20 })
    .and(AlphanumericOnly);

match username_validator.validate("john_doe") {
    Ok(_) => println!("Valid username!"),
    Err(e) => println!("Invalid: {}", e),
}
```

### Refined Types

```rust
// Function that only accepts validated strings
fn create_user(username: Refined<String, UsernameValidator>) -> User {
    User {
        username: username.into_inner(),
    }
}

// Can only call with validated username
let validator = UsernameValidator::default();
let username = Refined::new("john".to_string(), &validator)?;
create_user(username);
```

### Type-State Pattern

```rust
// Build and validate a parameter
let param = ParameterBuilder::new()
    .value("hello@example.com".to_string())
    .validate(&email_validator)?
    .build();

// Type guarantees it's validated
send_email(param.unwrap());
```

### Async Validation

```rust
struct EmailExists {
    db: Database,
}

#[async_trait]
impl AsyncValidator for EmailExists {
    type Input = str;
    type Output = ();
    type Error = ValidationError;
    
    async fn validate_async(&self, input: &str) -> Result<(), ValidationError> {
        if self.db.email_exists(input).await? {
            Ok(())
        } else {
            Err(ValidationError::custom("Email not found"))
        }
    }
}

// Use it
let validator = EmailExists { db };
validator.validate_async("user@example.com").await?;
```

### Nested Errors

```rust
let user_error = ValidationError::new("user_validation", "User validation failed")
    .with_nested(vec![
        ValidationError::min_length("username", 3, 2).with_field("username"),
        ValidationError::invalid_format("email", "email").with_field("email"),
        ValidationError::out_of_range("age", 18, 100, 15).with_field("age"),
    ]);

println!("{}", user_error);
// Output:
// user_validation: User validation failed
//   Nested errors:
//     1. [username] min_length: Must be at least 3 characters (params: {"min": "3", "actual": "2"})
//     2. [email] invalid_format: Invalid format (params: {"expected": "email"})
//     3. [age] out_of_range: Value must be between 18 and 100 (params: {"min": "18", "max": "100", "actual": "15"})
```

## Performance Considerations

### Complexity Tracking

Use `ValidationComplexity` to optimize validation order:

```rust
impl TypedValidator for NotNull {
    // ...
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("NotNull")
            .with_complexity(ValidationComplexity::Constant) // O(1)
    }
}

impl TypedValidator for RegexValidator {
    // ...
    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Regex")
            .with_complexity(ValidationComplexity::Expensive) // O(n²)
    }
}

// Sort validators by complexity before validating
let validators = sort_by_complexity(vec![regex, not_null, min_length]);
```

### Caching

Enable caching for expensive validators:

```rust
let expensive_validator = DatabaseLookup { /* ... */ }
    .cached(); // Adds automatic caching

// First call: performs database lookup
expensive_validator.validate("test").await?;

// Second call: returns cached result
expensive_validator.validate("test").await?; // Fast!
```

### Statistics

Track validator performance:

```rust
let mut stats = ValidatorStatistics::new();

let start = Instant::now();
let result = validator.validate(input);
stats.record(result.is_ok(), start.elapsed());

println!("Success rate: {:.2}%", stats.success_rate());
println!("Average time: {:?}", stats.average_time);
```

## Best Practices

1. **Order validators by cost**: Run cheap validators first
   ```rust
   not_null()           // O(1) - check first
       .and(min_length(5))  // O(1) - still cheap
       .and(regex(...))     // O(n²) - expensive last
   ```

2. **Use refined types for public APIs**: Make validation requirements explicit
   ```rust
   pub fn create_user(email: Refined<String, EmailValidator>) { ... }
   ```

3. **Leverage type-state**: Prevent using unvalidated data
   ```rust
   let validated = param.validate(&validator)?;
   // Can't use param anymore - only validated
   ```

4. **Provide rich errors**: Include field paths and parameters
   ```rust
   ValidationError::new("min_length", "Too short")
       .with_field("user.password")
       .with_param("min", "8")
       .with_help("Passwords must be at least 8 characters")
   ```

5. **Cache expensive validators**: Don't repeat expensive work
   ```rust
   let db_validator = EmailExistsValidator { db }.cached();
   ```

## Testing

The core module includes comprehensive tests:

```bash
# Run all core tests
cargo test -p nebula-validator --lib core

# Run specific test
cargo test -p nebula-validator core::tests::test_refined_types

# Run with all features
cargo test -p nebula-validator --all-features
```

## Feature Flags

```toml
[features]
async = ["async-trait", "tokio"]    # Async validation support
serde = ["serde", "serde_json"]     # Serialization support
cache = ["lru"]                      # Caching support
```