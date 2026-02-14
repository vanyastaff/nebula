# Feature Specification: Validator Serde Bridge

**Feature Branch**: `009-validator-serde-bridge`  
**Created**: 2026-02-11  
**Status**: Draft  
**Input**: User description: "Add serde_json::Value bridge to nebula-validator (AsValidatable implementations or a separate adapter that extracts typed data from Value), so that nebula-config can reuse all existing validators instead of its 700+ lines in schema.rs."

## Clarifications

### Session 2026-02-11

- Q: Должен ли мост поддерживать приведение типов (type coercion), например строка "42" → число для числовых валидаторов? → A: Нет — только строгое соответствие типов. nebula-config сам конвертирует значения перед валидацией при необходимости.
- Q: Должны ли пути полей поддерживать индексацию массивов, и если да — какой синтаксис? → A: Да, скобочная нотация: `"items[0].name"`.
- Q: Должен ли мост предоставлять валидаторы структуры объекта (required keys, additionalProperties)? → A: Нет — структурная валидация собирается потребителем из существующих комбинаторов (`Nested`, `Field`, `Each` и т.д.), которые мост делает совместимыми с `serde_json::Value`.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Validate JSON Values with Existing Validators (Priority: P1)

As a developer using nebula-validator, I want to validate `serde_json::Value` instances using the existing validator library (string validators, numeric validators, collection validators, etc.) without manually extracting and converting types first.

**Why this priority**: This is the foundational capability — without it, no downstream consumer (including nebula-config) can leverage nebula-validator for dynamic JSON data. Every other story depends on this bridge existing.

**Independent Test**: Can be fully tested by constructing `serde_json::Value` instances of various types (string, number, bool, array, object, null) and passing them through existing validators like `MinLength`, `MaxValue`, `NotEmpty`, etc. — verifying that validation succeeds for valid values and produces correct errors for invalid ones.

**Acceptance Scenarios**:

1. **Given** a `serde_json::Value::String("hello")`, **When** validated with `MinLength { min: 3 }`, **Then** validation passes.
2. **Given** a `serde_json::Value::String("hi")`, **When** validated with `MinLength { min: 3 }`, **Then** validation fails with an appropriate error message.
3. **Given** a `serde_json::Value::Number(42)`, **When** validated with `MaxValue { max: 100 }`, **Then** validation passes.
4. **Given** a `serde_json::Value::Bool(true)`, **When** validated with a boolean validator, **Then** validation passes.
5. **Given** a `serde_json::Value::Null`, **When** validated with a string validator like `MinLength`, **Then** validation fails with a type mismatch error.
6. **Given** a `serde_json::Value::Array([1, 2, 3])`, **When** validated with `MinSize { min: 2 }`, **Then** validation passes.

---

### User Story 2 - Validate Nested JSON Object Fields (Priority: P2)

As a developer, I want to validate individual fields within a JSON object by path, composing validators for nested structures without writing manual traversal logic.

**Why this priority**: Real-world configuration and workflow data is deeply nested. Field-level validation on JSON objects unlocks the primary use case of replacing nebula-config's schema.rs.

**Independent Test**: Can be tested by constructing nested JSON objects and applying field-path validators that reach into nested keys, verifying correct validation of deeply nested values.

**Acceptance Scenarios**:

1. **Given** a JSON object `{"server": {"port": 8080}}`, **When** validated with a field validator targeting path `"server.port"` with `InRange { min: 1, max: 65535 }`, **Then** validation passes.
2. **Given** a JSON object `{"server": {"port": 99999}}`, **When** validated with the same field-path validator, **Then** validation fails with an error referencing the field path.
3. **Given** a JSON object missing a required field, **When** validated with a required-field validator, **Then** validation fails with an appropriate "field missing" error.

---

### User Story 3 - nebula-config Reuses nebula-validator Instead of Custom schema.rs (Priority: P3)

As a maintainer of nebula-config, I want to replace the ~700-line custom `SchemaValidator` with validators composed from nebula-validator, reducing code duplication and maintenance burden.

**Why this priority**: This is the downstream consumption story — the motivating use case. It depends on Stories 1 and 2 being complete, but delivers the highest long-term value by eliminating duplicated validation logic.

**Independent Test**: Can be tested by migrating nebula-config's existing validation test suite to use nebula-validator-based validators, verifying identical behavior with significantly less code.

**Acceptance Scenarios**:

1. **Given** a configuration value that was previously validated by `SchemaValidator`, **When** the same value is validated using nebula-validator bridge validators, **Then** the validation result (pass/fail and error messages) is equivalent.
2. **Given** the nebula-config crate after migration, **When** measuring lines of validation code, **Then** the schema validation logic is reduced by at least 50%.
3. **Given** all existing nebula-config tests, **When** run after migration, **Then** all tests continue to pass.

---

### Edge Cases

- What happens when a `serde_json::Value::Number` is validated with a string validator? Should return a clear type mismatch error.
- What happens when a JSON number exceeds `i64` range (e.g., large `u64` or `f64`)? The bridge should handle both integer and floating-point numbers correctly.
- What happens when validating `serde_json::Value::Null` — should it be treated as "absent" (for optional/nullable validators) or as a type mismatch?
- What happens when a nested field path references a non-existent intermediate key (e.g., `"a.b.c"` but `"b"` doesn't exist)? Should produce a clear "path not found" error.
- What happens when array elements have mixed types and are validated with element-level validators?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide a way to validate `serde_json::Value` instances using existing nebula-validator validators (string, numeric, collection, boolean categories).
- **FR-002**: The system MUST return a clear type mismatch error when a JSON value type does not match the expected validator input type (e.g., validating a number with a string validator).
- **FR-003**: The system MUST support numeric validation for both integer and floating-point JSON numbers.
- **FR-004**: The system MUST support validating individual fields within JSON objects by field name or dot-separated path, including array index access via bracket notation (e.g., `"servers[0].port"`).
- **FR-005**: The system MUST produce validation errors that include the field path context (e.g., `"server.port"`) so users can identify which part of the data is invalid.
- **FR-006**: The system MUST handle `serde_json::Value::Null` as a distinct case, failing type-specific validators with an appropriate error rather than panicking.
- **FR-007**: The system MUST support validating elements within JSON arrays using existing collection validators.
- **FR-008**: The system MUST be composable with existing nebula-validator combinators (`And`, `Or`, `Not`, `Optional`, `When`, etc.).
- **FR-009**: The system MUST allow nebula-config to depend on nebula-validator and replace its custom schema validation logic with bridge-based validators.
- **FR-010**: The system MUST preserve backward compatibility — existing nebula-validator users must not be affected by the addition of JSON value support.

### Key Entities

- **Value Bridge / Adapter**: The mechanism (trait implementations, adapter type, or module) that connects `serde_json::Value` to nebula-validator's type-safe validation system.
- **Field Path**: A string using dot-separated keys and bracket notation for array indices (e.g., `"server.host"`, `"servers[0].port"`) identifying a location within a nested JSON structure for targeted validation.
- **Type Mismatch Error**: A specific validation error produced when a JSON value's runtime type does not match the validator's expected input type.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: All existing nebula-validator string, numeric, collection, and boolean validators can validate corresponding `serde_json::Value` variants without any modifications to the validators themselves.
- **SC-002**: Nested JSON object fields can be validated by path with errors that include the full path context.
- **SC-003**: nebula-config's schema validation code is reduced by at least 50% (from ~700 lines) after adopting the bridge.
- **SC-004**: All existing nebula-validator and nebula-config tests continue to pass after the bridge is introduced.
- **SC-005**: The bridge introduces no breaking changes to nebula-validator's public API for existing users.

## Assumptions

- `serde_json::Value::Number` will be validated as `f64` for floating-point validators and as `i64` for integer validators, following serde_json's own numeric representation.
- The bridge will be gated behind an optional feature flag (e.g., `serde-json`) in nebula-validator to avoid adding a hard dependency for users who don't need it.
- Null handling follows the convention: `Null` fails type-specific validators but passes `Optional`/`Nullable` combinators.
- The nebula-config migration (User Story 3) is a separate follow-up effort that depends on Stories 1 and 2 being complete and merged first.
- No type coercion: the bridge enforces strict type matching. A `Value::String("42")` passed to a numeric validator produces a type mismatch error. Consumers (e.g., nebula-config) are responsible for pre-converting values if coercion is needed.

## Scope Boundaries

**In scope:**
- `serde_json::Value` to nebula-validator bridge (trait implementations and/or adapter types)
- Field-path extraction from JSON objects for targeted validation
- Type mismatch error handling for JSON value / validator mismatches
- Feature-flag gating of the bridge

**Out of scope:**
- Full JSON Schema specification compliance (this is a validator bridge, not a JSON Schema engine)
- Dedicated structural validators (required keys, additionalProperties, minProperties) — consumers compose these from existing combinators (`Nested`, `Field`, `Each`)
- Deserialization or schema-to-Rust-struct mapping
- Validation of non-JSON formats (TOML, YAML, etc.)
- The actual nebula-config migration — that is a downstream consumer task
