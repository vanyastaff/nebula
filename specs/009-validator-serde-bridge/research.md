# Research: Validator Serde Bridge

**Feature**: 009-validator-serde-bridge
**Date**: 2026-02-11

## R1: Bridge Strategy — AsValidatable vs Adapter Type

### Decision
Use `AsValidatable<T> for serde_json::Value` trait implementations as the primary bridge mechanism, gated behind a `serde-json` feature flag.

### Rationale
- `AsValidatable` is the existing extension point designed for exactly this purpose — converting one type to another for validation
- The `validate_any` method already dispatches through `AsValidatable`, so implementing it for `Value` means all existing validators work with zero modifications
- An adapter/wrapper type (e.g., `JsonValue(&Value)`) would require validators to accept the wrapper, which breaks composability with existing combinators
- `serde_json` is already a dependency of nebula-validator (not optional currently)

### Alternatives Considered
1. **Wrapper type `JsonValue`**: Would require new `Validate` impls for every validator. Rejected — too much code, breaks composability.
2. **New trait `ValidateJson`**: Separate trait for JSON validation. Rejected — duplicates the trait hierarchy, doesn't compose with existing combinators.
3. **Blanket impl via serde `Deserialize`**: Convert `Value` to concrete types first. Rejected — loses structural information, doesn't support partial validation.

## R2: Numeric Value Extraction

### Decision
- `Value::Number` → `i64` via `Number::as_i64()` for integer validators
- `Value::Number` → `f64` via `Number::as_f64()` for float validators
- Return `ValidationError` with code `type_mismatch` if extraction fails (e.g., f64 value passed to i64 validator)

### Rationale
- `serde_json::Number` internally stores integers as i64/u64 and floats as f64
- `as_i64()` returns `None` for floats and u64 values > i64::MAX, which maps naturally to type mismatch errors
- `as_f64()` can represent all integers (with precision loss above 2^53), which is acceptable for validation purposes

### Alternatives Considered
1. **Always use f64**: Would lose integer precision for large values. Rejected.
2. **Support u64 separately**: Would require a third numeric AsValidatable impl. Deferred — can add later if needed.

## R3: Collection/Array Bridge

### Decision
- `Value::Array` → `[Value]` slice for collection validators (`MinSize`, `MaxSize`, `NotEmpty`, etc.)
- Element-level validation uses `Each` combinator with Value-aware inner validators
- Array size validators operate on `Vec<Value>` length directly

### Rationale
- Collection validators use `type Input = [T]` where T is the element type
- `Vec<Value>` naturally implements `AsValidatable<[Value]>` via the existing `Vec<T> → [T]` impl
- For heterogeneous arrays, each element is a `Value` that can be independently validated

### Alternatives Considered
1. **Convert arrays to typed vectors first**: Would fail on mixed-type arrays. Rejected.
2. **Special array validator**: Unnecessary — existing collection combinators work once `Value` implements `AsValidatable`.

## R4: Field Path Parsing and Traversal

### Decision
- Support dot notation for object keys: `"server.port"`
- Support bracket notation for array indices: `"items[0].name"`
- Path parsing produces a `Vec<PathSegment>` where segments are either `Key(String)` or `Index(usize)`
- Traversal returns `Result<&Value, ValidationError>` with path context on failure

### Rationale
- Bracket notation was explicitly chosen in clarification session
- serde_json's built-in `Value::pointer()` uses JSON Pointer syntax (`/server/port`) which doesn't support the bracket notation the user wants
- Custom path parser is simple (~30 lines) and more ergonomic for config use cases

### Alternatives Considered
1. **JSON Pointer syntax (`/server/0/port`)**: Standard but less familiar for config users. Rejected by clarification.
2. **Only dot notation with numeric keys for arrays (`items.0.name`)**: Simpler parser but less explicit. Rejected by clarification.

## R5: Feature Flag Strategy

### Decision
- Gate all `serde_json::Value` bridge code behind `serde-json` feature flag
- `serde_json` is already a non-optional dependency, but the bridge impls should be opt-in
- Consider making `serde_json` itself optional in a future cleanup (out of scope)

### Rationale
- Feature flags prevent compile-time overhead and code bloat for users who don't need JSON validation
- The existing `serde` feature flag only gates serialization of error types, not structural usage
- Convention: feature name `serde-json` matches the crate name

### Alternatives Considered
1. **Always enabled**: Simpler but adds compile time for all users. Rejected.
2. **Separate crate `nebula-validator-json`**: Too much overhead for ~200 lines of bridge code. Rejected.

## R6: Module Organization

### Decision
Integrate JSON support into existing modules — no new top-level modules:
- `core/validatable.rs` — `AsValidatable` impls for `Value` (behind `#[cfg(feature = "serde-json")]`), alongside all other type conversion impls
- `combinators/json_field.rs` — `JsonField<V>` combinator + `JsonPath` + `PathSegment` (behind `#[cfg(feature = "serde-json")]`), alongside `field.rs`, `each.rs`, `nested.rs`

### Rationale
- `AsValidatable` impls belong where all other `AsValidatable` impls live — `core/validatable.rs`
- `JsonField` is a combinator, so it belongs in `combinators/` alongside `Field`, `Each`, `Nested`
- `#[cfg]` attributes on individual items/sections within existing files are idiomatic Rust for feature-gating
- No separate `bridge/` module — JSON support is a first-class part of the validator, not an external bolt-on

### Alternatives Considered
1. **Separate `bridge/` top-level module**: Creates an isolated silo that feels like a bolt-on rather than native integration. Rejected.
2. **Separate crate `nebula-validator-json`**: Too much overhead for ~200 lines of code. Rejected.
3. **Add to `validators/` as a new validator category**: `JsonField` is a combinator, not a validator. Rejected.
