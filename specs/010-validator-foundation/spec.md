# Feature Specification: Validator Foundation Restructuring

**Feature Branch**: `010-validator-foundation`
**Created**: 2026-02-16
**Status**: Draft
**Input**: Restructure nebula-validator crate: clean dead code, modernize architecture for Rust 2026 (1.93), flatten module structure, prepare for integration with parameter and config consumers.

## Clarifications

### Session 2026-02-16

- Q: Что подразумевается под модернизацией для Rust 1.93? → A: Всё вместе: (A) переписать трейты под Rust 2024+ фичи (async fn in traits, const generics), (B) структурная очистка (rename, features, prelude), (C) убрать устаревшие паттерны (упростить Cow/GAT где возможно) + максимально плоская структура модулей.
- Q: Нужен ли deprecated alias `pub use foundation as core`? → A: Нет. Чистое переименование, сразу убираем legacy, 0 внешних потребителей.
- Q: Оставить или убрать AsyncValidate трейт (0 реализаций)? → A: Удалить полностью. Вернуть когда появится реальный use case (Phase 8).
- Q: Refined<T,V> и Parameter<T,S> type-state — feature flag или удалить? → A: Удалить полностью. Грамотно реализовать заново в Phase 7 когда появится потребность.
- Q: Стратегия плоской структуры валидаторов? → A: Убрать подпапки категорий, оставить файлы на одном уровне: `validators/length.rs`, `validators/email.rs`, `validators/range.rs` и т.д.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Parameter Developer Uses Validator via Prelude (Priority: P1)

A developer working on the `nebula-parameter` crate needs to validate a `serde_json::Value` against a rule (e.g., minimum string length) by importing a single prelude module. They should not need turbofish syntax, manual type extraction, or knowledge of the validator's internal module hierarchy.

**Why this priority**: This is the primary use case that unblocks Phase 1 (parameter integration). Without a clean, discoverable API surface, no consumer can adopt the validator crate.

**Independent Test**: Can be fully tested by importing `nebula_validator::prelude::*`, calling `min_length(5).validate_any(&json!("hello"))`, and verifying the result. Delivers value by proving the crate is consumable with a one-line import.

**Acceptance Scenarios**:

1. **Given** a developer imports `nebula_validator::prelude::*`, **When** they call `min_length(5).validate_any(&json!("hello"))`, **Then** validation succeeds with `Ok(())`.
2. **Given** a developer imports the prelude, **When** they call `min(0.0).validate_any(&json!(-1))`, **Then** validation returns a structured error with code `"min"` and params containing the minimum value.
3. **Given** a developer imports the prelude, **When** they call `json_min_size(2).validate_any(&json!([1]))`, **Then** validation fails without requiring turbofish type annotations.
4. **Given** a developer passes `Value::Null` to a string validator via `validate_any()`, **When** validation runs, **Then** it returns a `type_mismatch` error (not a panic).

---

### User Story 2 - Config Developer Validates Formats (Priority: P1)

A developer working on `nebula-config` needs format validators (email, hostname, date, time, UUID) that are more robust than the current inline checks (e.g., `contains('@')` for email). They import validators individually or via prelude and call `.validate(s)` on a `&str`.

**Why this priority**: Config currently has primitive format validators (email = `contains('@')`). Robust validators from nebula-validator eliminate duplication and improve correctness. This is a prerequisite for Phase 2 (config integration).

**Independent Test**: Can be tested by calling `email().validate("user@example.com")` and verifying pass/fail for valid and invalid inputs. Delivers value by providing production-quality format validation.

**Acceptance Scenarios**:

1. **Given** a valid RFC 5322 email address, **When** `email().validate("user@example.com")` is called, **Then** validation succeeds.
2. **Given** a string without a domain, **When** `email().validate("user@")` is called, **Then** validation fails with error code `"email"`.
3. **Given** a valid RFC 1123 hostname, **When** `hostname().validate("my-server.example.com")` is called, **Then** validation succeeds.
4. **Given** a hostname with leading hyphen in a label, **When** `hostname().validate("-invalid.com")` is called, **Then** validation fails.
5. **Given** a valid ISO 8601 date string, **When** `DateTime::date_only().validate("2026-02-16")` is called, **Then** validation succeeds and rejects strings with a time component.
6. **Given** a valid time string, **When** `time_only().validate("14:30:00")` is called, **Then** validation succeeds.

---

### User Story 3 - Validator Consumer Builds Without Unused Heavy Dependencies (Priority: P2)

A consumer of nebula-validator who does not need LRU caching or chain optimization should be able to depend on the crate with default features and not pull in the `moka` crate or compile optimizer code. Heavy optional functionality must be behind feature flags.

**Why this priority**: Reduces compile time and binary size for the common case. The caching and optimizer features have zero current consumers.

**Independent Test**: Can be tested by running `cargo check -p nebula-validator --no-default-features` and `cargo check -p nebula-validator` (default features), verifying both compile successfully and that `moka` is not linked without the `caching` feature.

**Acceptance Scenarios**:

1. **Given** default features only, **When** the crate is compiled, **Then** compilation succeeds and `moka` is not a dependency.
2. **Given** all features enabled, **When** the crate is compiled, **Then** compilation succeeds with caching and optimizer available.
3. **Given** a consumer does not enable the `caching` feature, **When** they try to use the caching combinator, **Then** they get a compile-time error (not a runtime error).

---

### User Story 4 - Internal Developer Navigates Flat, Modern Crate (Priority: P2)

An internal developer exploring the crate should find a flat, discoverable module structure without deep nesting. Validators live at `validators/length.rs`, not `validators/string/length.rs`. Dead code (Refined, type-state, AsyncValidate) is removed entirely, not hidden behind feature flags. The `core/` module is renamed to `foundation/` without a deprecated alias — a clean break.

**Why this priority**: The current deep nesting (`validators/string/length.rs`) and dead code (Refined, type-state, AsyncValidate with 0 consumers) obscure what is production-ready. The `core/` name shadows Rust's `core` crate.

**Independent Test**: Can be tested by verifying `use nebula_validator::foundation::Validate` compiles, `use nebula_validator::core::Validate` does NOT compile (no alias), validators are accessible at `validators::min_length` without subcategory paths, and Refined/AsyncValidate types are absent from the API.

**Acceptance Scenarios**:

1. **Given** the module `core/` has been renamed to `foundation/`, **When** a developer writes `use nebula_validator::foundation::Validate`, **Then** it compiles successfully.
2. **Given** no deprecated alias exists, **When** a developer writes `use nebula_validator::core::Validate`, **Then** it produces a compile error.
3. **Given** the flat validator structure, **When** a developer looks for `min_length`, **Then** they find it at `validators::min_length` (one level, no `string::` subcategory).
4. **Given** dead code removal, **When** a developer searches the public API, **Then** `Refined`, `Parameter<T,S>` type-state, and `AsyncValidate` are absent.

---

### User Story 5 - CI Pipeline Validates All Feature Combinations (Priority: P3)

The CI pipeline must verify that the crate compiles and passes tests under all supported feature combinations: no features, default features, and all features. Clippy must pass with `-D warnings`.

**Why this priority**: Feature flags introduce conditional compilation that can hide broken code paths. CI must catch these.

**Independent Test**: Can be tested by running the build and test commands across feature combinations.

**Acceptance Scenarios**:

1. **Given** the crate with default features, **When** tests are run, **Then** all tests pass.
2. **Given** the crate with all features, **When** tests are run, **Then** all tests pass including caching and optimizer tests.
3. **Given** the crate with no default features, **When** compilation is checked, **Then** compilation succeeds (JSON/serde-dependent code is gated).
4. **Given** any feature combination, **When** clippy is run with `-D warnings`, **Then** no warnings are produced.

---

### Edge Cases

- What happens when `Value::Null` is passed to any typed validator via `validate_any()`? Must return `type_mismatch` error, never panic.
- What happens when `Value::Number(3.14)` is converted to `i64` via `AsValidatable`? Must return `type_mismatch` (lossy conversion).
- What happens when very large numbers (beyond f64 precision) are validated? Must handle gracefully without panic or incorrect results.
- What happens when an empty string is passed to `hostname()` or `time_only()`? Must fail with a descriptive error.
- What happens when the `serde` feature is disabled? All JSON-dependent code (json module, AsValidatable for Value, JsonField combinator) must be conditionally compiled out.
- What happens to existing tests referencing `core/` paths? All must be updated to `foundation/` — no deprecated alias exists.
- What happens to tests referencing `Refined`, `Parameter<T,S>`, or `AsyncValidate`? They must be removed along with the dead code.

## Requirements *(mandatory)*

### Functional Requirements

**Module Structure & Cleanup:**

- **FR-001**: System MUST rename the `core/` module to `foundation/` with NO deprecated alias. All internal `use crate::core::` paths MUST be updated to `use crate::foundation::`. The old `core` path MUST NOT compile.
- **FR-002**: System MUST flatten the `validators/` directory by removing subcategory folders (`string/`, `numeric/`, `collection/`, `network/`, `logical/`). Individual validator files MUST live directly under `validators/` (e.g., `validators/length.rs`, `validators/range.rs`, `validators/ip_address.rs`).
- **FR-003**: System MUST provide a `prelude` module that re-exports all essential traits (`Validate`, `ValidateExt`, `AsValidatable`, `ValidationError`, `ValidationErrors`), all validator factory functions, and JSON convenience functions.
- **FR-004**: System MUST provide a `json` module with pre-specialized collection validator functions (`json_min_size()`, `json_max_size()`, `json_exact_size()`, `json_size_range()`) that eliminate the need for turbofish syntax when validating `serde_json::Value` arrays.
- **FR-005**: System MUST update `lib.rs` to export `foundation`, `validators`, `combinators`, `json` (behind serde), and `prelude` modules.

**Dead Code Removal:**

- **FR-006**: System MUST remove `AsyncValidate` trait and all associated code entirely (0 implementations, 0 consumers). To be re-added in Phase 8 when real use cases emerge.
- **FR-007**: System MUST remove `Refined<T,V>` type, type aliases (`NonEmptyString`, `EmailAddress`, etc.), and all associated tests entirely. To be properly reimplemented in Phase 7.
- **FR-008**: System MUST remove `Parameter<T,S>` type-state pattern (`Unvalidated`/`Validated` markers, `ParameterBuilder`) and all associated tests entirely. To be properly reimplemented in Phase 7.
- **FR-009**: System MUST remove the deprecated `Map` combinator.

**Feature Flags:**

- **FR-010**: System MUST gate the `Cached` combinator and its `moka` dependency behind a `caching` feature flag.
- **FR-011**: System MUST gate `ValidatorChainOptimizer` and `ValidatorStatistics` behind an `optimizer` feature flag.
- **FR-012**: System MUST gate all `serde_json::Value`-dependent code (json module, `AsValidatable` for Value, `JsonField` combinator) behind the `serde` feature flag (enabled by default).

**Rust 2024+ Modernization:**

- **FR-013**: System MUST leverage Rust 2024 edition features where they simplify code: `let chains` in validation logic, `async fn in traits` syntax (for future async work), improved `impl Trait` in return positions.
- **FR-014**: System MUST simplify `Cow<'static, str>` to `&'static str` in `ValidationError` fields where the value is always a static string literal (code, common messages). Keep `Cow` only where runtime-constructed strings are needed (field paths, parameterized messages).
- **FR-015**: System MUST review and simplify GAT usage in `AsValidatable` if Rust 2024 edition provides simpler alternatives for the same zero-copy conversion pattern.

**New Validators:**

- **FR-016**: System MUST provide a `Hostname` validator that validates RFC 1123 hostnames (length 1-253, labels 1-63 chars, alphanumeric + hyphens, no leading/trailing hyphens).
- **FR-017**: System MUST provide a `TimeOnly` validator that validates time strings (HH:MM:SS format, optional milliseconds, optional timezone).
- **FR-018**: System MUST provide a `DateTime::date_only()` builder method that validates date-only strings (YYYY-MM-DD) and rejects strings containing a time component.

**Quality:**

- **FR-019**: System MUST ensure all `AsValidatable` implementations for `serde_json::Value` handle edge cases: `Value::Null` returns `type_mismatch`, lossy numeric conversions return `type_mismatch`, large number overflow is handled gracefully.
- **FR-020**: System MUST compile and pass all remaining tests with `--no-default-features`, default features, and `--all-features`.
- **FR-021**: System MUST pass `cargo clippy -- -D warnings` with all feature combinations.

### Key Entities

- **Validate trait**: Core validation interface with `validate()` and `validate_any()` methods. Input type is generic.
- **AsValidatable trait**: Type conversion bridge enabling `serde_json::Value` to be validated by typed validators. Uses GAT (or simplified alternative) for zero-copy where possible.
- **ValidationError**: Structured error with code, message, field path, params, nested errors, severity, and help text. Uses `&'static str` for static fields, `Cow` only for dynamic content.
- **Prelude module**: Single-import entry point exposing all commonly-used traits, validators, and JSON helpers.
- **Feature flags**: `serde` (default), `caching` (optional, gates moka), `optimizer` (optional), `full` (all features). No `type-state` flag — that code is removed.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A consumer can validate any `serde_json::Value` with a single import (`use nebula_validator::prelude::*`) and zero turbofish annotations.
- **SC-002**: The crate compiles successfully under three feature configurations: no features, default features, and all features.
- **SC-003**: All remaining tests (after dead code removal) pass with zero regressions in validator/combinator behavior.
- **SC-004**: Default compilation (without `caching` feature) does not link the `moka` dependency.
- **SC-005**: Three new validators (Hostname, TimeOnly, DateTime::date_only) pass validation for their respective RFC/standard-compliant inputs and reject malformed inputs.
- **SC-006**: Clippy passes with `-D warnings` for all feature combinations.
- **SC-007**: The `foundation/` module is the only path to core traits. `core/` path does not compile.
- **SC-008**: The `json_min_size()` / `json_max_size()` convenience functions produce identical results to their turbofish equivalents.
- **SC-009**: Validators are accessible at one level of nesting (`validators::min_length`), not two (`validators::string::min_length`).
- **SC-010**: `Refined`, `Parameter<T,S>` type-state, `AsyncValidate`, and deprecated `Map` combinator are completely absent from the public API and source code.
