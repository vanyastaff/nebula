# Interactions

## Ecosystem Map (Current + Planned)

`nebula-core` is the foundation crate. All other Nebula crates depend on it. Core has **no** Nebula dependencies.

## Existing Crates (Downstream Consumers)

- **action:** IDs, `ScopeLevel`, `OperationContext`, `CoreError`, `PluginKey`, traits
- **credential:** `CredentialId`, `ScopeLevel`, context types, `CoreError`
- **engine:** IDs, scope, context, error, constants
- **execution:** IDs, scope, traits, types, error
- **expression:** IDs, `CoreError`, types
- **memory:** IDs, scope, constants
- **plugin:** `PluginKey`, IDs, traits
- **ports:** IDs, types, error
- **resilience:** IDs, constants, error
- **runtime:** IDs, scope, context, error
- **sdk:** IDs, traits, types, prelude
- **storage:** IDs, scope, error
- **telemetry:** IDs, context
- **webhook:** IDs, types
- **workflow:** IDs, scope, traits, types

## Planned Crates

- **api / cli / ui:** Will use IDs, context, error for request/response and auth
- **worker / cluster:** Will use IDs, scope, error for distributed execution

## Downstream Consumers (Expectations)

All consumers expect:

- **IDs:** Typed UUID wrappers; `new()`, `parse()`, `nil()`; serde round-trip; `Copy`
- **Scope:** `ScopeLevel` hierarchy; `is_contained_in`; `ScopedId` constructors
- **Traits:** `Scoped`, `HasContext` for common entity behavior
- **Error:** `CoreError` with `is_retryable()`, `is_client_error()`, `error_code()`, `user_message()`
- **Constants:** Stable defaults for timeouts, limits, env vars

## Upstream Dependencies

Core depends on:

- **nebula-log** — cross-cutting logging (only Nebula crate allowed).
- **domain-key:** Typed UUID wrappers; hard contract on `define_uuid` macro; KeyParseError, key_type for ParameterKey, CredentialKey.
- **serde, serde_json:** Serialization; required.
- **thiserror:** Error derivation; required.
- **chrono:** Timestamps in OperationContext, OperationResult.
- **postcard:** Binary serialization support and error conversions.
- **uuid, async-trait, dashmap:** As in Cargo.toml.

No other nebula-* crates: core is the foundation; only nebula-log is allowed.

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---------------------|-----------|----------|------------|------------------|-------|
| core → (none) | — | — | — | — | Core has no Nebula deps |
| action → core | in | IDs, scope, context, error, keys | sync | CoreError propagation | action consumes core types |
| credential → core | in | CredentialId, ScopeLevel, context | sync | CoreError | credential consumes core |
| engine → core | in | IDs, scope, context, error | sync | CoreError | engine consumes core |
| expression → core | in | IDs, CoreError, types | sync | CoreError | expression consumes core |
| runtime → core | in | IDs, scope, context, error | sync | CoreError | runtime consumes core |
| storage → core | in | IDs, scope, error | sync | CoreError | storage consumes core |
| (all others) → core | in | Various subsets | sync | CoreError | same pattern |

## Runtime Sequence

Core is passive; no runtime sequence originates from core. Consumers:

1. Create IDs via `Id::new()` or parse from storage/API
2. Build `OperationContext` for execution
3. Use `ScopeLevel` for resource lifecycle and access checks
4. Propagate `CoreError` on failure; check `is_retryable()` for retry logic

## Cross-Crate Ownership

- **core owns:** ID types, scope semantics, base traits, common types, error taxonomy, validated keys, constants
- **Consumers own:** When/how to use core types; domain-specific error wrapping; orchestration, persistence, transport

## Failure Propagation

- Core does not call into other crates; no failure propagation from core outward
- Consumers convert std/serde/uuid/chrono errors into `CoreError` via `From` impls
- Consumers may map `CoreError` to HTTP status, gRPC codes, or domain errors

## Versioning and Compatibility

- **Compatibility promise:** Patch/minor preserve public API and serialized forms of IDs, scope, types, keys
- **Breaking-change protocol:** Declare in MIGRATION.md; major version bump; migration path for consumers
- **Deprecation window:** Minimum 6 months for public API changes

## Contract Tests Needed

- **ID round-trip:** Serde JSON/postcard for all ID types
- **Scope containment:** `is_contained_in` matrix for all scope pairs
- **PluginKey normalization:** Snapshot tests for normalized output
- **Error code stability:** Snapshot tests for `error_code()` and `user_message()` output
