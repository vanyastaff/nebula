# Proposals (Senior Review)

This document contains deliberate improvements for `nebula-core` with explicit migration paths.

## P-001: Split `CoreError` into Foundation + Domain Error Layers (Breaking)

Problem:
- `CoreError` currently mixes foundation concerns with domain-level variants.

Proposal:
- Keep `CoreError` only for foundation concerns (validation, serialization, config, infra-level failures).
- Move domain variants (`WorkflowExecution`, `NodeExecution`, `Cluster`, `Tenant`, etc.) to owning crates.

Why:
- Cleaner boundaries.
- Better dependency hygiene.
- Smaller, more stable core API surface.

Implementation Plan:
1. Introduce crate-local error enums in `engine`, `runtime`, `cluster`, `tenant` layers.
2. Add `From<CoreError>` where needed.
3. Mark moved `CoreError` variants deprecated for one cycle.
4. Remove deprecated variants in next major release.

Migration:
- Replace matches on moved variants with crate-specific error handling.

## P-002: Make Scope Containment Strict and Verifiable (Breaking)

Problem:
- Some `ScopeLevel::is_contained_in` checks are intentionally simplified and can allow ambiguous containment.

Proposal:
- Introduce a strict containment API that requires ownership links (workflow->project, execution->workflow).
- Keep current method as legacy wrapper during migration.

Why:
- Security correctness.
- Predictable lifecycle cleanup in high-load executions.

Implementation Plan:
1. Add `ScopeGraph`/`ScopeResolver` trait for ownership lookups.
2. Add `is_contained_in_strict(&self, other, resolver)` method.
3. Migrate callers in engine/runtime/resource management.
4. Remove simplified behavior in next major version.

Migration:
- Runtime/engine must provide resolver implementations.

## P-003: Reduce Core Constant Bloat (Breaking in API Surface)

Problem:
- `constants.rs` currently includes many domain-specific defaults.

Proposal:
- Keep only cross-cutting foundation constants in core.
- Move domain-owned constants to owning crates (`api`, `runtime`, `storage`, etc.).

Why:
- Better ownership.
- Lower accidental coupling between crates.

Implementation Plan:
1. Tag constants as `core-owned` vs `domain-owned`.
2. Create destination constants modules in owning crates.
3. Re-export deprecated aliases in core for one cycle.
4. Remove aliases in next major release.

Migration:
- Update imports from `nebula_core::constants::*` to crate-local constants modules.

## P-004: Stable External Schema Policy for Core Types (Non-Breaking Now, Prevents Future Breakage)

Proposal:
- Freeze serialized representations for core public enums/structs used across boundaries.
- Add snapshot tests for JSON representation.

Why:
- Protects API/storage/plugin compatibility.

Implementation Plan:
1. Create `serde_contract` tests for `Status`, `Priority`, `ProjectType`, `RoleScope`, `InterfaceVersion`.
2. Fail CI on accidental schema drift.

## P-005: ID and Key Ergonomics for Throughput-Sensitive Paths (Non-Breaking)

Proposal:
- Add zero-allocation helper methods around common ID/key transformations.
- Add benchmarks for parse/serialize hot paths.

Why:
- Workflow engines under load create/process many IDs and context objects.

Implementation Plan:
1. Add criterion benches in `crates/core/benches`.
2. Add profile-guided optimization targets for ID parse paths.
3. Track allocation counts in benchmark reports.
