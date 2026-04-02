# Compatibility Policy

**Phase 2: Compatibility Contracts** — explicit stability promises for nebula-core.

## Contract Surface

The following types are considered **schema-stable**. Their serialized form (JSON for API/storage boundaries) must not change in patch/minor releases. Changes require a major version and MIGRATION.md.

| Type | Location | Serialization | Notes |
|------|----------|---------------|-------|
| `Status` | types.rs | Enum variant name (PascalCase) | `"Active"`, `"InProgress"`, etc. |
| `Priority` | types.rs | Enum variant name | `"Low"`, `"Normal"`, `"High"`, etc. |
| `ProjectType` | types.rs | snake_case (`rename_all`) | `"personal"`, `"team"` |
| `RoleScope` | types.rs | snake_case | `"global"`, `"project"`, etc. |
| `InterfaceVersion` | types.rs | `{"major":N,"minor":N}` | |
| `ScopeLevel` | scope.rs | Tagged enum with UUID strings | Variant name + ID(s) |
| ID types | id.rs | UUID string | `"550e8400-e29b-41d4-a716-446655440000"` |
| `CoreError::error_code()` | error.rs | Uppercase string | `"VALIDATION_ERROR"`, etc. |

## Enforcement

- **Schema contract tests** in `crates/core/tests/schema_contracts.rs` assert JSON and `error_code()` stability.
- CI runs these tests; accidental drift fails the build.
- Intentional changes: update expected values, bump major version, document in MIGRATION.md.

## Rules

1. **Patch/minor**: No breaking changes to serialized form or `error_code()`.
2. **Major**: Document in MIGRATION.md; provide compatibility path where possible.
3. **Deprecation**: Minimum 6 months; `#[deprecated]` with replacement path.

## Scope Semantics (Phase 3)

- `is_contained_in` — simplified level check; kept for backward compatibility.
- `is_contained_in_strict` — ID-verified containment via `ScopeResolver`; use when security/lifecycle correctness matters.
- Future major: may flip defaults or remove simplified behavior.
