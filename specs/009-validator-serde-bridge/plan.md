# Implementation Plan: Validator Serde Bridge

**Branch**: `009-validator-serde-bridge` | **Date**: 2026-02-11 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/009-validator-serde-bridge/spec.md`

## Summary

Add first-class `serde_json::Value` support to `nebula-validator` by extending existing modules with JSON-aware implementations. `AsValidatable` impls for `Value` are added alongside all other type conversions in `core/validatable.rs`. A `JsonField` combinator is added alongside existing combinators in `combinators/json_field.rs`. All gated behind a `serde-json` feature flag. This enables `nebula-config` to replace its ~700-line custom `SchemaValidator` with composed nebula-validator combinators.

**Technical approach**: Implement `AsValidatable<str>`, `AsValidatable<i64>`, `AsValidatable<f64>`, `AsValidatable<bool>`, and `AsValidatable<[Value]>` for `serde_json::Value` in `core/validatable.rs`. Add `JsonPath`, `PathSegment`, and `JsonField<V>` combinator in `combinators/json_field.rs`.

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)
**Primary Dependencies**: serde_json (already in nebula-validator), thiserror (errors via ValidationError)
**Storage**: N/A (pure validation, no persistence)
**Testing**: `cargo test -p nebula-validator --features serde-json`, sync tests only (no async needed)
**Target Platform**: Cross-platform (Windows primary development, Linux/macOS support)
**Project Type**: Workspace (11 crates organized in architectural layers)
**Performance Goals**: Extraction overhead < 10ns per value (single match + borrow)
**Constraints**: Zero additional allocations for type extraction; path parsing allocates once per path
**Scale/Scope**: Per-call validation of individual JSON values and fields; no batching or caching needed

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- [x] **Type Safety First**: `PathSegment` enum with `Key`/`Index` variants; `AsValidatable` returns typed `Result` preventing runtime panics; no stringly-typed APIs
- [x] **Isolated Error Handling**: Reuses nebula-validator's existing `ValidationError` type (same crate); new error codes (`type_mismatch`, `path_not_found`) use existing error structure
- [x] **Test-Driven Development**: Tests for each `AsValidatable` impl, path parsing, path traversal, `JsonField` combinator, and edge cases (null, type mismatch, missing paths)
- [x] **Async Discipline**: N/A — all validation is synchronous. No async operations, channels, or timeouts needed
- [x] **Modular Architecture**: Changes only `nebula-validator` (Domain layer). No new crate needed. No new top-level modules — code integrates into `core/` and `combinators/`. Feature-gated to avoid affecting non-JSON users
- [x] **Observability**: N/A — validation is a pure function returning errors. No logging, metrics, or tracing needed
- [x] **Simplicity**: Minimal new code (~200-300 lines across 2 files). No new dependencies. Reuses existing trait system and error types. No speculative features

## Project Structure

### Documentation (this feature)

```text
specs/009-validator-serde-bridge/
├── plan.md              # This file
├── spec.md              # Feature specification
├── research.md          # Research decisions
├── data-model.md        # Entity definitions
├── quickstart.md        # Usage examples
├── contracts/
│   └── api.md           # Public API contracts
├── checklists/
│   └── requirements.md  # Spec quality checklist
└── tasks.md             # (created by /speckit.tasks)
```

### Source Code (repository root)

```text
crates/nebula-validator/
├── src/
│   ├── lib.rs                        # UNCHANGED (no new top-level modules)
│   ├── core/
│   │   ├── validatable.rs            # MODIFIED: add AsValidatable impls for Value (cfg-gated)
│   │   └── ...                       # UNCHANGED
│   ├── combinators/
│   │   ├── mod.rs                    # MODIFIED: add json_field module (cfg-gated)
│   │   ├── field.rs                  # UNCHANGED (existing Field<T,U,V,F>)
│   │   ├── json_field.rs             # NEW: JsonField<V>, JsonPath, PathSegment
│   │   └── ...                       # UNCHANGED
│   └── validators/                   # UNCHANGED
├── tests/
│   └── json_integration.rs           # NEW: integration tests (cfg-gated)
└── Cargo.toml                        # MODIFIED: add serde-json feature flag
```

**Structure Decision**: All changes within existing `nebula-validator` modules. `AsValidatable` impls go in `core/validatable.rs` where all other type conversions live. `JsonField` combinator goes in `combinators/json_field.rs` alongside `field.rs`, `each.rs`, `nested.rs`. No new top-level modules — JSON support is woven into the existing architecture as a first-class citizen.

## Design Decisions

### D1: AsValidatable as the Integration Mechanism

The `AsValidatable<T>` trait is the existing extension point for type conversion in nebula-validator. By implementing it for `serde_json::Value` in the same file where all other impls live, all existing validators automatically work with JSON values via `validate_any()` — zero modifications to validators needed.

**Key insight**: `validate_any` already has the bound `S: AsValidatable<Self::Input>`, so adding `impl AsValidatable<str> for Value` means `MinLength.validate_any(&json!("hello"))` just works.

**Placement**: `core/validatable.rs` — next to `impl AsValidatable<str> for String`, `impl AsValidatable<i64> for i32`, etc. This is where type conversions belong.

### D2: Strict Type Matching (No Coercion)

Per clarification: `Value::String("42")` passed to a numeric validator produces a type mismatch error. Consumers pre-convert if needed. This keeps the implementation simple and predictable.

### D3: JsonField as a Native Combinator

The existing `Field<T, U, V, F>` combinator uses `F: Fn(&T) -> &U` — a compile-time accessor function. For JSON paths, we need runtime traversal. `JsonField<V>` is a new combinator in `combinators/json_field.rs`, living alongside `Field`, `Each`, `Nested`:

1. Parses path at construction time (fail-fast on invalid syntax)
2. Traverses JSON tree at validation time
3. Extracts and delegates to inner validator via `validate_any`

`JsonPath` and `PathSegment` are defined in the same file as supporting types.

### D4: Bracket Notation for Array Indices

Per clarification: `"items[0].name"` syntax. Parser handles:
- `"key"` → `Key("key")`
- `"key[0]"` → `Key("key"), Index(0)`
- `"a.b[2].c"` → `Key("a"), Key("b"), Index(2), Key("c")`

### D5: Feature Flag Gating

All JSON code behind `#[cfg(feature = "serde-json")]`. The `serde_json` crate dependency is already present but the impls are opt-in. Code is placed inline in existing files with `cfg` attributes, not in separate modules.

## Implementation Phases

### Phase 1: AsValidatable for Value (P1 — foundational)

**Files**: `core/validatable.rs`, `Cargo.toml`

1. Add `serde-json` feature flag to `Cargo.toml`
2. Add `#[cfg(feature = "serde-json")]` section in `core/validatable.rs`
3. Implement `AsValidatable<str>` for `Value` — extract string or type mismatch
4. Implement `AsValidatable<i64>` for `Value` — extract integer or type mismatch
5. Implement `AsValidatable<f64>` for `Value` — extract float or type mismatch
6. Implement `AsValidatable<bool>` for `Value` — extract bool or type mismatch
7. Implement `AsValidatable<[Value]>` for `Value` — extract array or type mismatch
8. Tests in the same file: happy path + type mismatch + null handling for each impl

**Validates**: FR-001, FR-002, FR-003, FR-006, FR-007, FR-008, FR-010, SC-001, SC-004, SC-005

### Phase 2: JsonField Combinator with Path Traversal (P2 — field validation)

**Files**: `combinators/json_field.rs`, `combinators/mod.rs`

1. Define `PathSegment` enum (`Key(String)`, `Index(usize)`)
2. Define `JsonPath` struct with parsing (`"a.b[0].c"` → segments)
3. Implement `JsonPath::resolve()` — JSON tree traversal returning `Result<&Value, ValidationError>`
4. Implement `Display` for `JsonPath`
5. Define `JsonField<V>` struct with `path: JsonPath`, `validator: V`, `required: bool`
6. Implement `Validate for JsonField<V>` with `type Input = serde_json::Value`
7. Add helper functions `json_field()` and `json_field_optional()`
8. Register module in `combinators/mod.rs` behind `#[cfg(feature = "serde-json")]`
9. Tests: path parsing, traversal, required/optional fields, missing paths, combinator composition

**Validates**: FR-004, FR-005, FR-008, SC-002

### Phase 3: Integration Tests and Documentation

**Files**: `tests/json_integration.rs`, doc comments in modified files

1. Integration tests covering full validation scenarios with composed validators
2. Test combinator composition: `json_field(...).and(json_field(...))`
3. Test with real-world config-like structures
4. Rustdoc examples on all public items (`AsValidatable` impls, `JsonField`, `JsonPath`)
5. Run quality gates: `cargo fmt`, `cargo clippy`, `cargo doc`

**Validates**: SC-004, SC-005

## Complexity Tracking

No constitution violations. No complexity justification needed.
