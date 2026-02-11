# Implementation Plan: Extend nebula-core with Identity and Multi-Tenancy Types

**Branch**: `006-extend-core-identity` | **Date**: 2026-02-05 | **Updated**: 2026-02-10 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/006-extend-core-identity/spec.md`

## Summary

Extend the existing `nebula-core` crate with new type-safe identifiers (`ProjectId`, `RoleId`, `OrganizationId`), enhanced scope hierarchy for multi-tenant isolation (Organization > Project > Workflow > Execution > Action), and RBAC foundational types (`ProjectType`, `RoleScope`). Additionally, migrate all existing ID types from `domain-key` 0.1.1 `key_type!` (SmartString-based) to `domain-key` 0.2.1 `define_uuid!` (UUID-based, `Copy`, 16 bytes) for consistency and performance.

**Technical Approach**: Upgrade `domain-key` to 0.2.1 with `uuid-v4` feature. Replace all `key_type!` / `define_domain!` ID definitions with `define_uuid!` macro. This gives all entity IDs: `Copy` semantics, 16-byte UUID storage, validated format, and uniform API. Extend `ScopeLevel` enum with new variants, add Copy-able enums for RBAC, and update prelude exports. String-based keys (`ParameterKey`, `CredentialKey`) remain on `key_type!`.

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)
**Primary Dependencies**: 
- `domain-key` 0.2.1 with `serde` + `uuid-v4` features (upgrade from 0.1.1)
- `serde` v1.0 (already present) - for serialization
- `uuid` v1.0 (already present) - used internally by domain-key Uuid<D>

**Storage**: N/A (this feature only adds types, no persistence logic)
**Testing**: `cargo test -p nebula-core`, unit tests for each new type, scope containment tests
**Target Platform**: Cross-platform (types are platform-agnostic)
**Project Type**: Extension to existing `nebula-core` crate (Core layer of 16-crate workspace)

**Performance Goals**: 
- All entity IDs are `Copy` (16 bytes, stack-allocated, zero heap allocation)
- Copy-able enums for hot path usage (no allocations)
- No string round-trip for UUID creation (`v4()` directly)

**Constraints**: 
- Must upgrade `domain-key` from 0.1.1 to 0.2.1
- Migrating existing ID types from `key_type!` to `define_uuid!` is a breaking API change (acceptable pre-1.0)
- Must follow existing code patterns and style in nebula-core
- Must fix all compilation errors in workspace after migration

**Scale/Scope**: 
- Upgrade domain-key dependency (Cargo.toml)
- Migrate 8 existing ID types from `key_type!` to `define_uuid!`
- Add 3 new ID types (ProjectId, RoleId, OrganizationId) via `define_uuid!`
- 2 new ScopeLevel variants (Organization, Project)
- 2 new enums (ProjectType, RoleScope)
- Fix all call sites in workspace that use old `Key<D>` API

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with `.specify/memory/constitution.md` principles:

- [x] **Type Safety First**: Feature uses `domain-key` 0.2.1 `Uuid<D>` for compile-time domain separation. All entity IDs are UUID-based with phantom type parameters preventing mixing. `Copy` semantics eliminate `.clone()` noise.
- [x] **Isolated Error Handling**: No errors introduced (types don't have error conditions). Future error handling will use existing `CoreError` from nebula-core.
- [x] **Test-Driven Development**: Test strategy defined - write tests for each ID type (creation, conversion, Display), scope containment hierarchy, and serialization before implementing.
- [x] **Async Discipline**: N/A - no async code in this feature (pure type definitions)
- [x] **Modular Architecture**: Extends existing `nebula-core` crate (Core layer). Existing dependency upgraded (domain-key 0.1.1 → 0.2.1); no new crate dependencies added. Future crates (nebula-user, nebula-project) will depend on these types.
- [x] **Observability**: N/A - no runtime behavior to observe (compile-time types only)
- [x] **Simplicity**: Feature adds necessary types for identity system without premature abstractions. Each type has clear, single purpose. No speculative features.
- [x] **Rust API Guidelines**: All types follow rustdoc conventions, standard naming (snake_case/PascalCase), include examples. Quality gates (fmt, clippy, doc) will run after implementation.

**Constitution Compliance**: ✅ PASS - No violations. Feature is pure type system extension following existing patterns.

## Project Structure

### Documentation (this feature)

```text
specs/006-extend-core-identity/
├── plan.md              # This file
├── research.md          # Phase 0: Pattern analysis, design decisions
├── data-model.md        # Phase 1: Type definitions, relationships
├── quickstart.md        # Phase 1: Usage examples for developers
└── tasks.md             # Phase 2: Implementation tasks (created by /speckit.tasks)
```

### Source Code (repository root)

```text
crates/
└── nebula-core/          # EXISTING crate - will be modified
    ├── src/
    │   ├── lib.rs        # Update prelude exports, re-export UuidParseError
    │   ├── id.rs         # Migrate all key_type! → define_uuid!, add 3 new types
    │   ├── keys.rs       # Unchanged (ParameterKey, CredentialKey stay on key_type!)
    │   ├── scope.rs      # Extend: ScopeLevel enum with Organization, Project
    │   └── types.rs      # Add: ProjectType, RoleScope enums
    ├── tests/
    │   └── (inline)      # Update existing tests for new Uuid<D> API
    └── Cargo.toml        # Upgrade domain-key to 0.2.1, add uuid-v4 feature
```

**Structure Decision**: 
- **Modifying existing crate**: `nebula-core` (Core layer)
- **Rationale**: These are foundational types that multiple future crates will depend on (nebula-user, nebula-project, nebula-rbac, nebula-auth). Placing them in `nebula-core` prevents circular dependencies and follows the existing pattern where UserId, TenantId already live in core.
- **Layer justification**: Core layer is the foundation for all other crates. Adding identity-related ID types here is consistent with existing ExecutionId, WorkflowId, UserId, TenantId.
- **No new crates**: Following Principle V (Modular Architecture), we extend existing core rather than creating a new crate, as these types have no independent existence - they're consumed by higher layers.

## Complexity Tracking

> **No constitution violations** - this section intentionally empty.

All principles followed without exceptions. Feature adds necessary type safety infrastructure using established patterns.

---

## Phase 0: Research & Resolution

**Goal**: Resolve all design decisions and establish patterns to follow.

### Research Tasks

1. **Pattern Analysis**: Document existing ID type patterns in `nebula-core/src/id.rs`
   - Extract: struct definition pattern, trait implementations, method signatures
   - Output: Template for new ID types (ProjectId, RoleId, OrganizationId)

2. **Scope Hierarchy Design**: Analyze `nebula-core/src/scope.rs` containment logic
   - Current hierarchy: Global > Workflow > Execution > Action
   - Design new hierarchy: Global > Organization > Project > Workflow > Execution > Action
   - Key decision: How to handle Workflow/Execution relationships with Project (runtime vs static)
   - Output: Containment matrix showing all variant relationships

3. **Enum Pattern Study**: Review Rust Copy enums in codebase
   - Best practices for simple, Copy-able enums
   - Serialization format (string vs integer)
   - Output: Pattern for ProjectType and RoleScope

4. **Naming Convention Verification**: Check against Rust API Guidelines
   - Verify: `ProjectId::v4()`, `parse()`, `get()` follow `domain-key` `Uuid<D>` API
   - Verify: enum variant names follow PascalCase
   - Output: Naming checklist

**Outputs**: 
- `research.md` with all patterns documented
- Design decisions for scope containment logic
- Justification for `Uuid<D>` over `Key<D>` documented in research.md

---

## Phase 1: Design & Contracts

**Goal**: Define exact type signatures, relationships, and usage examples.

### Data Model

**Outputs**: `data-model.md` with:

1. **ID Type Definitions** (using `domain-key` 0.2.1 `define_uuid!`):
```rust
use domain_key::define_uuid;

// All entity IDs — UUID-based, Copy, 16 bytes
define_uuid!(ProjectIdDomain => ProjectId);
define_uuid!(RoleIdDomain => RoleId);
define_uuid!(OrganizationIdDomain => OrganizationId);

// Existing IDs migrated from key_type! to define_uuid!
define_uuid!(UserIdDomain => UserId);
define_uuid!(TenantIdDomain => TenantId);
// ... etc for all 8 existing types

// API: v4(), nil(), parse(), get(), as_bytes(), domain(), is_nil()
// Traits: Copy, Clone, Debug, Display, FromStr, Eq, Ord, Hash, Serialize, Deserialize
```

2. **Scope Hierarchy Extension**:
```rust
pub enum ScopeLevel {
    Global,
    Organization(OrganizationId),  // NEW
    Project(ProjectId),              // NEW
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(ExecutionId, NodeId),
}

// New methods:
impl ScopeLevel {
    pub fn is_organization(&self) -> bool;
    pub fn is_project(&self) -> bool;
    pub fn organization_id(&self) -> Option<&OrganizationId>;
    pub fn project_id(&self) -> Option<&ProjectId>;
}
```

3. **RBAC Enums**:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectType {
    Personal,
    Team,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoleScope {
    Global,
    Project,
    Credential,
    Workflow,
}
```

4. **Containment Matrix**: Document which scopes contain which
   - Global contains all
   - Organization contains Project, Workflow, Execution, Action
   - Project contains Workflow, Execution, Action (runtime relationship)
   - Workflow contains Execution, Action (runtime)
   - Execution contains Action
   - Action contains nothing

### Quickstart Guide

**Outputs**: `quickstart.md` with usage examples:

```rust
// Example 1: Creating type-safe IDs (UUID-based, Copy)
use nebula_core::{ProjectId, RoleId, OrganizationId};

let project_id = ProjectId::v4();       // random UUID
let role_id = RoleId::v4();
let org_id = OrganizationId::v4();

// Copy — no .clone() needed
let copy = project_id;
assert_eq!(project_id, copy);

// Type safety prevents mixing:
fn process_project(id: ProjectId) { /* ... */ }
// process_project(role_id); // Compile error!

// Example 2: Scope-based isolation
use nebula_core::{ScopeLevel, ProjectId, ExecutionId};

let project_scope = ScopeLevel::Project(ProjectId::v4());
let execution_scope = ScopeLevel::Execution(ExecutionId::v4());

// Check containment for resource isolation
assert!(execution_scope.is_contained_in(&project_scope));

// Example 3: RBAC types
use nebula_core::{ProjectType, RoleScope};

let project_type = ProjectType::Team;
let role_scope = RoleScope::Project;

match role_scope {
    RoleScope::Global => { /* instance-wide permissions */ }
    RoleScope::Project => { /* project-specific permissions */ }
    _ => {}
}
```

### Agent Context Update

After Phase 1 design completion:

```bash
.specify/scripts/bash/update-agent-context.sh claude
```

This will update `CLAUDE.md` with:
- New types added to nebula-core: ProjectId, RoleId, OrganizationId
- Updated Scope system hierarchy
- RBAC enums (ProjectType, RoleScope)

---

## Phase 2: Task Breakdown

**Note**: This phase is executed by `/speckit.tasks`, not `/speckit.plan`.

Tasks will be generated based on:
- Upgrade `domain-key` to 0.2.1 with `uuid-v4` feature in Cargo.toml
- Migrate 8 existing ID types from `key_type!` to `define_uuid!` in `id.rs`
- Add 3 new ID types via `define_uuid!` in `id.rs`
- Fix all compilation errors across workspace (call sites using old API)
- Extend `ScopeLevel` enum in `scope.rs` (2 variants + 6 methods)
- Add 2 enums to `types.rs`
- Update `lib.rs` prelude exports
- Write tests for new and migrated types
- Update module documentation

**Estimated tasks**: 10-15 tasks (migration adds complexity vs pure additive).

---

## Quality Gates

### Phase 0 Completion
- [x] Constitution Check passed
- [ ] All NEEDS CLARIFICATION resolved in research.md
- [ ] Patterns documented with examples
- [ ] Design decisions justified

### Phase 1 Completion
- [ ] data-model.md complete with all type signatures
- [ ] quickstart.md has runnable examples
- [ ] Agent context updated
- [ ] Migration changes documented; all call sites identified

### Phase 2 Completion (verified by /speckit.tasks)
- [ ] All code follows existing patterns exactly
- [ ] `cargo fmt --all` passes
- [ ] `cargo clippy -p nebula-core -- -D warnings` passes
- [ ] `cargo test -p nebula-core` passes
- [ ] `cargo check --workspace` passes (backward compatibility)
- [ ] `cargo doc -p nebula-core` builds without warnings
- [ ] No phase markers or task references in public documentation

---

## Risk Assessment

| Risk | Impact | Mitigation | Status |
|------|--------|------------|--------|
| domain-key 0.2.1 API breaking changes | HIGH | Pre-1.0 project, mechanical migration, `cargo check --workspace` catches all | 📋 Planned |
| Other crates using old ID API | HIGH | Fix all call sites after migration, compile-time verification | 📋 Planned |
| Scope containment bugs | MEDIUM | Comprehensive test matrix for all combinations | 📋 Planned |
| domain-key 0.2.1 compatibility issues | MEDIUM | Published crate by project author, tested before upgrading | 📋 Planned |
| Missing exports in prelude | LOW | Verify with quickstart examples | 📋 Planned |
| Circular dependencies | LOW | All types in core layer, no new crate dependencies | ✅ Mitigated |

---

## Dependencies

**This feature blocks**:
- Milestone 1.1: nebula-user (needs RoleId)
- Milestone 1.2: nebula-project (needs ProjectId, ProjectType)
- Milestone 2.2: nebula-rbac (needs RoleId, RoleScope)
- Milestone 3.1: nebula-tenant (needs Project scope level)
- Milestone 4.3: nebula-organization (needs OrganizationId, Organization scope)

**This feature depends on**: None (extends existing nebula-core only)

---

## Success Criteria

1. ✅ All ID types compile with same trait set as existing types
2. ✅ Scope hierarchy tests cover all containment relationships
3. ✅ Existing nebula-core tests pass unchanged
4. ✅ Workspace compiles without errors (`cargo check --workspace`)
5. ✅ Zero clippy warnings
6. ✅ Documentation builds and includes examples
7. ✅ Quickstart examples compile and run
8. ✅ All type names preserved; API migrated to `Uuid<D>` (pre-1.0 breaking change accepted)

---

## Next Steps

1. Run `/speckit.plan` completion tasks (research, data-model, quickstart)
2. Review and commit Phase 0-1 artifacts
3. Run `/speckit.tasks` to generate implementation tasks
4. Execute tasks following TDD (tests first, then implementation)
5. Run quality gates before marking complete
