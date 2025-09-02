# Nebula Validator

Production-ready validation framework with advanced combinators and cross-field validation for the Nebula workflow engine.

## Overview

The Nebula Validator is a high-performance, extensible validation framework designed for the Nebula workflow engine. It provides a rich set of validation rules, logical combinators, and cross-field validation capabilities.

## Key Features

- **Comprehensive Validation Rules**: Basic, string, numeric, format, and custom validators
- **Logical Combinators**: AND, OR, NOT, XOR, and conditional validation
- **Cross-Field Validation**: Validate relationships between multiple fields
- **Performance Optimization**: Caching, complexity analysis, and resource management
- **Extensible Architecture**: Easy to add custom validators and rules
- **Async Support**: Full async/await support for I/O-bound validations
- **Rich Error Reporting**: Detailed error information with suggestions
- **Metrics & Monitoring**: Comprehensive performance metrics and observability
- **🚀 Fluent Builder API**: Type-safe, fluent interface for building validators
- **📝 Derive Macros**: Automatic validator generation from struct definitions
- **🔧 Type Safety**: Compile-time guarantees with phantom types

## Architecture

The framework is organized into several well-defined layers:

### Core Modules

- **`types`**: Core data structures, enums, and configuration types
- **`traits`**: Core validation traits, combinators, and extension points
- **`registry`**: Validator registration, discovery, and management
- **`cache`**: Result caching for performance optimization
- **`metrics`**: Performance metrics collection and monitoring

### Legacy Modules (Maintained for Backward Compatibility)

- **`validators`**: Concrete validation implementations
- **`builder`**: Fluent API for creating validators
- **`presets`**: Pre-configured validation rules
- **`pipeline`**: Complex validation workflows
- **`schema`**: JSON Schema validation support

## Quick Start

### Using the Fluent Builder API

```rust
use nebula_validator::{string, numeric, collection, Validatable};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a string validator with fluent builder
    let username_validator = string()
        .min_length(3)
        .max_length(20)
        .pattern(r"^[a-zA-Z0-9_]+$")
        .required()
        .build();
    
    // Create a numeric validator
    let age_validator = numeric()
        .min(18.0)
        .max(120.0)
        .required()
        .build();
    
    // Create a collection validator
    let tags_validator = collection()
        .min_length(1)
        .max_length(10)
        .required()
        .build();
    
    // Validate values
    let username = json!("john_doe");
    let age = json!(25);
    let tags = json!(["rust", "async"]);
    
    let results = tokio::join!(
        username_validator.validate(&username),
        age_validator.validate(&age),
        tags_validator.validate(&tags)
    );
    
    println!("Username: {:?}", results.0);
    println!("Age: {:?}", results.1);
    println!("Tags: {:?}", results.2);
    
    Ok(())
}
```

### Using Derive Macros

```rust
use nebula_validator_derive::Validate;
use nebula_validator::Validatable;

#[derive(Validate)]
struct User {
    #[validate(length(min = 3, max = 20))]
    #[validate(pattern = r"^[a-zA-Z0-9_]+$")]
    username: String,
    
    #[validate(email)]
    email: String,
    
    #[validate(range(min = 18, max = 120))]
    age: u8,
    
    #[validate(required = true)]
    password: String,
}

#[tokio::main]
async fn main() {
    let user = User {
        username: "john_doe".to_string(),
        email: "john@example.com".to_string(),
        age: 25,
        password: "secret123".to_string(),
    };
    
    // Automatic validation
    match user.validate().await {
        Ok(()) => println!("✅ User is valid!"),
        Err(errors) => {
            println!("❌ Validation failed:");
            for error in errors {
                println!("  - {}: {}", error.path.unwrap_or_default(), error.message);
            }
        }
    }
}
```

## 🚀 New Features

### Fluent Builder API

The new Fluent Builder API provides a type-safe, intuitive way to create validators:

```rust
use nebula_validator::{string, numeric, collection};

// String validation with type safety
let username_validator = string()
    .min_length(3)
    .max_length(20)
    .pattern(r"^[a-zA-Z0-9_]+$")
    .email()  // Compile-time error if used on numeric validator
    .required()
    .build();

// Numeric validation
let age_validator = numeric()
    .min(18.0)
    .max(120.0)
    .required()
    .build();

// Collection validation
let tags_validator = collection()
    .min_length(1)
    .max_length(10)
    .required()
    .build();
```

**Key Benefits:**
- **Type Safety**: Phantom types prevent invalid method calls
- **Intuitive API**: Fluent interface that reads like natural language
- **Compile-time Guarantees**: Builder state is enforced at compile time
- **Composability**: Easy to combine and reuse validators

### Derive Macros

Automatically generate validators from struct definitions:

```rust
use nebula_validator_derive::Validate;

#[derive(Validate)]
struct Product {
    #[validate(length(min = 1, max = 100))]
    name: String,
    
    #[validate(range(min = 0.01, max = 999999.99))]
    price: f64,
    
    #[validate(pattern = r"^[A-Z]{2,3}-[0-9]{6}$")]
    sku: String,
    
    #[validate(custom = "validate_product_category")]
    category: String,
}

fn validate_product_category(category: &str) -> Result<(), String> {
    let valid_categories = ["electronics", "clothing", "books"];
    if valid_categories.contains(&category) {
        Ok(())
    } else {
        Err("Invalid product category".to_string())
    }
}
```

**Supported Attributes:**
- `#[validate(length(min = X, max = Y))]` - String/collection length
- `#[validate(range(min = X, max = Y))]` - Numeric range
- `#[validate(pattern = "regex")]` - Regular expression
- `#[validate(email)]`, `#[validate(url)]`, `#[validate(uuid)]` - Format validation
- `#[validate(required = true)]` - Required field
- `#[validate(custom = "function_name")]` - Custom validation function

## Core Types

### ValidationResult

```rust
use nebula_validator::types::ValidationResult;

let result = ValidationResult::success(());
let failed_result = ValidationResult::failure(vec![error]);
```

### ValidatorMetadata

```rust
use nebula_validator::types::{ValidatorMetadata, ValidatorCategory};

let metadata = ValidatorMetadata::new(
    "email_validator",
    "Email Address Validator",
    ValidatorCategory::Format,
)
.with_description("Validates email address format")
.with_tags(vec!["email".to_string(), "format".to_string()]);
```

### ValidationConfig

```rust
use nebula_validator::types::ValidationConfig;

let config = ValidationConfig::new()
    .with_cache_ttl(600) // 10 minutes
    .with_max_depth(5)
    .with_performance_budget(500); // 500ms
```

## Core Traits

### Validatable

The main trait that all validators implement:

```rust
use nebula_validator::traits::Validatable;
use async_trait::async_trait;

#[async_trait]
impl Validatable for MyValidator {
    async fn validate(&self, value: &Value) -> ValidationResult<()> {
        // Implementation here
    }
    
    fn metadata(&self) -> ValidatorMetadata {
        // Return metadata
    }
}
```

### ValidatableExt

Extension trait providing fluent combinators:

```rust
use nebula_validator::traits::ValidatableExt;

let combined = validator1
    .and(validator2)
    .or(validator3)
    .not()
    .when(condition_validator);
```

## Registry System

The registry provides centralized validator management:

```rust
use nebula_validator::registry::{ValidatorRegistry, RegistryBuilder};

// Create registry
let registry = RegistryBuilder::new()
    .with_cache_ttl(300)
    .build();

// Register validators
registry.register_simple(
    "email",
    "Email Validator",
    ValidatorCategory::Format,
    email_validator,
).await?;

// Discover validators
let email_validators = registry.list_by_category(&ValidatorCategory::Format).await;
```

## Caching System

Built-in caching for performance optimization:

```rust
use nebula_validator::cache::{ValidationCache, CacheBuilder};

// Create cache
let cache = CacheBuilder::new()
    .with_max_entries(1000)
    .with_default_ttl(Duration::from_secs(300))
    .with_eviction_policy(EvictionPolicy::LRU)
    .build();

// Use cache
if let Some(cached_result) = cache.get(&cache_key).await {
    return cached_result;
}

// Store result
cache.set_default(cache_key, result).await?;
```

## Metrics & Monitoring

Comprehensive metrics collection:

```rust
use nebula_validator::metrics::{MetricsRegistry, MetricsBuilder};

// Create metrics registry
let metrics = MetricsBuilder::new().build();

// Record validation
metrics.record_validation(
    true,
    Duration::from_millis(50),
    &validator_id,
    ValidationComplexity::Simple,
).await;

// Get metrics
let all_metrics = metrics.all_metrics().await;
println!("Success rate: {:.2}%", all_metrics.validation.success_rate());
```

## Validation Categories

- **Basic**: Null checks, type validation, required fields
- **String**: Length, pattern matching, format validation
- **Numeric**: Range validation, integer checks, precision
- **Format**: Email, URL, UUID, IP address validation
- **Logical**: AND, OR, NOT, XOR combinations
- **Cross-Field**: Field relationship validation
- **Conditional**: Conditional validation rules
- **Collection**: Array and object validation
- **Custom**: User-defined validation logic

## Performance Features

- **Caching**: Configurable result caching with multiple eviction policies
- **Complexity Analysis**: Built-in complexity tracking for optimization
- **Async Support**: Full async/await support for I/O-bound operations
- **Resource Management**: Configurable limits and budgets
- **Metrics**: Comprehensive performance monitoring

## Error Handling

Rich error reporting with detailed context:

```rust
use nebula_validator::types::{ValidationError, ErrorCode, ErrorSeverity};

let error = ValidationError::new(
    ErrorCode::InvalidFormat,
    "Invalid email format"
)
.with_field_path("email")
.with_actual_value(json!("invalid-email"))
.with_suggestion("Use format: user@domain.com")
.with_severity(ErrorSeverity::Error);
```

## Configuration

Framework-wide configuration:

```rust
use nebula_validator::types::ValidationConfig;

let config = ValidationConfig::new()
    .with_cache_ttl(600)
    .with_max_depth(10)
    .with_performance_budget(1000)
    .with_custom_option("strict_mode", json!(true));
```

## Testing

Built-in testing support:

```rust
#[tokio::test]
async fn test_validator() {
    let validator = MyValidator::new();
    let value = json!("test");
    
    let result = validator.validate(&value).await;
    assert!(result.is_success());
}
```

## Contributing

When adding new validators or features:

1. Follow the established architecture patterns
2. Implement the appropriate traits
3. Add comprehensive tests
4. Update documentation
5. Ensure backward compatibility

## License

This project is licensed under the same license as the Nebula workflow engine.

## Dependencies

- **Runtime**: tokio (async runtime)
- **Serialization**: serde, serde_json
- **Error Handling**: thiserror, anyhow
- **Logging**: tracing
- **Collections**: dashmap
- **Time**: chrono
- **Validation**: regex, email-validator, url, uuid, base64

## Minimum Supported Rust Version

MSRV: 1.87.0
