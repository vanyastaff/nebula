# Feature Specification: Migrate to serde_json Value System

**Feature Branch**: `008-serde-value-migration`
**Created**: 2026-02-11
**Status**: Draft
**Input**: User description: "Migrate from custom nebula-value to serde_json::Value and RawValue. Replace persistent data structures (im::Vector, im::HashMap) with standard collections. Use temporal types as strings + chrono. Use serde ecosystem for special types (Bytes, Decimal). Bottom-up migration: nebula-config → nebula-resilience → nebula-expression. Hide RawValue optimizations in framework internals for deferred deserialization and pass-through scenarios."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Direct Ecosystem Integration (Priority: P1)

Developers working with workflow data can use standard `serde_json::Value` directly without conversion overhead when integrating with third-party libraries and database drivers that already use serde_json.

**Why this priority**: This is the primary motivation for the migration - eliminating conversion overhead between nebula-value and serde_json when working with the broader Rust ecosystem (database drivers, HTTP libraries, etc.).

**Independent Test**: Can be fully tested by integrating any serde-based library (e.g., sqlx, reqwest) and verifying zero conversion code is needed between workflow data and library APIs.

**Acceptance Scenarios**:

1. **Given** a workflow node needs to call a database using sqlx, **When** passing workflow data as parameters, **Then** the data is used directly without conversion from nebula-value to serde_json::Value
2. **Given** a workflow receives JSON from an HTTP API, **When** the data enters the workflow system, **Then** it remains as serde_json::Value without intermediate conversions
3. **Given** workflow data contains temporal values (dates, times), **When** stored in a database, **Then** chrono types serialize/deserialize directly via serde without custom conversion logic

---

### User Story 2 - Simple Node Developer API (Priority: P2)

Workflow node developers work with a clean `Value`-based API without needing to understand RawValue performance optimizations or when to parse vs pass-through data.

**Why this priority**: Developer experience is critical for the workflow platform, but this can be built on top of P1's foundation.

**Independent Test**: A developer can implement a new workflow node using only the `Node` trait with `execute(&self, input: &Value) -> Result<Value>` signature, and the framework handles all RawValue optimizations transparently.

**Acceptance Scenarios**:

1. **Given** a developer implements a new workflow node, **When** they write the execute method using `&Value`, **Then** they never see or handle RawValue types
2. **Given** a pass-through node (like Filter or Switch), **When** data flows through without inspection, **Then** the framework automatically avoids parsing (uses RawValue internally) without developer intervention
3. **Given** a node needs to inspect specific fields, **When** accessing `value["fieldName"]`, **Then** parsing happens lazily only for accessed data

---

### User Story 3 - Zero Regression Migration (Priority: P3)

All existing workflow functionality continues working after migration with no behavioral changes, performance regressions, or breaking API changes for end users.

**Why this priority**: This ensures the migration is safe, but validating it depends on completing P1 first.

**Independent Test**: Run the complete existing test suite (cargo test --workspace) and verify 100% pass rate with zero modifications to test expectations.

**Acceptance Scenarios**:

1. **Given** the migration is complete for nebula-config, **When** running all config-related tests, **Then** all tests pass without modification
2. **Given** the migration is complete for nebula-expression, **When** evaluating expressions with temporal data, **Then** results match pre-migration behavior exactly
3. **Given** a workflow using complex nested data structures, **When** executing after migration, **Then** output is identical to pre-migration output

---

### Edge Cases

- What happens when RawValue contains invalid JSON during lazy parsing?
  - Framework catches the error during first access and returns a clear error to the node
- How does the system handle very large JSON values (e.g., 100MB payloads)?
  - RawValue defers parsing so large pass-through data never consumes parse time, but nodes accessing the data will need to handle large Value structures (existing behavior)
- What happens when temporal string formats are invalid?
  - chrono parsing errors are caught and wrapped in domain-specific errors (e.g., ExpressionError::InvalidDate)
- How are circular references handled in serde_json?
  - serde_json doesn't support circular references (same as nebula-value), so this remains a constraint

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST replace all uses of `nebula_value::Value` with `serde_json::Value` in nebula-config, nebula-resilience, and nebula-expression crates
- **FR-002**: System MUST use `serde_json::value::RawValue` for deferred deserialization at workflow node boundaries
- **FR-003**: System MUST store temporal types (Date, Time, DateTime, Duration) as ISO 8601/RFC 3339 strings and use `chrono` for parsing when temporal operations are needed
- **FR-004**: System MUST use serde ecosystem types for special data: `bytes::Bytes` for binary, `rust_decimal::Decimal` for precise numbers
- **FR-005**: System MUST hide RawValue complexity from node developers by providing a clean `execute(&self, input: &Value) -> Result<Value>` API
- **FR-006**: System MUST migrate crates bottom-up in this order: nebula-config → nebula-resilience → nebula-expression
- **FR-007**: Each crate MUST define its own error types using `thiserror` and include `#[from] serde_json::Error` where JSON operations occur
- **FR-008**: System MUST remove all dependencies on the `im` crate (persistent data structures) and use standard `Vec` and `HashMap` from serde_json
- **FR-009**: System MUST remove the `nebula-value` crate entirely after all dependent crates are migrated
- **FR-010**: All existing tests MUST pass without modification after migration (except for nebula-value's own tests)

### Key Entities

- **Value**: Represents any workflow data - scalars, objects, arrays. Implemented by `serde_json::Value` (Null, Bool, Number, String, Array, Object)
- **RawValue**: Represents unparsed JSON data at workflow boundaries. Used internally by framework for zero-copy pass-through optimization
- **Temporal Value**: Date/Time data stored as ISO 8601 strings in Value, parsed to `chrono` types when temporal operations are needed
- **Special Types**: Binary data (`bytes::Bytes`) and precise decimals (`rust_decimal::Decimal`) that serialize via serde but aren't native JSON types

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: All workspace tests pass after migration with 100% success rate (measured by `cargo test --workspace`)
- **SC-002**: Zero compilation errors and warnings after migration (measured by `cargo check --workspace` and `cargo clippy --workspace -- -D warnings`)
- **SC-003**: Node developers can implement nodes without seeing RawValue types (measured by reviewing public API surface - no RawValue in public traits)
- **SC-004**: Integration with serde-based libraries requires zero conversion code (measured by code review - no `.into()` or `From::from()` between workflow Value and library APIs)
- **SC-005**: Pass-through nodes avoid JSON parsing (measured by adding debug logging to verify RawValue cloning without serde_json::from_str calls)
- **SC-006**: Codebase complexity reduces (measured by lines of code - nebula-value crate deletion saves ~5000+ lines, removing `im` dependency)
- **SC-007**: Zero performance regression - migration maintains or improves performance compared to nebula-value baseline (validated by passing all existing tests; formal benchmarks deferred to future work if issues arise)

## Clarifications

### Session 2026-02-11

- Q: What is the maximum JSON payload size the system needs to handle efficiently for typical workflow scenarios? → A: Up to 100MB (with optimization to avoid sending large objects as single payloads, removing unnecessary data, and splitting where possible)
- Q: What is the acceptable performance deviation after migration compared to current nebula-value implementation? → A: 0% - performance must remain identical or improve (strict regression testing required)
- Q: What validation strategy is required to confirm "zero regression" after migration? → A: Tests only - if all existing tests pass, migration is successful (no additional benchmarks or integration tests required initially)
- Q: What rollback strategy should be provided if migration reveals critical issues? → A: No rollback needed - migration will be fully tested on feature branch before merge, issues resolved before integration to main

## Assumptions

- Database drivers (sqlx, diesel) and HTTP libraries (reqwest, hyper) already use serde_json, making integration seamless
- Temporal data in workflows is already stored as ISO 8601 strings in most cases (n8n compatibility)
- Most workflow nodes inspect data (require parsing), but key pass-through nodes (Filter, Switch, Merge) benefit from RawValue optimization
- Standard Vec/HashMap performance is acceptable for workflow use cases (cloning is infrequent enough that persistent data structures aren't needed)
- Existing nebula-value types like `Decimal` and `Bytes` can use serde ecosystem equivalents without behavioral changes
- Typical workflow payloads are under 10MB; system targets efficient handling up to 100MB with RawValue optimizations, though best practice is to split large datasets rather than passing as single objects

## Dependencies

- Successful migration depends on nebula-config being migrated first (simplest, minimal Value usage)
- nebula-expression migration depends on nebula-config and nebula-resilience being complete (most complex, extensive Value usage)
- No external dependencies or blocked resources
- Migration occurs entirely on feature branch `008-serde-value-migration`; integration to main only after all tests pass and validation complete

## Out of Scope

- Performance optimization beyond removing conversion overhead (no new caching, no query optimization)
- Changes to workflow semantics or execution model
- New features for temporal types or special data types (use existing serde ecosystem capabilities)
- Migration of other workspace crates beyond nebula-config, nebula-resilience, nebula-expression (other crates will be migrated in future work if needed)
- Custom serde Deserialize implementations (use ecosystem defaults)
