# 🚀 Enhanced Nebula Validator

This document describes the major enhancements and new features added to `nebula-validator` based on the design documents v2_1.md, v2_2.md, and v2.md.

## ✨ New Features Overview

### 1. **Core Valid/Invalid System**
- **Rich validation results** with proof tracking and transformation history
- **ValidationProof** with TTL, signatures, and composite proofs
- **Error recovery** with automatic fixing capabilities
- **Type-safe** `Validated<T>` enum for better error handling

### 2. **Enhanced Conditional Validators**
- **WhenChain** - Switch/case style conditional validation
- **Field conditions** - Rich field predicates (exists, equals, matches, etc.)
- **Combined conditions** - All, Any, None, Not logical combinations
- **Enhanced Required/Optional** - Conditional field requirements

### 3. **Advanced Logical Validators**
- **WeightedOr** - OR validation with weights and priorities
- **ParallelAnd** - Parallel execution with concurrency control
- **XOR** - Exactly N validators must pass
- **EnhancedAll/EnhancedAny** - Performance-optimized logical operations

### 4. **Performance Optimizations**
- **Memoized** - Result caching with TTL
- **Throttled** - Rate limiting for external validators
- **Lazy** - On-demand validator creation
- **Deferred** - Runtime validator configuration

### 5. **Rule Composition System**
- **RuleComposer** - Dependency-aware rule execution
- **Topological sorting** - Automatic execution order determination
- **Rule chains** - Sequential rule execution
- **Rule groups** - Logical rule organization

## 🔧 Usage Examples

### Complex Conditional Validation

```rust
use nebula_validator::prelude::*;

let registration_validator = RuleComposer::new()
    // Basic validation
    .rule("username", 
        Required::new()
            .and(StringLength::new(3, 20))
            .and(Pattern::new(r"^[a-zA-Z0-9_]+$"))
    )
    
    // Conditional validation based on account type
    .rule("account_type_validation",
        WhenChain::new()
            .when(
                field("account_type").equals(json!("business")),
                EnhancedAll::new()
                    .add(Required::new().for_field("company_name"))
                    .add(Required::new().for_field("tax_id"))
            )
            .when(
                field("account_type").equals(json!("personal")),
                EnhancedAll::new()
                    .add(Required::new().for_field("first_name"))
                    .add(Required::new().for_field("last_name"))
            )
            .otherwise(AlwaysInvalid::new("Invalid account type"))
    )
    
    // XOR validation
    .rule("contact_verification",
        Xor::new()
            .add(field("email_verified").equals(json!(true)))
            .add(field("phone").exists().and(PhoneNumber::valid()))
            .expect(XorExpectation::ExactlyOne)
    );

// Execute validation
let result = registration_validator.validate(&form_data).await?;
```

### Performance-Optimized Validation

```rust
let performance_validator = EnhancedAll::new()
    .parallel()
    .max_concurrency(4)
    .fail_fast()
    .add(Memoized::new(Email::new(), Duration::from_secs(300)))
    .add(Throttled::new(Url::new(), 100))
    .add(Lazy::new(|| Box::new(PhoneNumber::new())))
    .add(Deferred::new());

// Set deferred validator later
performance_validator.set(AlwaysValid::new()).await;
```

### Rule Composition with Dependencies

```rust
let composer = RuleComposer::new()
    .rule("basic_info", 
        EnhancedAll::new()
            .add(Required::new().for_field("name"))
            .add(Required::new().for_field("age"))
    )
    .dependent_rule("premium_access",
        Predicate::new(
            "premium_check",
            |v| v["subscription"].as_str().map_or(false, |s| s == "premium"),
            "Premium subscription required"
        ),
        vec!["age_verification".to_string()]
    );

// Automatic dependency resolution and execution
let result = composer.validate(&data).await?;
```

### Valid/Invalid System with Proofs

```rust
let validator = Predicate::new(
    "positive_number",
    |v| v.as_f64().map_or(false, |n| n > 0.0),
    "Number must be positive"
);

let result = validator.validate(&json!(42.0)).await?;

match result.into_validated() {
    Validated::Valid(valid) => {
        println!("Value: {:?}", valid.value());
        println!("Proof: {:?}", valid.proof());
        println!("Expired: {}", valid.is_expired());
    },
    Validated::Invalid(invalid) => {
        // Try to fix the error
        let fixed = invalid.try_fix(|_value, _errors| async {
            Ok(json!(0.0))
        }).await;
    }
}
```

## 🏗️ Architecture

### Core Types

```rust
// Main validation result type
pub enum Validated<T> {
    Valid(Valid<T>),
    Invalid(Invalid<T>),
}

// Valid value with metadata
pub struct Valid<T> {
    value: T,
    proof: ValidationProof,
    transformations: Vec<TransformationRecord>,
}

// Invalid value with errors
pub struct Invalid<T> {
    value: Option<T>,
    errors: Vec<ValidationError>,
    metadata: InvalidMetadata,
}

// Validation proof
pub struct ValidationProof {
    pub validator_id: ValidatorId,
    pub validated_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub context: HashMap<String, Value>,
    pub signature: Option<String>,
    pub proof_type: ProofType,
}
```

### Enhanced Validators

```rust
// Conditional validation
pub struct When<C, T, F> { ... }
pub struct WhenChain { ... }
pub struct FieldCondition { ... }

// Logical validation
pub struct WeightedOr { ... }
pub struct ParallelAnd { ... }
pub struct Xor { ... }

// Performance validators
pub struct Memoized<V> { ... }
pub struct Throttled<V> { ... }
pub struct Lazy<F> { ... }
pub struct Deferred { ... }

// Rule composition
pub struct RuleComposer { ... }
pub struct RuleChain { ... }
pub struct RuleGroup { ... }
```

## 🚀 Performance Features

### 1. **Parallel Execution**
- **Concurrency control** with semaphore-based limiting
- **Fail-fast mode** for early termination
- **Error collection** strategies (first error vs. all errors)

### 2. **Caching & Memoization**
- **Result caching** with configurable TTL
- **Hash-based** cache keys for efficient lookups
- **Automatic cleanup** of expired entries

### 3. **Rate Limiting**
- **Sliding window** rate limiting
- **Configurable** requests per second
- **Graceful degradation** on limit exceeded

### 4. **Lazy Loading**
- **On-demand** validator creation
- **Factory pattern** for dynamic validators
- **Memory optimization** for rarely-used validators

## 🔒 Error Handling & Recovery

### 1. **Rich Error Information**
- **Error codes** with severity levels
- **Context preservation** across transformations
- **Error chaining** for complex validation scenarios

### 2. **Automatic Recovery**
- **Error fixing** with custom logic
- **Fallback strategies** for failed validations
- **Graceful degradation** on partial failures

### 3. **Validation Proofs**
- **Audit trail** for validation decisions
- **TTL support** for time-sensitive validations
- **Signature support** for secure validations

## 📊 Monitoring & Observability

### 1. **Performance Metrics**
- **Execution time** tracking
- **Cache hit rates** for memoized validators
- **Concurrency levels** for parallel execution

### 2. **Validation Statistics**
- **Success/failure rates** by validator type
- **Error distribution** by error code
- **Dependency graph** execution statistics

### 3. **Health Checks**
- **Validator availability** monitoring
- **Cache health** status
- **Rate limiter** performance metrics

## 🔧 Configuration

### 1. **Validator Configuration**
```rust
let validator = WeightedOr::new()
    .add(validator1, 1.0)
    .add(validator2, 0.8)
    .min_weight(1.5)
    .no_short_circuit();
```

### 2. **Performance Configuration**
```rust
let validator = ParallelAnd::new()
    .max_concurrency(8)
    .fail_fast()
    .collect_all_errors();
```

### 3. **Caching Configuration**
```rust
let validator = Memoized::new(base_validator, Duration::from_secs(300))
    .with_max_entries(1000);
```

## 🧪 Testing

### 1. **Unit Testing**
```rust
#[tokio::test]
async fn test_weighted_or_validation() {
    let validator = WeightedOr::new()
        .add(AlwaysValid::new(), 1.0)
        .min_weight(1.0);
    
    let result = validator.validate(&json!(42)).await?;
    assert!(result.is_success());
}
```

### 2. **Integration Testing**
```rust
#[tokio::test]
async fn test_rule_composition() {
    let composer = RuleComposer::new()
        .rule("test", AlwaysValid::new());
    
    let result = composer.validate(&test_data).await?;
    assert!(result.is_success());
}
```

## 📈 Migration Guide

### From v1.x to Enhanced Version

1. **Update imports** to use new enhanced validators
2. **Replace basic combinators** with enhanced versions
3. **Add performance optimizations** where appropriate
4. **Use new Valid/Invalid system** for better error handling

### Example Migration

```rust
// Old way
let validator = And::new(validator1, validator2);

// New way
let validator = EnhancedAll::new()
    .add(validator1)
    .add(validator2)
    .parallel()
    .max_concurrency(4);
```

## 🎯 Best Practices

### 1. **Performance**
- Use **parallel execution** for independent validators
- Enable **memoization** for expensive validations
- Set appropriate **concurrency limits** based on system resources

### 2. **Error Handling**
- Implement **custom error recovery** logic
- Use **validation proofs** for audit trails
- Set appropriate **TTL values** for validation results

### 3. **Rule Composition**
- Design **clear dependency graphs** for complex rules
- Use **rule groups** for logical organization
- Implement **fallback strategies** for critical validations

## 🔮 Future Enhancements

### 1. **Planned Features**
- **Machine learning** integration for adaptive validation
- **Distributed validation** across multiple nodes
- **Real-time validation** with streaming support

### 2. **Performance Improvements**
- **SIMD optimizations** for numeric validations
- **GPU acceleration** for complex pattern matching
- **Smart caching** with predictive invalidation

### 3. **Integration Features**
- **GraphQL** validation support
- **OpenAPI** schema validation
- **Database constraint** validation

## 📚 Additional Resources

- [API Documentation](https://docs.rs/nebula-validator)
- [Examples Directory](./examples/)
- [Performance Benchmarks](./benches/)
- [Contributing Guidelines](./CONTRIBUTING.md)

---

**Note**: This enhanced version maintains full backward compatibility while adding powerful new features. Existing code will continue to work without modification.
