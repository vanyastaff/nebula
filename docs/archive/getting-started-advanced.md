# Getting Started with Nebula Value

> Tutorial for building type-safe applications with Nebula (values: serde_json::Value, serde)

## What is Nebula Value?

Nebula Value is a type-safe value system for Rust that provides:
- **Zero-cost abstractions** - No runtime overhead compared to raw types
- **Rich validation** - Built-in validators with detailed error reporting  
- **Type safety** - Catch errors at compile time and runtime
- **Ecosystem integration** - Works seamlessly with web frameworks and databases

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
# Values: serde_json::Value + serde (nebula-value не используется)
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

## Your First Nebula Value Program

```rust
use nebula_value::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create typed values
    let name = Text::new("Alice Johnson");
    let age = Integer::new(28);
    let email = Text::new("alice@example.com");
    
    // Validate data
    age.validate(&InRange::new(18i64, 65i64))?;
    email.validate(&Email)?;
    
    println!("User: {} ({}), email: {}", 
             name.as_ref(), 
             age.as_ref(), 
             email.as_ref());
    
    Ok(())
}
```

## Core Concepts

### 1. Typed Values

Workflow data uses `serde_json::Value`; type-safe access via params/validators:

```rust
use nebula_value::prelude::*;

// These are zero-cost wrappers
let text: Text = Text::new("Hello");                    // TypedValue<TextInner>
let number: Integer = Integer::new(42);                 // TypedValue<IntegerInner>  
let flag: Bool = Bool::new(true);                       // TypedValue<BoolInner>

// Access inner values explicitly
assert_eq!(text.as_ref().len(), 5);
assert_eq!(number.as_ref().get(), 42);
assert_eq!(flag.as_ref().get(), true);

// Move out inner values
let owned_string: String = text.into_inner().into();
let raw_number: i64 = number.into_inner().get();
```

### 2. Universal Value Enum

All typed values can convert to a universal `Value` enum:

```rust
use nebula_value::prelude::*;

let name = Text::new("Bob");
let age = Integer::new(30);

// Convert to universal Value (zero-cost)
let name_value: Value = name.into_value();
let age_value: Value = age.into_value();

// Pattern match on Value
match name_value {
    Value::Text(text) => println!("Name: {}", text.as_ref()),
    _ => unreachable!(),
}

// Convert back (with validation)
let restored_name = Text::try_from(name_value)?;
```

### 3. Built-in Validation

```rust
use nebula_value::prelude::*;
use nebula_value::validation::*;

// Individual validators
let password = Text::new("mypassword123");
password.validate(&MinLength::new(8))?;
password.validate(&ContainsUppercase)?;
password.validate(&ContainsDigit)?;

// Validator combinators
let strong_password_validator = MinLength::new(8)
    .and(ContainsUppercase)
    .and(ContainsLowercase)
    .and(ContainsDigit);

password.validate(&strong_password_validator)?;

// Numeric validation
let age = Integer::new(25);
age.validate(&InRange::new(0i64, 120i64))?;
age.validate(&Positive)?;
```

## Building a User Registration System

Let's build a practical example - a user registration system with validation:

```rust
use nebula_value::prelude::*;
use nebula_value::validation::*;
use std::collections::HashMap;

#[derive(Debug)]
struct User {
    username: Text,
    email: Text,
    age: Integer,
    password: Text,
}

impl User {
    fn new(
        username: String, 
        email: String, 
        age: i64, 
        password: String
    ) -> Result<Self, ValidationErrors> {
        let mut errors = ValidationErrors::new();
        
        // Validate username
        let username = match Text::new(username) {
            username => {
                if let Err(e) = username.validate(
                    &MinLength::new(3)
                        .and(MaxLength::new(20))
                        .and(AlphanumericOnly)
                ) {
                    errors.add(e.with_path("username"));
                }
                username
            }
        };
        
        // Validate email
        let email = match Text::new(email) {
            email => {
                if let Err(e) = email.validate(&Email) {
                    errors.add(e.with_path("email"));
                }
                email
            }
        };
        
        // Validate age
        let age = match Integer::new(age) {
            age => {
                if let Err(e) = age.validate(&InRange::new(13i64, 120i64)) {
                    errors.add(e.with_path("age"));
                }
                age
            }
        };
        
        // Validate password
        let password = match Text::new(password) {
            password => {
                if let Err(e) = password.validate(
                    &MinLength::new(8)
                        .and(ContainsUppercase)
                        .and(ContainsLowercase)
                        .and(ContainsDigit)
                        .and(ContainsSpecialChar)
                ) {
                    errors.add(e.with_path("password"));
                }
                password
            }
        };
        
        // Return result
        if errors.is_empty() {
            Ok(User { username, email, age, password })
        } else {
            Err(errors)
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Valid user
    let user = User::new(
        "alice123".to_string(),
        "alice@example.com".to_string(),
        25,
        "SecurePass123!".to_string(),
    )?;
    
    println!("User created: {:?}", user);
    
    // Invalid user (multiple validation errors)
    match User::new(
        "x".to_string(),                    // Too short username
        "not-an-email".to_string(),         // Invalid email
        12,                                 // Age too low
        "weak".to_string(),                 // Weak password
    ) {
        Ok(_) => unreachable!(),
        Err(errors) => {
            println!("Validation failed with {} errors:", errors.len());
            for error in &errors.errors {
                println!("  - {}: {}", error.path, error.message);
            }
        }
    }
    
    Ok(())
}
```

## Working with Collections

```rust
use nebula_value::prelude::*;

// Arrays of typed values
let numbers = Array::from(vec![
    Integer::new(1).into_value(),
    Integer::new(2).into_value(),
    Integer::new(3).into_value(),
]);

// Validate array size
numbers.validate(&MinItems::new(1))?;
numbers.validate(&MaxItems::new(10))?;

// Maps with typed values  
let mut user_data = Map::new();
user_data.insert("name", Text::new("Charlie").into_value());
user_data.insert("age", Integer::new(35).into_value());
user_data.insert("active", Bool::new(true).into_value());

// Access values with type safety
let name: Text = user_data.try_get("name")?;
let age: Integer = user_data.try_get("age")?;

println!("User: {} ({})", name.as_ref(), age.as_ref());
```

## Error Handling

Nebula Value provides rich error information:

```rust
use nebula_value::prelude::*;

let short_password = Text::new("123");
match short_password.validate(&MinLength::new(8)) {
    Ok(()) => println!("Password is valid"),
    Err(error) => {
        println!("Validation error:");
        println!("  Code: {}", error.code);           // "min_length"
        println!("  Message: {}", error.message);     // "Must be at least 8 characters"
        println!("  Path: {}", error.path);           // Empty for single field
        
        // Access validation parameters
        if let Some(min) = error.get_param("min") {
            println!("  Required minimum: {}", min);   // "8"
        }
        if let Some(actual) = error.get_param("actual") {
            println!("  Actual length: {}", actual);   // "3"
        }
    }
}
```

## Converting to/from JSON

With the `serde` feature enabled:

```rust
use nebula_value::prelude::*;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
struct UserProfile {
    name: Text,
    age: Integer, 
    email: Text,
    active: Bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profile = UserProfile {
        name: Text::new("David"),
        age: Integer::new(30),
        email: Text::new("david@example.com"),
        active: Bool::new(true),
    };
    
    // Serialize to JSON
    let json = serde_json::to_string(&profile)?;
    println!("JSON: {}", json);
    
    // Deserialize from JSON (with automatic validation)
    let restored: UserProfile = serde_json::from_str(&json)?;
    println!("Restored: {:?}", restored);
    
    Ok(())
}
```

## Value Construction Macro

With the `macros` feature:

```rust
use nebula_value::prelude::*;

// Direct value construction (no JSON overhead)
let user_data = value!({
    "name": Text::new("Eve"),
    "age": Integer::new(22),
    "settings": value!({
        "theme": Text::new("dark"),
        "notifications": Bool::new(true)
    }),
    "tags": value!([
        Text::new("developer"),
        Text::new("rust"),
        Text::new("beginner")
    ])
});

// Access nested values
let theme: Text = user_data.try_get("settings.theme")?;
let first_tag: Text = user_data.try_get("tags[0]")?;

println!("Theme: {}, First tag: {}", theme.as_ref(), first_tag.as_ref());
```

## Best Practices

### 1. Use Dynamic Types for Most Cases

```rust
// ✅ Recommended - covers 98% of use cases
let count = Integer::new(42);        // i64-based
let amount = UInteger::new(1000);    // u64-based
let score = Float::new(98.5);        // f64-based

// ❌ Only use fixed-size types when you specifically need them
#[cfg(feature = "popular-ints")]
let flags = U8::new(0xFF);           // Only for byte manipulation
```

### 2. Validate at Boundaries

```rust
// ✅ Validate user input immediately
fn create_user(input: UserInput) -> Result<User, ValidationErrors> {
    let email = Text::new(input.email).validate(&Email)?;
    let age = Integer::new(input.age).validate(&InRange::new(18i64, 65i64))?;
    
    Ok(User { email, age })
}

// ✅ Use validated types in your domain
struct User {
    email: Text,  // Already validated
    age: Integer, // Already validated
}
```

### 3. Leverage Type Safety

```rust
use nebula_value_derive::ValueType;

// ✅ Use custom types to prevent mixing up values
#[derive(ValueType, Debug, Clone, PartialEq)]
pub struct UserId(String);

#[derive(ValueType, Debug, Clone, PartialEq)] 
pub struct SessionId(String);

// Compiler prevents mixing these up
fn get_user(id: UserId) -> User { /* ... */ }
fn get_session(id: SessionId) -> Session { /* ... */ }
```

### 4. Handle Errors Gracefully

```rust
match validation_result {
    Ok(user) => {
        // Success path
        save_user(user)?;
        Ok(())
    }
    Err(errors) => {
        // Log for debugging
        log::warn!("User validation failed: {}", errors);
        
        // Return user-friendly response
        Err(ApiError::ValidationFailed {
            errors: errors.errors.into_iter().map(|e| ApiValidationError {
                field: e.path,
                code: e.code,
                message: translate_error(&e, user_locale),
            }).collect()
        })
    }
}
```

## What's Next?

Now that you understand the basics, explore these guides:

- **[Type Guide](types.md)** - Complete reference for all built-in types
- **[Validation Guide](validation.md)** - Advanced validation patterns and custom validators
- **[Custom Types](custom-types.md)** - Creating your own domain-specific types
- **[Error Handling](error-handling.md)** - Advanced error handling patterns
- **[Integration Guides](integration/)** - Using with web frameworks and databases

## Common Questions

**Q: Why not just use `serde_json::Value`?**  
A: `serde_json::Value` is untyped and requires runtime checks. Nebula Value provides compile-time type safety with zero runtime overhead.

**Q: Is validation always required?**  
A: No! You can create values without validation. Validation is opt-in when you call `.validate()`.

**Q: Can I use this in `no_std` environments?**  
A: Not yet, but it's planned for v0.2 with an `alloc` feature.

**Q: How does performance compare to raw types?**  
A: Identical! `TypedValue<T>` is `#[repr(transparent)]` and compiles to the same assembly as raw types.

Start building type-safe applications today! 🚀