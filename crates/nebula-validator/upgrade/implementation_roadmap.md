# 🚀 nebula-validator v2.0 - Implementation Roadmap

## 📋 Обзор

Полная переписка `nebula-validator` с фокусом на:
- **Type safety** через refined types и type-state pattern
- **Zero-cost abstractions** через generics и compile-time оптимизации  
- **Composability** через traits и комбинаторы
- **Performance** через кэширование и ленивые вычисления

---

## 🏗️ Структура проекта

```
crates/nebula-validator/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs                 # Public API
│   ├── core/
│   │   ├── mod.rs
│   │   ├── traits.rs          # TypedValidator, AsyncValidator
│   │   ├── refined.rs         # Refined<T, V> types
│   │   ├── state.rs           # Type-state pattern
│   │   ├── error.rs           # ValidationError
│   │   └── metadata.rs        # ValidatorMetadata
│   ├── combinators/
│   │   ├── mod.rs
│   │   ├── and.rs             # And<L, R>
│   │   ├── or.rs              # Or<L, R>
│   │   ├── not.rs             # Not<V>
│   │   ├── map.rs             # Map<V, F>
│   │   ├── when.rs            # When<V, C>
│   │   ├── optional.rs        # Optional<V>
│   │   └── cached.rs          # Cached<V>
│   ├── validators/
│   │   ├── mod.rs
│   │   ├── string/
│   │   │   ├── mod.rs
│   │   │   ├── length.rs      # MinLength, MaxLength, ExactLength
│   │   │   ├── pattern.rs     # MatchesRegex, Email, Url
│   │   │   └── content.rs     # Contains, StartsWith, EndsWith
│   │   ├── numeric/
│   │   │   ├── mod.rs
│   │   │   ├── range.rs       # InRange, Min, Max
│   │   │   ├── comparison.rs  # Equal, NotEqual, GreaterThan
│   │   │   └── properties.rs  # Even, Odd, Positive, Negative
│   │   ├── collection/
│   │   │   ├── mod.rs
│   │   │   ├── size.rs        # MinSize, MaxSize, ExactSize
│   │   │   ├── elements.rs    # All, Any, Contains, Unique
│   │   │   └── structure.rs   # HasKey, HasAllKeys, OnlyKeys
│   │   ├── logical/
│   │   │   ├── mod.rs
│   │   │   ├── boolean.rs     # IsTrue, IsFalse
│   │   │   └── nullable.rs    # NotNull, Required, Optional
│   │   └── custom/
│   │       ├── mod.rs
│   │       ├── async_val.rs   # AsyncValidator implementations
│   │       └── user.rs        # User-defined validators
│   ├── macros/
│   │   ├── mod.rs
│   │   └── derive.rs          # Derive macros
│   ├── prelude.rs             # Common imports
│   └── bridge/
│       ├── mod.rs
│       └── value.rs           # Bridge to nebula-value (legacy support)
├── tests/
│   ├── integration_tests.rs
│   ├── property_tests.rs      # Property-based tests
│   └── benchmarks.rs
└── examples/
    ├── basic.rs
    ├── composition.rs
    ├── refined_types.rs
    ├── type_state.rs
    └── async_validation.rs
```

---

## 📅 Phase 1: Core Foundation (Week 1-2)

### Priority: 🔴 CRITICAL

#### 1.1 Core Traits (Days 1-3)

**Files to create:**
- `src/core/traits.rs`
- `src/core/error.rs`
- `src/core/metadata.rs`

**Tasks:**
```rust
// core/traits.rs
- [ ] Define TypedValidator trait
- [ ] Define AsyncValidator trait  
- [ ] Define ValidatorExt trait
- [ ] Add marker traits (Send, Sync bounds)

// core/error.rs
- [ ] ValidationError struct with structured fields
- [ ] Error builder pattern
- [ ] Display/Debug implementations
- [ ] Error code constants

// core/metadata.rs
- [ ] ValidatorMetadata struct
- [ ] ValidationComplexity enum
- [ ] Introspection utilities
```

**Tests:**
```rust
#[test]
fn test_validator_trait_object_safety();
#[test]
fn test_error_chain_construction();
#[test]
fn test_metadata_introspection();
```

#### 1.2 Refined Types (Days 4-5)

**Files to create:**
- `src/core/refined.rs`

**Tasks:**
```rust
- [ ] Refined<T, V> struct
- [ ] new() with validation
- [ ] new_unchecked() unsafe constructor
- [ ] into_inner() / get() accessors
- [ ] Implement std traits (Debug, Clone, etc.)
```

**Tests:**
```rust
#[test]
fn test_refined_creation_valid();
#[test]
fn test_refined_creation_invalid();
#[test]
fn test_refined_type_safety();
#[test]
fn test_refined_deref_coercion();
```

#### 1.3 Type-State Pattern (Days 6-7)

**Files to create:**
- `src/core/state.rs`

**Tasks:**
```rust
- [ ] Unvalidated state marker
- [ ] Validated<V> state marker
- [ ] Parameter<T, S> struct
- [ ] State transition methods
```

**Tests:**
```rust
#[test]
fn test_unvalidated_to_validated_transition();
#[test]
fn test_validated_unwrap_safety();
#[test]
fn test_state_compile_time_guarantees(); // compile_fail tests
```

#### 1.4 Documentation & Examples (Days 8-10)

```rust
- [ ] Write comprehensive rustdoc
- [ ] Create examples/basic.rs
- [ ] Update README.md
- [ ] Architecture decision records (ADR)
```

**Milestone:** ✅ Core traits ready, refined types working, type-state implemented

---

## 📅 Phase 2: Combinators (Week 3)

### Priority: 🔴 CRITICAL

#### 2.1 Basic Combinators (Days 1-3)

**Files to create:**
- `src/combinators/and.rs`
- `src/combinators/or.rs`
- `src/combinators/not.rs`

**Tasks:**
```rust
// Each combinator needs:
- [ ] Struct definition
- [ ] TypedValidator impl
- [ ] AsyncValidator impl (if applicable)
- [ ] Builder methods in ValidatorExt
- [ ] Tests for laws (associativity, commutativity, etc.)
```

**Tests:**
```rust
#[test]
fn test_and_both_pass();
#[test]
fn test_and_first_fails();
#[test]
fn test_and_associativity();
#[test]
fn test_or_short_circuit();
#[test]
fn test_not_inversion();
```

#### 2.2 Advanced Combinators (Days 4-5)

**Files to create:**
- `src/combinators/map.rs`
- `src/combinators/when.rs`
- `src/combinators/optional.rs`

**Tasks:**
```rust
- [ ] Map<V, F> for output transformation
- [ ] When<V, C> for conditional validation
- [ ] Optional<V> for nullable values
```

#### 2.3 Performance Combinators (Days 6-7)

**Files to create:**
- `src/combinators/cached.rs`

**Tasks:**
```rust
- [ ] Cached<V> with RwLock
- [ ] Cache key generation (hash-based)
- [ ] Cache invalidation strategy
- [ ] Memory-bounded cache (LRU)
```

**Tests:**
```rust
#[test]
fn test_cache_hit();
#[test]
fn test_cache_miss();
#[test]
fn test_cache_concurrency();
```

**Milestone:** ✅ All combinators working, laws verified, examples created

---

## 📅 Phase 3: String Validators (Week 4)

### Priority: 🔴 CRITICAL

#### 3.1 Length Validators (Days 1-2)

**Files to create:**
- `src/validators/string/length.rs`

**Tasks:**
```rust
- [ ] MinLength validator
- [ ] MaxLength validator  
- [ ] ExactLength validator
- [ ] InLengthRange validator
- [ ] Helper functions (min_length, max_length, etc.)
```

#### 3.2 Pattern Validators (Days 3-4)

**Files to create:**
- `src/validators/string/pattern.rs`

**Tasks:**
```rust
- [ ] MatchesRegex validator
- [ ] Email validator (regex + DNS check option)
- [ ] Url validator
- [ ] PhoneNumber validator (with country codes)
- [ ] Uuid validator
```

#### 3.3 Content Validators (Days 5-7)

**Files to create:**
- `src/validators/string/content.rs`

**Tasks:**
```rust
- [ ] Contains validator
- [ ] StartsWith validator
- [ ] EndsWith validator
- [ ] Alphanumeric validator
- [ ] NoWhitespace validator
- [ ] Custom character set validator
```

**Milestone:** ✅ Complete string validation suite

---

## 📅 Phase 4: Numeric Validators (Week 5)

### Priority: 🟡 HIGH

#### 4.1 Range Validators (Days 1-2)

**Files to create:**
- `src/validators/numeric/range.rs`

**Tasks:**
```rust
- [ ] InRange<T> validator (generic over numbers)
- [ ] Min<T> validator
- [ ] Max<T> validator
- [ ] Support for i8..i128, u8..u128, f32, f64
```

#### 4.2 Comparison Validators (Days 3-4)

**Files to create:**
- `src/validators/numeric/comparison.rs`

**Tasks:**
```rust
- [ ] Equal<T> validator
- [ ] NotEqual<T> validator
- [ ] GreaterThan<T> validator
- [ ] GreaterThanOrEqual<T> validator
- [ ] LessThan<T> validator
- [ ] LessThanOrEqual<T> validator
```

#### 4.3 Property Validators (Days 5-7)

**Files to create:**
- `src/validators/numeric/properties.rs`

**Tasks:**
```rust
- [ ] Even validator
- [ ] Odd validator
- [ ] Positive validator
- [ ] Negative validator
- [ ] DivisibleBy validator
- [ ] IsPrime validator (optional, expensive)
```

**Milestone:** ✅ Complete numeric validation suite

---

## 📅 Phase 5: Collection Validators (Week 6)

### Priority: 🟡 HIGH

#### 5.1 Size Validators (Days 1-2)

**Files to create:**
- `src/validators/collection/size.rs`

**Tasks:**
```rust
- [ ] MinSize validator
- [ ] MaxSize validator
- [ ] ExactSize validator (with const generic option)
- [ ] Generic over Vec, HashMap, HashSet, etc.
```

#### 5.2 Element Validators (Days 3-5)

**Files to create:**
- `src/validators/collection/elements.rs`

**Tasks:**
```rust
- [ ] All<V> validator (all elements pass V)
- [ ] Any<V> validator (at least one element passes V)
- [ ] Contains<T> validator
- [ ] Unique validator (no duplicates)
- [ ] Sorted validator (for ordered collections)
```

#### 5.3 Structure Validators (Days 6-7)

**Files to create:**
- `src/validators/collection/structure.rs`

**Tasks:**
```rust
- [ ] HasKey<K> validator (for maps)
- [ ] HasAllKeys<K> validator
- [ ] OnlyKeys<K> validator
- [ ] Schema validator (for nested objects)
```

**Milestone:** ✅ Complete collection validation suite

---

## 📅 Phase 6: Derive Macros (Week 7-8)

### Priority: 🟢 MEDIUM

#### 6.1 Validator Derive (Days 1-5)

**Files to create:**
- `nebula-validator-derive/` (new crate)
- `src/macros/derive.rs`

**Tasks:**
```rust
- [ ] #[derive(Validator)] macro
- [ ] Attribute macros (#[validate(min_length = 5)])
- [ ] Struct field validation
- [ ] Nested validator composition
```

**Example:**
```rust
#[derive(Validator)]
struct UserInput {
    #[validate(min_length = 3, max_length = 20, alphanumeric)]
    username: String,
    
    #[validate(email)]
    email: String,
    
    #[validate(min = 18, max = 100)]
    age: u8,
}
```

#### 6.2 Refined Derive (Days 6-10)

**Tasks:**
```rust
- [ ] #[derive(Refined)] macro
- [ ] Automatic validation in From/TryFrom
- [ ] Serde integration (#[serde(try_from = "...")]
```

**Milestone:** ✅ Derive macros working, examples created

---

## 📅 Phase 7: Advanced Features (Week 9-10)

### Priority: 🟢 LOW

#### 7.1 Async Validators (Days 1-3)

**Files to create:**
- `src/validators/custom/async_val.rs`

**Tasks:**
```rust
- [ ] AsyncValidator implementations
- [ ] Database lookup validators
- [ ] API call validators
- [ ] Timeout handling
- [ ] Retry logic
```

#### 7.2 Registry System (Days 4-6)

**Files to create:**
- `src/registry/mod.rs`

**Tasks:**
```rust
- [ ] ValidatorRegistry struct
- [ ] Dynamic validator lookup by name
- [ ] Serialization/deserialization of validators
- [ ] Plugin system integration
```

#### 7.3 Context System (Days 7-10)

**Files to create:**
- `src/context/mod.rs`

**Tasks:**
```rust
- [ ] ValidationContext struct
- [ ] Cross-field validation support
- [ ] Parent/child relationships
- [ ] Context propagation in combinators
```

**Milestone:** ✅ Advanced features implemented

---

## 📅 Phase 8: Testing & Polish (Week 11-12)

### Priority: 🔴 CRITICAL

#### 8.1 Comprehensive Testing (Days 1-5)

**Tasks:**
```rust
- [ ] Unit tests for all validators (target: 100% coverage)
- [ ] Integration tests
- [ ] Property-based tests (using proptest)
- [ ] Compile-fail tests for type safety
```

#### 8.2 Benchmarks (Days 6-8)

**Files to create:**
- `benches/validators.rs`

**Tasks:**
```rust
- [ ] Benchmark suite using criterion
- [ ] Compare with v1 implementation
- [ ] Optimize hot paths
- [ ] Memory usage profiling
```

#### 8.3 Documentation (Days 9-10)

**Tasks:**
```rust
- [ ] Complete rustdoc for all public APIs
- [ ] Tutorial-style documentation
- [ ] Migration guide from v1
- [ ] Best practices guide
```

#### 8.4 Examples (Days 11-14)

**Tasks:**
```rust
- [ ] 10+ comprehensive examples
- [ ] Real-world use cases
- [ ] Integration with nebula-parameter
- [ ] Performance examples
```

**Milestone:** ✅ Production-ready release

---

## 🔄 Migration Strategy

### Backwards Compatibility

```toml
[features]
default = ["v2-api"]
v1-api = []      # Keep old API for compatibility
v2-api = []      # New API
full = ["v1-api", "v2-api"]
```

### Bridge Module

```rust
// src/bridge/value.rs
// Wrap v2 validators to work with nebula-value::Value

pub struct ValueValidator<V> {
    inner: V,
}

impl<V> Validator for ValueValidator<V>
where
    V: TypedValidator<Input = str>,
{
    async fn validate(&self, value: &Value, ctx: Option<&ValidationContext>) 
        -> Result<Valid<()>, Invalid<()>> 
    {
        if let Value::Text(s) = value {
            self.inner.validate(s)
                .map(|_| Valid::new(()))
                .map_err(|e| Invalid::simple(e.to_string()))
        } else {
            Err(Invalid::simple("Expected string"))
        }
    }
}
```

---

## 📊 Success Metrics

### Code Quality
- [ ] 100% test coverage for core
- [ ] 90%+ coverage for validators
- [ ] Zero clippy warnings
- [ ] Passes miri tests
- [ ] No unsafe code (except well-documented)

### Performance
- [ ] 10x faster than v1 for simple validators
- [ ] No allocations for most validators
- [ ] < 1ms for complex validator chains
- [ ] Efficient memory usage (< 1MB for typical usage)

### API Quality
- [ ] Compile-time type safety where possible
- [ ] Ergonomic builder APIs
- [ ] Good error messages
- [ ] Comprehensive documentation

---

## 🎯 Next Steps

1. **Review this roadmap** - есть ли что-то что нужно изменить?
2. **Prioritize** - хотите ли изменить приоритеты фаз?
3. **Start implementation** - начнем с Phase 1?
4. **Set up tooling** - CI/CD, benchmarks, etc.

**Что делаем первым?** 🚀
