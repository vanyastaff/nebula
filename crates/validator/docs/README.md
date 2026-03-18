# nebula-validator

Composable, type-safe validation for the Nebula workflow engine.

`nebula-validator` provides two complementary approaches: **programmatic validators** that
compose at the type level via `.and()` / `.or()` / `.not()`, and **declarative `Rule` values**
that serialize to JSON and evaluate at runtime. Both approaches share the same structured
`ValidationError` type, making them interchangeable at API and storage boundaries.

---

## Table of Contents

- [Core Concepts](#core-concepts)
- [Quick Start](#quick-start)
- [Feature Matrix](#feature-matrix)
- [Crate Layout](#crate-layout)
- [Documentation](#documentation)

---

## Core Concepts

| Concept | Description |
|---------|-------------|
| **`Validate<T>`** | Core trait every validator implements. Statically typed to its input. |
| **`ValidateExt<T>`** | Blanket impl on every `Validate<T>` that adds `.and()`, `.or()`, `.not()`, `.when()`. |
| **`ValidationError`** | 80-byte structured error: machine-readable `code`, human-readable `message`, `field` path (RFC 6901), `params`, `severity`, `help`, and a `nested` tree. |
| **`ValidationErrors`** | Aggregate wrapper collecting multiple `ValidationError`s. |
| **`AnyValidator<T>`** | Type-erased validator for heterogeneous collections. Dynamic dispatch; implements `Clone`. |
| **`Validated<T>`** | Proof token certifying a value passed validation. Only constructable through a validated code path. |
| **`Rule`** | Serializable declarative rule enum covering value constraints, context predicates, and logical combinators. |
| **`ExecutionMode`** | Controls which `Rule` categories run: `StaticOnly`, `Deferred`, or `Full`. |
| **`ValidatorError`** | Crate-level operational error separating configuration bugs from validation failures. |

---

## Quick Start

### Composing validators

```rust
use nebula_validator::prelude::*;

// Chain validators with .and()
let username = min_length(3).and(max_length(20)).and(alphanumeric());
assert!(username.validate("alice").is_ok());
assert!(username.validate("ab").is_err());      // too short
assert!(username.validate("alice_123").is_err()); // underscore not alphanumeric
```

### Proof tokens

```rust
use nebula_validator::prelude::*;

// validate_into wraps the value in Validated<T> on success
let name: Validated<String> = min_length(3).validate_into("alice".to_string())?;

// Downstream functions can require Validated<String> in their signature —
// the compiler enforces that the value was checked before it arrives.
fn process(name: Validated<String>) { /* ... */ }
process(name);
```

### Struct field validation

```rust
use nebula_validator::combinators::MultiField;
use nebula_validator::validators::{min_length, in_range};

struct User { name: String, age: u32 }

let validator = MultiField::<User>::new()
    .add_field("name", min_length(2), |u: &User| u.name.as_str())
    .add_field("age",  in_range(18u32, 130), |u: &User| &u.age);

// Returns one error per failing field; all fields are always checked.
let result = validator.validate(&User { name: "Al".into(), age: 15 });
// Err: multiple_field_errors with nested name/age errors
```

### JSON validation

```rust
use nebula_validator::combinators::{json_field, json_field_optional};
use nebula_validator::validators::{min_length, min, is_true};
use serde_json::json;

let v = json_field("/server/host", min_length(1))
    .and(json_field("/server/port", min::<i64>(1)))
    .and(json_field_optional("/server/tls", is_true()));

v.validate(&json!({ "server": { "host": "localhost", "port": 8080 } }))?;
```

### Declarative rules

```rust
use nebula_validator::{Rule, ExecutionMode, validate_rules};
use serde_json::json;

let rules = vec![
    Rule::MinLength { min: 3, message: None },
    Rule::Pattern { pattern: "^[a-z]+$".into(), message: None },
];
validate_rules(&json!("alice"), &rules, ExecutionMode::StaticOnly)?;
```

---

## Feature Matrix

| Feature | Type | Notes |
|---------|------|-------|
| Type-bound composable validators | `Validate<T>` + `ValidateExt` | Zero-cost generics; no runtime dispatch by default |
| Logical combinators | `And`, `Or`, `Not`, `When`, `Unless` | Short-circuit evaluation |
| Optional and nullable | `Optional`, `Required` | `None` pass-through vs required presence |
| Collection validation | `Each`, `MinSize`, `MaxSize`, `SizeRange` | Per-element errors with index tracking |
| Struct field validation | `Field`, `MultiField` | Named field paths in dot notation |
| JSON document validation | `JsonField`, `JsonFieldOptional` | RFC 6901 JSON Pointer paths |
| Type-erased validators | `AnyValidator<T>` | Heterogeneous `Vec<AnyValidator<T>>` |
| Proof tokens | `Validated<T>` | Zero-cost; `Deserialize` intentionally omitted |
| Declarative rules | `Rule` enum | JSON-serializable; runtime evaluation |
| Batch rule evaluation | `validate_rules` + `ExecutionMode` | `StaticOnly` / `Deferred` / `Full` |
| Caching | `Cached<V>` | Memoizes by input hash; thread-safe |
| Lazy construction | `Lazy<V>` | Defers expensive init (regex, etc.) until first use |
| Custom validators | `validator!` macro | Generates struct + `Validate<T>` impl + constructor |
| Structured errors | `ValidationError` | Nested trees, RFC 6901 paths, sensitive-key redaction |
| Error aggregation | `ValidationErrors` | Collects multiple errors; `into_single_error()` |
| String validators | `length`, `pattern`, `content` modules | Unicode-aware; byte-mode variants for performance |
| Numeric range validators | `range` module | Inclusive and exclusive bounds |
| Network validators | `network` module | IPv4, IPv6, hostname — `std::net` only |
| Temporal validators | `temporal` module | Date, Time, DateTime (RFC 3339), UUID — pure Rust |
| Boolean validators | `boolean` module | `IsTrue`, `IsFalse`; const zero-cost variants |

---

## Crate Layout

```
nebula-validator/
├── src/
│   ├── lib.rs                 Re-exports, crate-level doc
│   ├── prelude.rs             Single-import convenience module
│   ├── rule.rs                Declarative Rule enum
│   ├── engine.rs              validate_rules, ExecutionMode
│   ├── error.rs               ValidatorError, ValidatorResult
│   ├── proof.rs               Validated<T> proof token
│   ├── macros.rs              validator!, compose!, any_of! macros (private)
│   ├── foundation/
│   │   ├── traits.rs          Validate<T>, ValidateExt<T>, Validatable
│   │   ├── any.rs             AnyValidator<T>
│   │   ├── context.rs         ValidationContext
│   │   ├── category.rs        ErrorCategory
│   │   ├── error.rs           ValidationError
│   │   ├── field_path.rs      FieldPath (RFC 6901 typed path)
│   │   ├── validatable.rs     SelfValidating trait
│   │   └── mod.rs
│   ├── validators/
│   │   ├── length.rs          MinLength, MaxLength, ExactLength, LengthRange, NotEmpty
│   │   ├── pattern.rs         Contains, StartsWith, EndsWith, Alphanumeric, …
│   │   ├── content.rs         MatchesRegex, Email, Url
│   │   ├── range.rs           Min, Max, InRange, GreaterThan, LessThan, ExclusiveRange
│   │   ├── size.rs            MinSize, MaxSize, ExactSize, SizeRange, NotEmptyCollection
│   │   ├── boolean.rs         IsTrue, IsFalse, IS_TRUE, IS_FALSE
│   │   ├── nullable.rs        Required, NotNull
│   │   ├── network.rs         Ipv4, Ipv6, IpAddr, Hostname
│   │   ├── temporal.rs        Date, Time, DateTime, Uuid
│   │   └── mod.rs
│   └── combinators/
│       ├── and.rs             And<L, R>
│       ├── or.rs              Or<L, R>
│       ├── not.rs             Not<V>
│       ├── when.rs            When<V, C>
│       ├── unless.rs          Unless<V, C>
│       ├── optional.rs        Optional<V>
│       ├── each.rs            Each<V>
│       ├── field.rs           Field<T, U, V, F>
│       ├── nested.rs          MultiField<T>, CollectionNested<T>
│       ├── json_field.rs      JsonField<V, I>
│       ├── factories.rs       AllOf<V>, AnyOf<V>
│       ├── message.rs         WithMessage<V>, WithCode<V>
│       ├── cached.rs          Cached<V>
│       ├── lazy.rs            Lazy<V>
│       ├── error.rs           combinator error helpers
│       └── mod.rs
└── docs/
    ├── README.md              ← this file
    ├── architecture.md        Design decisions, module map, data flow, invariants
    ├── api-reference.md       Complete public API reference
    ├── combinators.md         Combinator catalog, composition patterns, performance notes
    ├── extending.md           Writing custom validators, the validator! macro
    └── migration.md           Versioning policy, breaking changes, migration mappings
```

---

## Documentation

| Document | Contents |
|----------|----------|
| [`architecture.md`](architecture.md) | Design decisions, module map, data flow, test strategy, invariants |
| [`api-reference.md`](api-reference.md) | Every public type, trait, and method with signatures and examples |
| [`combinators.md`](combinators.md) | Full combinator catalog, composition patterns, caching, JSON field access |
| [`extending.md`](extending.md) | Writing custom validators, the `validator!` macro, the `Rule` enum |
| [`migration.md`](migration.md) | Versioning policy, error code stability, breaking change catalog |
