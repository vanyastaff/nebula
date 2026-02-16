# Implementation Plan: Validator Foundation Restructuring

**Branch**: `010-validator-foundation` | **Date**: 2026-02-16 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `specs/010-validator-foundation/spec.md`

## Summary

Restructure `nebula-validator` (Domain layer) for production readiness: rename `core/` to `foundation/`, flatten validators into one-level directory, remove dead code (~650 LOC: AsyncValidate, Refined, type-state, Map combinator), add feature flags (caching, optimizer), create prelude and JSON convenience modules, add 3 new validators (Hostname, TimeOnly, DateTime::date_only). Research confirms Cow<'static, str> and GAT patterns are already optimal — no trait-level changes needed. See [research.md](research.md).

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92, targeting 1.93 compatibility)
**Primary Dependencies**: serde, serde_json, regex, url, uuid, base64, moka (optional)
**Storage**: N/A (pure validation library, no I/O)
**Testing**: `cargo test -p nebula-validator`, `cargo test -p nebula-validator --all-features`
**Target Platform**: Cross-platform (Windows primary development, Linux/macOS support)
**Project Type**: Workspace (11 crates, Domain layer)
**Performance Goals**: Zero-cost abstractions — validators compile to inline code via monomorphization. No performance regression from restructuring.
**Constraints**: All existing validator/combinator tests must pass (behavior-preserving refactor). ~650 LOC dead code removed.
**Scale/Scope**: Single crate, ~20K LOC source, ~500 tests, 31 validator files, 16 combinator files.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- [x] **Type Safety First**: Validators use generic `Input` types, `AsValidatable` GAT ensures compile-time type-safe conversions. `Validate` trait enforces typed input. No `String`-as-code-smell.
- [x] **Isolated Error Handling**: `ValidationError` is crate-local, uses `Cow<'static, str>` (not thiserror — it's a data struct, not an enum). `ValidationErrors` wraps collection. No cross-crate error dependencies.
- [x] **Test-Driven Development**: All new validators (Hostname, TimeOnly, date_only) require tests before implementation. Existing 500+ tests serve as regression suite. Feature-gated code tested under `--all-features`.
- [x] **Async Discipline**: N/A — pure synchronous validation library. AsyncValidate trait is being removed (0 implementations). No Tokio dependency.
- [x] **Modular Architecture**: `nebula-validator` is Domain layer. No new cross-crate dependencies. Future consumers (parameter, config) depend downward. Feature flags isolate optional components.
- [x] **Observability**: N/A — library crate, no runtime state. Validators return structured errors with codes and params for consumer logging.
- [x] **Simplicity**: Dead code removed (~650 LOC). Flat module structure replaces 3-level nesting. Feature flags hide unused components. No premature abstractions (Refined, type-state deleted per YAGNI).

## Project Structure

### Documentation (this feature)

```text
specs/010-validator-foundation/
├── plan.md              # This file
├── spec.md              # Feature specification
├── research.md          # Phase 0 research findings
├── data-model.md        # Target data model and module structure
├── quickstart.md        # Consumer usage guide
├── contracts/
│   └── public-api.rs    # Target public API contract
└── checklists/
    └── requirements.md  # Spec quality checklist
```

### Source Code (repository root)

```text
crates/validator/
├── Cargo.toml                    # Updated: feature flags, moka optional
├── src/
│   ├── lib.rs                    # Updated: foundation, validators, combinators, json, prelude
│   ├── prelude.rs                # NEW: single-import for all consumers
│   ├── json.rs                   # NEW: json_min_size(), json_max_size(), etc.
│   ├── foundation/               # RENAMED from core/
│   │   ├── mod.rs                # Updated: no refined, state, AsyncValidate
│   │   ├── traits.rs             # Updated: no AsyncValidate, Map removed from ValidateExt
│   │   ├── error.rs              # Unchanged
│   │   ├── context.rs            # Unchanged
│   │   ├── metadata.rs           # Updated: ValidatorStatistics behind optimizer feature
│   │   ├── category.rs           # Unchanged
│   │   └── validatable.rs        # Unchanged (serde-gated Value impls already correct)
│   ├── validators/               # FLATTENED: no subcategory folders
│   │   ├── mod.rs                # Updated: flat re-exports
│   │   ├── length.rs             # Moved from string/
│   │   ├── pattern.rs            # Moved from string/
│   │   ├── content.rs            # Moved from string/
│   │   ├── uuid.rs               # Moved from string/
│   │   ├── datetime.rs           # Moved from string/ + date_only()
│   │   ├── time.rs               # NEW: TimeOnly
│   │   ├── json_string.rs        # Moved from string/json.rs (renamed)
│   │   ├── password.rs           # Moved from string/
│   │   ├── phone.rs              # Moved from string/
│   │   ├── credit_card.rs        # Moved from string/
│   │   ├── iban.rs               # Moved from string/
│   │   ├── semver.rs             # Moved from string/
│   │   ├── slug.rs               # Moved from string/
│   │   ├── hex.rs                # Moved from string/
│   │   ├── base64.rs             # Moved from string/
│   │   ├── range.rs              # Moved from numeric/
│   │   ├── properties.rs         # Moved from numeric/
│   │   ├── divisibility.rs       # Moved from numeric/
│   │   ├── float.rs              # Moved from numeric/
│   │   ├── percentage.rs         # Moved from numeric/
│   │   ├── size.rs               # Moved from collection/
│   │   ├── elements.rs           # Moved from collection/
│   │   ├── structure.rs          # Moved from collection/
│   │   ├── ip_address.rs         # Moved from network/
│   │   ├── hostname.rs           # NEW: RFC 1123
│   │   ├── port.rs               # Moved from network/
│   │   ├── mac_address.rs        # Moved from network/
│   │   ├── boolean.rs            # Moved from logical/
│   │   └── nullable.rs           # Moved from logical/
│   ├── combinators/              # Unchanged structure
│   │   ├── mod.rs                # Updated: no map re-exports
│   │   ├── and.rs, or.rs, not.rs
│   │   ├── optional.rs, when.rs, unless.rs
│   │   ├── each.rs, lazy.rs
│   │   ├── cached.rs             # Feature-gated: #[cfg(feature = "caching")]
│   │   ├── field.rs, json_field.rs, nested.rs, message.rs
│   │   ├── error.rs
│   │   └── optimizer.rs          # Feature-gated: #[cfg(feature = "optimizer")]
│   └── macros/
│       └── mod.rs                # Unchanged
├── tests/
│   ├── integration_test.rs       # Updated: core:: → foundation::
│   ├── combinator_error_test.rs  # Updated: core:: → foundation::
│   ├── json_integration.rs       # Updated: core:: → foundation::
│   ├── validation_context_test.rs # Updated: core:: → foundation::
│   └── optimizer_test.rs         # Feature-gated: #[cfg(feature = "optimizer")]
│   # DELETED: refined_test.rs
├── benches/                      # Updated: core:: → foundation::
└── examples/                     # Updated: core:: → foundation::
```

**Structure Decision**: Only `nebula-validator` crate is modified. No new crates. Domain layer positioning unchanged. Feature flags reduce mandatory dependencies (moka → optional).

## Implementation Phases

### Phase A: Dead Code Removal & Core Rename

**Goal**: Remove all dead code, rename `core/` → `foundation/`. Crate compiles and passes tests.

**Steps**:

1. **Delete dead code files**:
   - Delete `src/core/refined.rs` (Refined<T,V>, type aliases)
   - Delete `src/core/state.rs` (Parameter<T,S>, Unvalidated, Validated, ParameterBuilder)
   - Delete `src/combinators/map.rs` (deprecated Map combinator)
   - Delete `tests/refined_test.rs`

2. **Remove dead code references**:
   - `src/core/mod.rs`: Remove `pub mod refined`, `pub mod state`, re-exports of Refined/Parameter/AsyncValidate
   - `src/core/traits.rs`: Remove `AsyncValidate` trait definition entirely
   - `src/combinators/mod.rs`: Remove `pub mod map`, all Map/map/map_to/map_unit re-exports
   - `src/core/traits.rs` (ValidateExt): Remove `.map()` method

3. **Rename `core/` → `foundation/`**:
   - `git mv src/core src/foundation`
   - `src/lib.rs`: `pub mod core` → `pub mod foundation` (NO deprecated alias)
   - Global find/replace in ALL source files: `crate::core::` → `crate::foundation::`
   - Global find/replace in tests: `nebula_validator::core::` → `nebula_validator::foundation::`
   - Update benchmarks and examples similarly

4. **Verify**: `cargo test -p nebula-validator` — all remaining tests pass, `cargo clippy` clean.

### Phase B: Flatten Validators

**Goal**: Move all validator files from subcategory folders to flat `validators/` directory.

**Steps**:

1. **Move files** (using git mv for history preservation):
   - Move all 14 files from `validators/string/` to `validators/` (rename `json.rs` → `json_string.rs`)
   - Move all 5 files from `validators/numeric/` to `validators/`
   - Move all 3 files from `validators/collection/` to `validators/`
   - Move all 3 files from `validators/network/` to `validators/`
   - Move all 2 files from `validators/logical/` to `validators/`

2. **Delete empty subcategory directories** and their `mod.rs` files:
   - Delete `validators/string/`, `validators/numeric/`, `validators/collection/`, `validators/network/`, `validators/logical/`

3. **Rewrite `validators/mod.rs`**: Flat module declarations and re-exports. All items previously accessible via `validators::string::min_length` now at `validators::min_length`.

4. **Update internal imports**: Any file that used `use crate::validators::string::*` → `use crate::validators::*` (or specific items).

5. **Verify**: `cargo test -p nebula-validator` — all tests pass, no import errors.

### Phase C: Feature Flags

**Goal**: Gate optional components behind feature flags. moka becomes optional dependency.

**Steps**:

1. **Update `Cargo.toml`**:
   ```toml
   [features]
   default = ["serde"]
   serde = []
   caching = ["dep:moka"]
   optimizer = []
   full = ["serde", "caching", "optimizer"]

   [dependencies]
   moka = { version = "0.12", features = ["sync"], optional = true }
   ```

2. **Gate caching** (`#[cfg(feature = "caching")]`):
   - `src/combinators/cached.rs`: Wrap entire module
   - `src/combinators/mod.rs`: Conditional `pub mod cached` and re-exports
   - `src/foundation/traits.rs` (ValidateExt): Gate `.cached()` and `.cached_with_capacity()` methods

3. **Gate optimizer** (`#[cfg(feature = "optimizer")]`):
   - `src/combinators/optimizer.rs`: Wrap entire module
   - `src/combinators/mod.rs`: Conditional `pub mod optimizer` and re-exports
   - `src/foundation/metadata.rs`: Gate `ValidatorStatistics` and `RegisteredValidatorMetadata`
   - `tests/optimizer_test.rs`: Add `#[cfg(feature = "optimizer")]` to entire file

4. **Verify serde gating**: Ensure `AsValidatable<_> for Value` impls in `validatable.rs` and `JsonField` in `combinators/json_field.rs` are already behind `#[cfg(feature = "serde")]`. If not, add gates.

5. **Verify all 4 feature combinations**:
   - `cargo check -p nebula-validator --no-default-features`
   - `cargo check -p nebula-validator` (default = serde)
   - `cargo check -p nebula-validator --features caching`
   - `cargo check -p nebula-validator --all-features`

### Phase D: New Modules (prelude, json)

**Goal**: Create the prelude and json convenience modules.

**Steps**:

1. **Create `src/json.rs`**:
   - `#[cfg(feature = "serde")]` at module level
   - Type aliases: `JsonMinSize`, `JsonMaxSize`, `JsonExactSize`, `JsonSizeRange`
   - Factory functions: `json_min_size()`, `json_max_size()`, `json_exact_size()`, `json_size_range()`

2. **Create `src/prelude.rs`**:
   - Re-export all traits from `foundation`
   - Re-export all validator factory functions from `validators`
   - Re-export combinators
   - Conditionally re-export `json::*` behind `serde` feature

3. **Update `src/lib.rs`**: Add `pub mod prelude` and `#[cfg(feature = "serde")] pub mod json`

4. **Write tests**: Verify prelude import works end-to-end with `validate_any()` on `serde_json::Value`.

5. **Verify**: `cargo test -p nebula-validator --all-features`

### Phase E: New Validators

**Goal**: Implement Hostname, TimeOnly, DateTime::date_only().

**Steps** (TDD for each):

1. **Hostname validator** (`src/validators/hostname.rs`):
   - Write tests first: valid hostnames, invalid (leading hyphen, too long, empty, dots)
   - Implement RFC 1123 rules (see research.md R7)
   - Add factory function `hostname()` and re-export in `validators/mod.rs` and `prelude.rs`

2. **TimeOnly validator** (`src/validators/time.rs`):
   - Write tests first: valid times, invalid (25:00:00, bad format, empty)
   - Implement parsing (see research.md R8)
   - Add builder `.require_timezone()` and factory `time_only()`
   - Re-export in `validators/mod.rs` and `prelude.rs`

3. **DateTime::date_only()** (modify `src/validators/datetime.rs`):
   - Write tests first: "2026-02-16" passes, "2026-02-16T10:00:00" fails
   - Add `date_only_mode` flag and builder method
   - Re-export in prelude

4. **Verify**: All new and existing tests pass.

### Phase F: Quality Gates & Final Verification

**Goal**: All CI checks pass across all feature combinations.

**Steps**:

1. Run full quality gate suite:
   ```bash
   cargo fmt --all -- --check
   cargo clippy -p nebula-validator -- -D warnings
   cargo clippy -p nebula-validator --all-features -- -D warnings
   cargo check -p nebula-validator --no-default-features
   cargo check -p nebula-validator --all-features
   cargo test -p nebula-validator
   cargo test -p nebula-validator --all-features
   cargo doc --no-deps -p nebula-validator
   ```

2. Verify success criteria SC-001 through SC-010 (see spec.md).

3. Update examples to use `foundation::` paths and `prelude::*`.

4. Ensure `cargo test --workspace` passes (no impact on other crates).

## Execution Order & Dependencies

```
Phase A (Dead Code & Rename)
    ↓
Phase B (Flatten Validators)
    ↓
Phase C (Feature Flags)
    ↓
Phase D (Prelude & JSON modules)
    ↓
Phase E (New Validators)
    ↓
Phase F (Quality Gates)
```

Each phase produces a compilable, testable crate. Phases are sequential — each builds on the previous.

## Risk Mitigation

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Internal import breakage during rename | Medium | Run `cargo check` after each file move. Use `git mv` for history. |
| Feature flag gating misses a `use moka` | Low | `--no-default-features` compilation catches ungated moka usage. |
| Flattening creates name collisions | Low | All 27 validator files have unique names (verified in research). |
| Tests referencing deleted types | Medium | Grep for `Refined`, `Parameter`, `AsyncValidate`, `Map` in all test files. |
| Existing consumers break | None | 0 external consumers. All code is internal workspace. |
