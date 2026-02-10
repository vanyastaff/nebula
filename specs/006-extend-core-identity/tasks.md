# Implementation Tasks: Extend nebula-core with Identity Types

**Feature**: 006-extend-core-identity  
**Branch**: `006-extend-core-identity`  
**Updated**: 2026-02-10  
**Spec**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md)

## Overview

This document breaks down the implementation into independently testable, parallelizable tasks. The scope includes upgrading `domain-key` to 0.2.1, migrating all existing ID types from `key_type!` to `define_uuid!`, adding new identity types, and extending the scope hierarchy.

**Total Tasks**: 24 tasks  
**Parallel Opportunities**: 10 parallelizable tasks  
**Critical Path**: Dependency upgrade → ID migration → Scope extension → RBAC enums → Polish

---

## Implementation Strategy

### Migration-First Approach

The core change is upgrading `domain-key` from 0.1.1 to 0.2.1 and migrating all entity IDs from `Key<D>` (SmartString) to `Uuid<D>` (uuid::Uuid, Copy). This must happen first because all subsequent work depends on the new API.

**Phase order**:
1. **Upgrade dependency** — domain-key 0.2.1 in Cargo.toml
2. **Migrate existing IDs** — key_type! → define_uuid! for all 8 types
3. **Fix workspace** — update all call sites using old API
4. **Add new types** — ProjectId, RoleId, OrganizationId, scope variants, RBAC enums
5. **Polish** — tests, docs, quality gates

### TDD Workflow

Per Constitution Principle III (Test-Driven Development):
1. Write test → verify it fails (RED)
2. Implement minimum code to pass (GREEN)
3. Refactor if needed
4. Run quality gates before marking complete

**Quality Gates** (run after each phase):
```bash
cargo fmt --all
cargo clippy -p nebula-core -- -D warnings
cargo test -p nebula-core
cargo check --workspace
cargo doc -p nebula-core
```

---

## Task Dependency Graph

```
Phase 1 (Setup)
└─> Phase 2 (Dependency Upgrade + Migration)
     └─> Phase 3 (Fix Workspace Compilation)
          ├──> Phase 4 (US1: New ID Types) ──┬──> Phase 5 (US2: Scope Extension)
          │                                    │
          │                                    └──> Phase 6 (US3+US4: RBAC Enums)
          └──────────────────────────────────────> Phase 7 (Polish)
```

**Critical Path**: Setup → Upgrade → Fix Workspace → New IDs → Scope → Polish

---

## Phase 1: Setup

**Goal**: Verify baseline and prepare for migration.

### Tasks

- [x] T001 Verify nebula-core compiles with existing tests (`cargo test -p nebula-core`)
- [x] T002 Verify workspace compiles (`cargo check --workspace`)

**Completion Criteria**:
- All existing tests pass (baseline)
- No uncommitted changes blocking the migration

---

## Phase 2: Dependency Upgrade & ID Migration

**Goal**: Upgrade domain-key and migrate all entity IDs to `Uuid<D>`.

### Tasks

- [x] T003 Upgrade `domain-key` from 0.1.1 to 0.2.1 with `uuid-v4` feature in `crates/nebula-core/Cargo.toml`

**Cargo.toml change**:
```toml
# Before
domain-key = { version = "0.1.1", features = ["serde"] }

# After
domain-key = { version = "0.2.1", features = ["serde", "uuid-v4"] }
```

- [x] T004 Migrate all ID types in `crates/nebula-core/src/id.rs` from `key_type!` to `define_uuid!`

**id.rs change**:
```rust
// Before
use domain_key::{define_domain, key_type};
define_domain!(UserIdDomain, "user-id");
key_type!(UserId, UserIdDomain);
// ... 8 types total

// After
use domain_key::define_uuid;
define_uuid!(UserIdDomain => UserId);
define_uuid!(TenantIdDomain => TenantId);
define_uuid!(ExecutionIdDomain => ExecutionId);
define_uuid!(WorkflowIdDomain => WorkflowId);
define_uuid!(NodeIdDomain => NodeId);
define_uuid!(ActionIdDomain => ActionId);
define_uuid!(ResourceIdDomain => ResourceId);
define_uuid!(CredentialIdDomain => CredentialId);
```

- [x] T005 Update `crates/nebula-core/src/keys.rs` — keep `key_type!` for string keys, update imports for domain-key 0.2.1 compatibility

- [x] T006 Update `id.rs` tests for new `Uuid<D>` API (replace `new(uuid.to_string())` with `v4()`, `as_str()` with `get()`, etc.)

- [x] T007 Write tests for new ID types: creation (`v4()`, `nil()`, `parse()`), Copy semantics, serialization round-trip, domain separation (RED — tests fail before T008)

- [x] T008 Add new ID types to `id.rs`: `ProjectId`, `RoleId`, `OrganizationId` (GREEN — tests pass). Add module-level rustdoc (`//!`) documenting all ID types and their `domain-key` `Uuid<D>` basis.

```rust
// New types added alongside migrated ones
define_uuid!(ProjectIdDomain => ProjectId);
define_uuid!(RoleIdDomain => RoleId);
define_uuid!(OrganizationIdDomain => OrganizationId);
```

**Completion Criteria**:
- `cargo test -p nebula-core` passes
- All IDs are `Uuid<D>` based
- String keys unchanged

---

## Phase 3: Fix Workspace Compilation

**Goal**: Fix all crates that use nebula-core ID types with the old `Key<D>` API.

### Tasks

- [x] T009 Run `cargo check --workspace` and identify all compilation errors from ID API changes

- [x] T010 Fix `crates/nebula-core/src/scope.rs` — update any `.clone()` calls to use Copy, fix constructor patterns

- [x] T011 Fix `crates/nebula-core/src/traits.rs` — update trait method signatures/impls if they reference ID string methods

- [x] T012 Audit and fix `crates/nebula-credential/` — crate has its own `domain-key` dependency; update for 0.2.1 compatibility and fix any nebula-core ID type usage

- [x] T013 Fix remaining crates — iterate `cargo check --workspace` until clean

**Completion Criteria**:
- `cargo check --workspace` passes with zero errors
- `cargo test --workspace` passes
- No remaining `.as_str()` or `.new("string")` on migrated ID types

---

## Phase 4: User Story 1 — Type-Safe New ID Types (P1)

**Story Goal**: Developers can use `ProjectId`, `RoleId`, `OrganizationId` with compile-time type safety.

**Independent Test Criteria**: 
- Can import new types from `nebula_core`
- `v4()`, `nil()`, `parse()` work correctly
- Copy semantics — both copies usable
- JSON round-trip works
- Compiler rejects passing wrong ID type

### Tasks

- [x] T014 Add `ProjectId`, `RoleId`, `OrganizationId` to prelude exports in `crates/nebula-core/src/lib.rs`

- [x] T015 Re-export `domain_key::UuidParseError` from prelude for downstream parse error handling

**Verification**:
```bash
cargo test -p nebula-core -- id::tests
cargo clippy -p nebula-core -- -D warnings
cargo check --workspace
```

**Deliverable**: Type-safe ID types ready for nebula-user (Milestone 1.1)

---

## Phase 5: User Story 2 — Project-Scoped Resource Isolation (P1)

**Story Goal**: Developers can use `ScopeLevel::Organization` and `ScopeLevel::Project` for multi-tenant resource isolation.

### Test Tasks

- [x] T016 [P] Write tests for Organization and Project scope variants: creation, `is_organization()`, `is_project()`, `organization_id()`, `project_id()`

- [x] T017 [P] Write containment matrix tests for full 6-level hierarchy (36 combinations: both positive AND negative cases — e.g., Organization NOT contained in Project, peer projects don't contain each other)

### Implementation Tasks

- [x] T018 Extend `ScopeLevel` enum with `Organization(OrganizationId)` and `Project(ProjectId)` variants, update all match arms (`is_contained_in`, `parent`, `child`, `Display`, type checks, ID extractors), add `ChildScopeType::Organization` and `ChildScopeType::Project`. Add rustdoc on new methods.

**Implementation Notes**:
- IDs are Copy — use `*exec_id` instead of `exec_id.clone()` in match arms
- Containment: Global > Organization > Project > Workflow > Execution > Action
- Static containment (no runtime ID matching for Org/Project)

**Verification**:
```bash
cargo test -p nebula-core -- scope::tests
cargo test -p nebula-core
```

**Deliverable**: Scope-based isolation ready for nebula-project (Milestone 1.2)

---

## Phase 6: User Story 3+4 — RBAC Enums (P2/P3)

**Story Goal**: Developers can use `RoleScope` and `ProjectType` enums for RBAC and project management.

### Tasks

- [x] T019a [P] Write tests for `RoleScope` enum: all 4 variants, Copy, pattern matching, JSON round-trip ("global", "project", "credential", "workflow") (RED)

- [x] T019b [P] Write tests for `ProjectType` enum: both variants, Copy, JSON round-trip ("personal", "team") (RED)

- [x] T020a Implement `RoleScope` enum with rustdoc in `crates/nebula-core/src/types.rs` (GREEN)

```rust
/// Scope where a role applies
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleScope { Global, Project, Credential, Workflow }
```

- [x] T020b Implement `ProjectType` enum with rustdoc in `crates/nebula-core/src/types.rs` (GREEN)

```rust
/// Type of project: personal (single-user) or team (multi-user)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType { Personal, Team }
```

- [x] T021 Add `RoleScope` and `ProjectType` to prelude exports in `crates/nebula-core/src/lib.rs`

**Verification**:
```bash
cargo test -p nebula-core -- types::tests
```

**Deliverable**: RBAC types ready for nebula-rbac (Milestone 2.2)

---

## Phase 7: Polish & Quality Gates

**Goal**: Final verification, documentation, workspace-wide checks.

### Tasks

- [x] T022 Run full quality gate suite:
  ```bash
  cargo fmt --all
  cargo clippy --workspace -- -D warnings
  cargo test --workspace
  cargo doc -p nebula-core --no-deps
  cargo check --workspace --all-targets
  ```

**Completion Criteria**:
- Zero clippy warnings
- All tests pass (new + existing + workspace)
- Documentation builds without errors
- Workspace compiles cleanly

---

## Parallel Execution Map

### Within Phase 2 (after T003 completes)
```
T004 (migrate IDs) ──> T006 (update tests)
T007 (write new ID tests) ──> T008 (add new IDs)
T005 (update keys.rs) — independent
```

### Within Phase 5 (test + impl parallel)
```
T016 (scope variant tests) ─┐
T017 (containment tests)   ─┼──> T018 (implement scope changes)
```

### Phase 6 (test-first, then parallel impl)
```
T019a (RoleScope tests)    ─┐
T019b (ProjectType tests)  ─┼──> T020a (impl RoleScope) + T020b (impl ProjectType)
                             └──> T021 (prelude) — after T020a+T020b
```

---

## Task Summary

| Phase | Tasks | Description |
|-------|-------|-------------|
| Setup | 2 | Baseline verification |
| Upgrade & Migration | 6 | domain-key 0.2.1, migrate IDs, add new IDs |
| Fix Workspace | 5 | Fix compilation across all crates |
| US1: New IDs | 2 | Prelude exports, error re-export |
| US2: Scope Extension | 3 | Organization/Project variants + containment |
| US3+US4: RBAC Enums | 5 | Tests (T019a/T019b) → impl (T020a/T020b) → prelude (T021) |
| Polish | 1 | Quality gates, docs |
| **Total** | **24** | |

---

## Definition of Done

### Per Task
- [x] Tests written and passing
- [x] Code follows existing patterns
- [x] No clippy warnings introduced

### Per Phase
- [x] Phase-specific quality gates pass
- [x] `cargo check --workspace` clean

### Feature Complete
- [x] All 7 phases delivered
- [x] `cargo test --workspace` passes
- [x] `cargo clippy --workspace -- -D warnings` clean
- [x] `cargo doc -p nebula-core` builds
- [x] All new types exported in prelude
- [x] domain-key 0.2.1 with uuid-v4 feature in Cargo.toml

---

## Risk Mitigation

| Risk | Mitigation | Verification |
|------|------------|--------------|
| domain-key 0.2.1 breaking changes | Mechanical migration, compile-time checks | `cargo check --workspace` |
| Other crates broken by ID migration | Fix all call sites in Phase 3 | `cargo test --workspace` |
| Scope containment bugs | Test matrix (36 combinations) | Test coverage |
| Missing prelude exports | Verify with quickstart examples | Manual example compilation |

---

## References

- Specification: [spec.md](./spec.md) - User stories and acceptance criteria
- Plan: [plan.md](./plan.md) - Technical approach and decisions
- Data Model: [data-model.md](./data-model.md) - Complete type signatures
- Quickstart: [quickstart.md](./quickstart.md) - Usage examples to verify
- Research: [research.md](./research.md) - Pattern analysis and design decisions
- domain-key 0.2.1: https://crates.io/crates/domain-key
