# Tasks: Validator Serde Bridge

**Feature**: 009-validator-serde-bridge
**Created**: 2026-02-11
**Updated**: 2026-02-12

## Implementation Notes

The approved implementation plan diverges from the original spec in two ways (both improvements):

1. **RFC 6901 JSON Pointer** instead of custom dot-notation path parser — standard format, zero parsing code, uses `serde_json::Value::pointer()` directly
2. **Feature merged into `serde`** instead of separate `serde-json` flag — `serde_json` is already a non-optional dependency and the existing `serde` feature already uses it for `Refined` serialization and `ValidationError::to_json_value()`

## Phase 1: Collection Validator Consistency (Breaking Change)

- [X] **T1.1**: Change `MinSize<T>` from `type Input = Vec<T>` to `type Input = [T]`, remove `T: Clone` bound — `size.rs`
- [X] **T1.2**: Change `MaxSize<T>` from `type Input = Vec<T>` to `type Input = [T]`, remove `T: Clone` bound — `size.rs`
- [X] **T1.3**: Change `ExactSize<T>` from `type Input = Vec<T>` to `type Input = [T]`, remove `T: Clone` bound — `size.rs`
- [X] **T1.4**: Change `NotEmptyCollection<T>` from `type Input = Vec<T>` to `type Input = [T]`, remove `T: Clone` bound — `size.rs`
- [X] **T1.5**: Change `SizeRange<T>` from `type Input = Vec<T>` to `type Input = [T]`, remove `T: Clone` bound — `size.rs`
- [X] **T1.6**: Change `Unique<T>` from `type Input = Vec<T>` to `type Input = [T]` — `elements.rs`
- [X] **T1.7**: Verify all existing tests pass via `&Vec<T>` → `&[T]` deref coercion

## Phase 2: AsValidatable Impls for serde_json::Value

- [X] **T2.1**: Add `json_type_name()` helper function returning `&'static str` for Value variants — `validatable.rs`
- [X] **T2.2**: Implement `AsValidatable<str> for Value` — extract `Value::String` or type_mismatch — `validatable.rs`
- [X] **T2.3**: Implement `AsValidatable<i64> for Value` — extract `Value::Number.as_i64()` or type_mismatch — `validatable.rs`
- [X] **T2.4**: Implement `AsValidatable<f64> for Value` — extract `Value::Number.as_f64()` or type_mismatch — `validatable.rs`
- [X] **T2.5**: Implement `AsValidatable<bool> for Value` — extract `Value::Bool` or type_mismatch — `validatable.rs`
- [X] **T2.6**: Implement `AsValidatable<[Value]> for Value` — extract `Value::Array` slice or type_mismatch — `validatable.rs`
- [X] **T2.7**: Add 14 unit tests for all impls (happy path + type mismatch + null handling) — `validatable.rs`
- [X] **T2.8**: All impls gated behind `#[cfg(feature = "serde")]`

## Phase 3: JsonField Combinator (RFC 6901 JSON Pointer)

- [X] **T3.1**: Define `JsonField<V>` struct with `pointer: Cow<'static, str>`, `inner: V`, `required: bool` — `json_field.rs`
- [X] **T3.2**: Implement `Validate for JsonField<V>` using `Value::pointer()` for path traversal — `json_field.rs`
- [X] **T3.3**: Add `json_field()` helper (required field) — `json_field.rs`
- [X] **T3.4**: Add `json_field_optional()` helper (optional field) — `json_field.rs`
- [X] **T3.5**: Register module in `combinators/mod.rs` with re-exports and prelude — `mod.rs`
- [X] **T3.6**: Add 11 unit tests (required/optional, nested paths, array index, composition, type mismatch, root pointer) — `json_field.rs`

## Phase 4: Integration Tests

- [X] **T4.1**: Test direct value validation (`validate_any` with JSON string) — `json_integration.rs`
- [X] **T4.2**: Test config structure validation (nested JSON fields) — `json_integration.rs`
- [X] **T4.3**: Test array element access by index — `json_integration.rs`
- [X] **T4.4**: Test optional field handling (missing field passes) — `json_integration.rs`
- [X] **T4.5**: Test type mismatch error messages — `json_integration.rs`
- [X] **T4.6**: Test null value type mismatch — `json_integration.rs`
- [X] **T4.7**: Test `MinSize` with JSON array — `json_integration.rs`
- [X] **T4.8**: Test composed `json_field` validators with `.and()` — `json_integration.rs`

## Phase 5: Quality Gates

- [X] **T5.1**: `cargo fmt --all -- --check` passes
- [X] **T5.2**: `cargo clippy -p nebula-validator -- -D warnings` passes
- [X] **T5.3**: `cargo doc -p nebula-validator --no-deps` builds
- [X] **T5.4**: `cargo test -p nebula-validator` — all 449 tests pass (with default `serde` feature)
- [X] **T5.5**: Feature gate works — 424 tests without `serde` feature, 449 with

## Phase 6: Commit

- [X] **T6.1**: Commit all changes with appropriate message
